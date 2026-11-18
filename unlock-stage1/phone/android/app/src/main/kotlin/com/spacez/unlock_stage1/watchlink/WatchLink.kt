package com.spacez.unlock_stage1.watchlink

import com.spacez.unlock_stage1.PermCompat
import com.spacez.unlock_stage1.PcCompanion
import com.spacez.unlock_stage1.UnlockCrypto
import com.spacez.unlock_stage1.UnlockState

import android.Manifest
import android.bluetooth.BluetoothManager
import android.content.Context
import android.content.pm.PackageManager
import android.util.Base64
import android.util.Log
import com.google.android.gms.tasks.Tasks
import com.google.android.gms.wearable.CapabilityClient
import com.google.android.gms.wearable.CapabilityInfo
import com.google.android.gms.wearable.MessageEvent
import com.google.android.gms.wearable.Wearable
import com.google.android.gms.wearable.WearableListenerService
import java.security.MessageDigest
import java.security.PublicKey
import java.security.SecureRandom
import java.util.concurrent.CompletableFuture
import java.io.DataInputStream
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit

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
            WatchProtocol.PATH_PHONE_INPUT -> {   // item 7: only from the pinned watch; unsigned (a move every 16 ms cannot each be signed)
                if (e.sourceNodeId != WatchTrust.pinnedNode(ctx) || e.data.size > 2_000) return
                PhoneControl.onInput(ctx, e.sourceNodeId, e.data)
            }
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
                    WatchCommand.WAKE_PC -> kotlin.concurrent.thread { Log.i(TAG, "watch wake: ${com.spacez.unlock_stage1.extras.PcWake.wake(ctx)}") }
                    WatchCommand.RESTART_PC -> kotlin.concurrent.thread { Log.i(TAG, "watch restart: ${com.spacez.unlock_stage1.extras.PcWake.power(ctx, "restart")}") }
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
 * Phone -> PC lock events over the PC's existing RFCOMM link service 7c1e0030 (the one autolock's
 * events.rs listens on). One short connection per event: connect, write one frame
 * `u16 len | 02 07 | CU1E...`, read the PC's ACK, close. Nothing is held open, nothing polls;
 * the Bluetooth radio is only used when the watch actually produced a lock signal.
 *
 * The frame is MAC'd with the v2 AUTO key (level 1) when the Keystore releases it (phone unlocked
 * or unlocked in the last [UnlockCrypto.AUTO_WINDOW_S] s), otherwise with the no-auth level-0 key
 * ([UnlockCrypto.lockEventMac]); autolock events.rs accepts both for these lock-only events, with
 * one shared counter (2026-10-20). A lock event can only lock, never unlock.
 */
object PcEvents {
    private const val TAG = "CUWatchLink"
    private val exec by lazy { Executors.newSingleThreadExecutor { r -> Thread(r, "pc-events").apply { isDaemon = true } } }
    private var lastSent = 0L

    /** Non-blocking: queues the send on a worker thread. */
    fun send(ctx: Context, e: PcEvent) {
        val app = ctx.applicationContext
        exec.execute { runCatching { sendNow(app, e) }.onFailure { Log.w(TAG, "pc event $e failed: ${it.message}") } }
    }

    private fun counterNext(ctx: Context): Long {
        val p = ctx.getSharedPreferences("cu_watch_trust", Context.MODE_PRIVATE)
        val c = p.getLong("event_counter", 0).coerceAtLeast(System.currentTimeMillis() / 1000) + 1 // survives a data clear
        p.edit().putLong("event_counter", c).commit()
        return c
    }

    private fun sendNow(ctx: Context, e: PcEvent) {
        val now = System.currentTimeMillis()
        if (e != PcEvent.LOCK_NOW && now - lastSent < 5_000) return // hints: at most one per 5 s
        val pcId = UnlockCrypto.pcId(ctx) ?: return Unit.also { note("no PC paired") }
        val addr = PcCompanion.pcAddress(ctx)
            ?: PcCompanion.associations(ctx).firstNotNullOfOrNull { it.deviceMacAddress?.toString() }
            ?: return Unit.also { note("PC Bluetooth address not known yet (it is learned on the first unlock attempt)") }
        if (!PermCompat.btConnect(ctx)) return Unit.also { note("no Bluetooth permission") }
        val ad = ctx.getSystemService(BluetoothManager::class.java)?.adapter?.takeIf { it.isEnabled } ?: return Unit.also { note("Bluetooth is off") }
        val attempts = if (e == PcEvent.LOCK_NOW) 2 else 1
        for (n in 1..attempts) {
            val (mac, keyName) = UnlockCrypto.lockEventMac() ?: return Unit.also { note("no event key in the Keystore (not paired?): $e dropped") }
            val counter = counterNext(ctx)
            val wire = PcEventFrame.wire(PcEventFrame.build(mac, e, System.currentTimeMillis(), counter, pcId))
            val t0 = System.currentTimeMillis()
            // Aura Link first, but only when the PC said it owns locking (LOCK_SINK); same signed frame.
            if (com.spacez.unlock_stage1.AuraPhone.trySendLock(wire.copyOfRange(2, wire.size))) {
                lastSent = System.currentTimeMillis()
                UnlockState.log("watch -> PC event $e #$counter ($keyName) delivered over Aura Link in ${lastSent - t0} ms")
                return
            }
            var s: android.bluetooth.BluetoothSocket? = null
            val ok = runCatching {
                s = ad.getRemoteDevice(addr).createRfcommSocketToServiceRecord(PC_LINK_UUID)
                s!!.connect()
                s!!.outputStream.apply { write(wire); flush() }
                val ack = ByteArray(4)
                val reader = Thread { runCatching { DataInputStream(s!!.inputStream).readFully(ack) } }.apply { isDaemon = true; start() }
                reader.join(3_000)
                // PC ACK = len 0x0002, ver 2, T_ACK (0x0A); anything else (or none) means the PC refused it.
                ack.contentEquals(byteArrayOf(0, 2, 2, 0x0A))
            }.onFailure { Log.w(TAG, "pc event $e attempt $n: ${it.javaClass.simpleName} ${it.message}") }.getOrDefault(false)
            runCatching { s?.close() }
            lastSent = System.currentTimeMillis()
            UnlockState.log("watch -> PC event $e #$counter ($keyName) ${if (ok) "delivered" else "NOT acknowledged"} in ${lastSent - t0} ms (attempt $n)")
            if (ok) return
            if (n < attempts) Thread.sleep(800)
        }
    }

    private fun note(s: String) { Log.i(TAG, s); UnlockState.log("watch -> PC event: $s") }

    /**
     * Item 17 remote Lock PC: the body of an Aura Link LOCK message (`02 07 | CU1E..` LockNow), signed
     * with the lock-event key (AUTO when released, else the no-auth level-0 key, lock-only). Same frame
     * and counter as the watch relay, so the PC's existing verifier and anti-replay decide.
     */
    fun lockNowBody(ctx: Context): ByteArray? {
        val pcId = UnlockCrypto.pcId(ctx) ?: return null
        val (mac, _) = UnlockCrypto.lockEventMac() ?: return null
        val wire = PcEventFrame.wire(PcEventFrame.build(mac, PcEvent.LOCK_NOW, System.currentTimeMillis(), counterNext(ctx), pcId))
        return wire.copyOfRange(2, wire.size)
    }

    /** autolock events.rs / unlock_pc::transports::PC_LINK_UUID */
    val PC_LINK_UUID: java.util.UUID = java.util.UUID.fromString("7c1e0030-5a1f-4e6b-9d2a-3c0b1a2f4e01")
}
