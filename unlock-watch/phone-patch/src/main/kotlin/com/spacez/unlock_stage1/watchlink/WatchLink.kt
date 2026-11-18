package com.spacez.unlock_stage1.watchlink

import android.content.Context
import android.util.Base64
import android.util.Log
import com.google.android.gms.tasks.Tasks
import com.google.android.gms.wearable.CapabilityClient
import com.google.android.gms.wearable.CapabilityInfo
import com.google.android.gms.wearable.MessageEvent
import com.google.android.gms.wearable.Wearable
import com.google.android.gms.wearable.WearableListenerService
import java.security.KeyStore
import java.security.MessageDigest
import java.security.PublicKey
import java.security.SecureRandom
import java.util.concurrent.CompletableFuture
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.TimeUnit
import javax.crypto.Mac
import javax.crypto.SecretKey

private const val TAG = "CUWatchLink"

/**
 * Pinned watch public key. Not secret (it is a public key); stored app-private. The pending
 * key is whatever the watch sent last; it is trusted only after the user confirms that the
 * fingerprint on the phone matches the one on the watch.
 */
object WatchTrust {
    private fun p(ctx: Context) = ctx.getSharedPreferences("cu_watch_trust", Context.MODE_PRIVATE)

    fun pinnedKey(ctx: Context): PublicKey? =
        p(ctx).getString("key", null)?.let { WatchProtocol.publicKeyFromDer(Base64.decode(it, Base64.NO_WRAP)) }
    fun pinnedNode(ctx: Context): String? = p(ctx).getString("node", null)

    fun pendingFingerprint(ctx: Context): String? =
        p(ctx).getString("pending_key", null)?.let { WatchProtocol.fingerprint(Base64.decode(it, Base64.NO_WRAP)) }

    internal fun setPending(ctx: Context, der: ByteArray, node: String) {
        if (WatchProtocol.publicKeyFromDer(der) == null) { Log.w(TAG, "pair: not an EC public key"); return }
        p(ctx).edit().putString("pending_key", Base64.encodeToString(der, Base64.NO_WRAP)).putString("pending_node", node).apply()
        Log.i(TAG, "pair request from watch $node, fingerprint ${WatchProtocol.fingerprint(der)} (waiting for user confirm)")
    }

    /** Call from the app UI after the user said the two fingerprints match. Blocking: not on the main thread. */
    fun confirmPending(ctx: Context): Boolean {
        val pr = p(ctx)
        val der = pr.getString("pending_key", null)?.let { Base64.decode(it, Base64.NO_WRAP) } ?: return false
        val node = pr.getString("pending_node", null) ?: return false
        pr.edit().putString("key", Base64.encodeToString(der, Base64.NO_WRAP)).putString("node", node)
            .remove("pending_key").remove("pending_node").putLong("push_seq", -1).apply()
        val ack = MessageDigest.getInstance("SHA-256").digest(der)
        runCatching { Tasks.await(Wearable.getMessageClient(ctx).sendMessage(node, WatchProtocol.PATH_PAIR_ACK, ack), 5, TimeUnit.SECONDS) }
            .onFailure { Log.w(TAG, "pair ack not delivered: ${it.message}") }
        Log.i(TAG, "watch pinned: ${WatchProtocol.fingerprint(der)}")
        return true
    }

    fun unpair(ctx: Context) = p(ctx).edit().clear().apply()

    internal fun pushGuard(ctx: Context) = SeqGuard(p(ctx).getLong("push_seq", -1))
    internal fun savePushSeq(ctx: Context, s: Long) = p(ctx).edit().putLong("push_seq", s).apply()
}

/**
 * The call UnlockService makes before answering a PC challenge without a tap:
 *
 *   val v = WatchGate.check(this, challengeNonce = c.nonce)   // worker thread, ~100-400 ms when near
 *   if (v == Verdict.ALLOW) sign at TAP level (no tap) else fall through to the prompt
 *   (wired in UnlockService.onChallenge since 2026-10-20, owner decision, no-lockout rule 8)
 *
 * Fail closed: no pinned watch, watch out of range, timeout, bad signature, off wrist or locked
 * all mean "no automatic unlock". The normal Approve tap and the PC PIN are never affected.
 */
object WatchGate {
    private val waiting = ConcurrentHashMap<String, CompletableFuture<ByteArray>>()
    private val rng = SecureRandom()

    /** Last PUSH (display only, never used to unlock). */
    @Volatile var lastPush: WatchStatus? = null

    /**
     * [challengeNonce]: pass the PC challenge's 16-byte nonce so the watch's signed answer is bound to
     * that exact challenge (fresh challenge, no-lockout rule 8); null = a random nonce (status checks).
     */
    fun check(ctx: Context, timeoutMs: Long = 1_500, challengeNonce: ByteArray? = null): Verdict {
        val key = WatchTrust.pinnedKey(ctx) ?: return Verdict.NO_REPLY.also { Log.i(TAG, "no watch paired") }
        val node = reachableWatch(ctx) ?: return Verdict.NO_REPLY.also { Log.i(TAG, "watch not reachable") }
        val nonce = challengeNonce?.takeIf { it.size == WatchProtocol.NONCE_LEN }?.copyOf()
            ?: ByteArray(WatchProtocol.NONCE_LEN).also { rng.nextBytes(it) }
        val id = nonce.joinToString("") { "%02x".format(it) }
        val f = CompletableFuture<ByteArray>()
        waiting[id] = f
        val t0 = System.currentTimeMillis()
        return try {
            Tasks.await(Wearable.getMessageClient(ctx).sendMessage(node, WatchProtocol.PATH_QUERY, nonce), timeoutMs, TimeUnit.MILLISECONDS)
            val left = (timeoutMs - (System.currentTimeMillis() - t0)).coerceAtLeast(1)
            val frame = runCatching { f.get(left, TimeUnit.MILLISECONDS) }.getOrNull()
            WatchPolicy.evaluate(frame, key, nonce, System.currentTimeMillis())
        } catch (e: Exception) {
            Log.w(TAG, "watch query failed: ${e.javaClass.simpleName}")
            Verdict.NO_REPLY
        } finally {
            waiting.remove(id)
        }.also { Log.i(TAG, "watch verdict $it in ${System.currentTimeMillis() - t0} ms") }
    }

    private fun reachableWatch(ctx: Context): String? {
        val pinned = WatchTrust.pinnedNode(ctx)
        val nodes = runCatching {
            Tasks.await(Wearable.getCapabilityClient(ctx).getCapability(WatchProtocol.CAPABILITY_WATCH, CapabilityClient.FILTER_REACHABLE), 1, TimeUnit.SECONDS).nodes
        }.getOrDefault(emptySet())
        return nodes.firstOrNull { it.id == pinned }?.id
    }

    /** A REPLY for an outstanding nonce. The signature is checked by WatchPolicy in check(). */
    internal fun onReply(frame: ByteArray) {
        if (frame.size < WatchProtocol.HEADER_LEN) return
        val id = frame.copyOfRange(7, 7 + WatchProtocol.NONCE_LEN).joinToString("") { "%02x".format(it) }
        waiting[id]?.complete(frame)
    }
}

/** Woken by Play services for /cu/watch/... messages from the watch app (same package + cert). */
class WatchLinkService : WearableListenerService() {
    override fun onMessageReceived(e: MessageEvent) {
        val ctx = applicationContext
        when (e.path) {
            WatchProtocol.PATH_PAIR -> WatchTrust.setPending(ctx, e.data, e.sourceNodeId)
            WatchProtocol.PATH_STATUS -> {
                if (e.sourceNodeId != WatchTrust.pinnedNode(ctx)) return
                val key = WatchTrust.pinnedKey(ctx) ?: return
                // Kind byte sits right after magic + version.
                if (e.data.size > 5 && e.data[5] == Kind.PUSH.code) {
                    val s = WatchCodec.decodeVerified(e.data, key)?.status ?: return
                    synchronized(this) {
                        val g = WatchTrust.pushGuard(ctx)
                        if (!g.accept(s)) return
                        WatchTrust.savePushSeq(ctx, g.lastSeq)
                    }
                    WatchGate.lastPush = s
                    Log.i(TAG, "watch push $s")
                    if (s.wristKnown && !s.onWrist) PcEvents.send(ctx, PcEvent.HINT_OFF_WRIST)
                } else {
                    WatchGate.onReply(e.data)
                }
            }
            WatchProtocol.PATH_COMMAND -> {
                if (e.sourceNodeId != WatchTrust.pinnedNode(ctx)) return
                val key = WatchTrust.pinnedKey(ctx) ?: return
                val d = WatchCodec.decodeVerified(e.data, key) ?: return Unit.also { Log.w(TAG, "command: bad signature") }
                if (d.status.kind != Kind.COMMAND) return
                if (kotlin.math.abs(System.currentTimeMillis() - d.status.watchMs) > WatchProtocol.MAX_SKEW_MS) return
                synchronized(this) {
                    val g = WatchTrust.pushGuard(ctx) // one seq space on the watch for PUSH and COMMAND
                    if (!g.accept(d.status)) return Unit.also { Log.w(TAG, "command replayed, ignored") }
                    WatchTrust.savePushSeq(ctx, g.lastSeq)
                }
                when (WatchCommand.of(String(d.payload))) {
                    WatchCommand.LOCK_PC -> PcEvents.send(ctx, PcEvent.LOCK_NOW)
                    else -> Log.i(TAG, "command ${String(d.payload).take(32)} not supported yet") // TODO (parked): ring, media, slides
                }
            }
            // TODO (parked): APPROVE/DENY for challenges relayed to the watch.
        }
    }

    /** Event-driven: Play services calls this when the watch's reachability changes. */
    override fun onCapabilityChanged(info: CapabilityInfo) {
        if (info.name != WatchProtocol.CAPABILITY_WATCH) return
        val pinned = WatchTrust.pinnedNode(applicationContext) ?: return
        if (info.nodes.none { it.id == pinned }) {
            Log.i(TAG, "watch out of range")
            PcEvents.send(applicationContext, PcEvent.HINT_OUT_OF_RANGE)
        }
    }
}

/**
 * Phone -> PC lock events. The phone app's UnlockService sets [sink] to its BLE GATT notify on
 * the EVENT characteristic (see README "Phone side"). If no PC is subscribed the event is
 * dropped: a missed lock hint just means the normal autolock timer decides, as today.
 */
object PcEvents {
    /** Returns true when the frame went out to a subscribed PC. */
    @Volatile var sink: ((ByteArray) -> Boolean)? = null
    private const val KEY_ALIAS = "continuity-unlock-v1" // UnlockCrypto.ALIAS
    private var lastSent = 0L

    @Synchronized fun send(ctx: Context, e: PcEvent) {
        val now = System.currentTimeMillis()
        if (e != PcEvent.LOCK_NOW && now - lastSent < 5_000) return // hints: at most one per 5 s
        val pcId = ctx.getSharedPreferences("unlock", 0).getString("pc_id", null)
            ?.chunked(2)?.map { it.toInt(16).toByte() }?.toByteArray() ?: return Unit.also { Log.i(TAG, "no PC paired") }
        val key = runCatching { KeyStore.getInstance("AndroidKeyStore").apply { load(null) }.getKey(KEY_ALIAS, null) as SecretKey? }
            .getOrNull() ?: return
        val p = ctx.getSharedPreferences("cu_watch_trust", Context.MODE_PRIVATE)
        val counter = p.getLong("event_counter", 0) + 1
        p.edit().putLong("event_counter", counter).commit()
        val frame = PcEventFrame.build(Mac.getInstance("HmacSHA256").apply { init(key) }, e, now, counter, pcId)
        val ok = sink?.invoke(frame) ?: false
        lastSent = now
        Log.i(TAG, "pc event $e #$counter ${if (ok) "sent" else "no PC subscribed"}")
    }
}
