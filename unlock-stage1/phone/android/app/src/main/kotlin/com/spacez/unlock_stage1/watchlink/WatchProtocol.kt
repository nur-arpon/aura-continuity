package com.spacez.unlock_stage1.watchlink

import java.nio.ByteBuffer
import java.security.KeyFactory
import java.security.MessageDigest
import java.security.PublicKey
import java.security.Signature
import java.security.spec.X509EncodedKeySpec

/**
 * Watch <-> phone frames for "unlock only when the watch is on the wrist and unlocked".
 *
 * Shared, unchanged, by the watch app and the phone app (copy this file into the phone tree).
 * Pure JVM: no Android imports, so the unit tests run on the PC.
 *
 * Trust: the watch makes an EC P-256 key pair in its Android Keystore (cannot be exported).
 * At pairing only the PUBLIC key goes to the phone, which pins it after the user checks the
 * same fingerprint on both screens. Nothing secret ever crosses the Data Layer.
 *
 * Frame (big endian):
 *   "CUW1"(4) | ver(1) | kind(1) | flags(1) | nonce(16) | watch_ms(8) | seq(8) | sig_len(2) | sig
 * The signature (SHA256withECDSA, DER) covers every byte before sig_len.
 */
object WatchProtocol {
    const val VERSION: Byte = 1
    val MAGIC = byteArrayOf('C'.code.toByte(), 'U'.code.toByte(), 'W'.code.toByte(), '1'.code.toByte())
    const val NONCE_LEN = 16
    const val HEADER_LEN = 4 + 1 + 1 + 1 + NONCE_LEN + 8 + 8
    const val MAX_SIG_LEN = 80 // DER P-256 ECDSA is at most 72 bytes
    const val SIG_ALG = "SHA256withECDSA"

    // Data Layer message paths. The watch's WearableListenerService filters on "/cu/watch".
    const val PATH_QUERY = "/cu/watch/query"          // phone -> watch: 16-byte nonce, "status now?"
    const val PATH_CHALLENGE = "/cu/watch/challenge"  // phone -> watch: nonce(16) | pc label (utf-8): relayed PC unlock request
    const val PATH_STATUS = "/cu/watch/status"        // watch -> phone: signed frame (REPLY / PUSH / APPROVE / DENY)
    const val PATH_PAIR = "/cu/watch/pair"            // watch -> phone: X.509 public key (DER)
    const val PATH_PAIR_ACK = "/cu/watch/pair_ack"    // phone -> watch: SHA-256 of the pinned key (32 bytes)
    const val PATH_COMMAND = "/cu/watch/command"      // watch -> phone: signed COMMAND frame + utf-8 command
    const val PATH_PC_STATE = "/cu/watch/pc_state"    // phone -> watch: utf-8 "connected=1;locked=0;name=..."
    const val PATH_PHONE_INPUT = "/cu/watch/phone_input"            // watch -> phone: INPUT-shaped json (move/click/scroll/key/text) for the PHONE's own screen
    const val PATH_PHONE_INPUT_STATE = "/cu/watch/phone_input_state" // phone -> watch: "ok" | "a11y_off" (watch control service not enabled)

    // Capabilities (res/values/wear.xml on each side) so each finds the other without polling.
    const val CAPABILITY_WATCH = "continuity_unlock_watch"
    const val CAPABILITY_PHONE = "continuity_unlock_phone"

    const val MAX_SKEW_MS = 120_000L

    fun fingerprint(publicKeyDer: ByteArray): String {
        val h = MessageDigest.getInstance("SHA-256").digest(publicKeyDer)
        return h.copyOf(6).joinToString("") { "%02X".format(it) }.chunked(4).joinToString("-")
    }

    fun publicKeyFromDer(der: ByteArray): PublicKey? =
        runCatching { KeyFactory.getInstance("EC").generatePublic(X509EncodedKeySpec(der)) }.getOrNull()
}

enum class Kind(val code: Byte) {
    REPLY(1),    // answer to a PATH_QUERY nonce
    PUSH(2),     // unsolicited state change (nonce is zero); display / cache only, never unlocks
    APPROVE(3),  // user (or auto rule) on the watch approved the relayed challenge with this nonce
    DENY(4),     // user tapped Deny on the watch
    COMMAND(5);  // watch remote command (lock PC, media...); nonce is random, seq is anti-replay

    companion object { fun of(b: Byte) = entries.firstOrNull { it.code == b } }
}

data class WatchStatus(
    val kind: Kind,
    val nonce: ByteArray,
    /** False when the watch has no off-body sensor or it did not answer in time. */
    val wristKnown: Boolean,
    val onWrist: Boolean,
    /** KeyguardManager.isDeviceLocked: the watch needs its PIN / pattern. */
    val deviceLocked: Boolean,
    /** KeyguardManager.isKeyguardLocked: the lock screen is showing (may be true while worn and unlocked-trusted). */
    val keyguardLocked: Boolean,
    val watchMs: Long,
    val seq: Long,
) {
    init { require(nonce.size == WatchProtocol.NONCE_LEN) }

    val flags: Byte get() = ((if (onWrist) 1 else 0) or (if (wristKnown) 2 else 0) or
        (if (deviceLocked) 4 else 0) or (if (keyguardLocked) 8 else 0)).toByte()

    /** The only state that may count toward an automatic unlock. */
    val readyForAutoUnlock: Boolean get() = wristKnown && onWrist && !deviceLocked

    fun signedBytes(): ByteArray = ByteBuffer.allocate(WatchProtocol.HEADER_LEN)
        .put(WatchProtocol.MAGIC).put(WatchProtocol.VERSION).put(kind.code).put(flags)
        .put(nonce).putLong(watchMs).putLong(seq).array()

    override fun equals(other: Any?) = other is WatchStatus && kind == other.kind && nonce.contentEquals(other.nonce) &&
        flags == other.flags && watchMs == other.watchMs && seq == other.seq
    override fun hashCode() = (kind.hashCode() * 31 + nonce.contentHashCode()) * 31 + seq.hashCode()
    override fun toString() = "WatchStatus($kind wrist=${if (wristKnown) onWrist else "?"} deviceLocked=$deviceLocked " +
        "keyguard=$keyguardLocked seq=$seq ms=$watchMs)"
}

/** Signs with the watch key. Production: Android Keystore; tests: a software key. */
fun interface StatusSigner { fun sign(data: ByteArray): ByteArray }

object WatchCodec {
    /** Frame + optional trailing payload (used by COMMAND frames; the payload is covered by the signature). */
    fun encode(s: WatchStatus, signer: StatusSigner, payload: ByteArray = ByteArray(0)): ByteArray {
        val body = s.signedBytes()
        val sig = signer.sign(body + payload)
        require(sig.size in 1..WatchProtocol.MAX_SIG_LEN)
        return ByteBuffer.allocate(body.size + 2 + sig.size + payload.size)
            .put(body).putShort(sig.size.toShort()).put(sig).put(payload).array()
    }

    class Decoded(val status: WatchStatus, val payload: ByteArray)

    /** Null on any malformed frame or bad signature. */
    fun decodeVerified(frame: ByteArray, key: PublicKey): Decoded? {
        if (frame.size < WatchProtocol.HEADER_LEN + 2) return null
        val b = ByteBuffer.wrap(frame)
        val magic = ByteArray(4).also { b.get(it) }
        if (!magic.contentEquals(WatchProtocol.MAGIC) || b.get() != WatchProtocol.VERSION) return null
        val kind = Kind.of(b.get()) ?: return null
        val flags = b.get().toInt()
        val nonce = ByteArray(WatchProtocol.NONCE_LEN).also { b.get(it) }
        val ms = b.long
        val seq = b.long
        val sigLen = b.short.toInt()
        if (sigLen < 1 || sigLen > WatchProtocol.MAX_SIG_LEN || b.remaining() < sigLen) return null
        val sig = ByteArray(sigLen).also { b.get(it) }
        val payload = ByteArray(b.remaining()).also { b.get(it) }
        val ok = runCatching {
            Signature.getInstance(WatchProtocol.SIG_ALG).run {
                initVerify(key)
                update(frame, 0, WatchProtocol.HEADER_LEN)
                update(payload)
                verify(sig)
            }
        }.getOrDefault(false)
        if (!ok) return null
        val s = WatchStatus(kind, nonce, flags and 2 != 0, flags and 1 != 0, flags and 4 != 0, flags and 8 != 0, ms, seq)
        return Decoded(s, payload)
    }
}

enum class Verdict { ALLOW, NO_REPLY, BAD_SIGNATURE, WRONG_KIND, WRONG_NONCE, CLOCK_SKEW, WRIST_UNKNOWN, OFF_WRIST, WATCH_LOCKED, DENIED }

/**
 * Phone-side decision. Fail closed: anything other than a fresh, correctly signed REPLY (or
 * APPROVE) for the nonce we just sent, saying "on wrist and unlocked", is not ALLOW. Not ALLOW
 * only means "no automatic unlock": the normal Approve / PIN paths are untouched.
 */
object WatchPolicy {
    fun evaluate(frame: ByteArray?, key: PublicKey, expectedNonce: ByteArray, nowMs: Long,
                 accept: Set<Kind> = setOf(Kind.REPLY)): Verdict {
        if (frame == null) return Verdict.NO_REPLY
        val s = WatchCodec.decodeVerified(frame, key)?.status ?: return Verdict.BAD_SIGNATURE
        return evaluate(s, expectedNonce, nowMs, accept)
    }

    fun evaluate(s: WatchStatus, expectedNonce: ByteArray, nowMs: Long, accept: Set<Kind> = setOf(Kind.REPLY)): Verdict {
        if (s.kind == Kind.DENY && Kind.APPROVE in accept && MessageDigest.isEqual(s.nonce, expectedNonce)) return Verdict.DENIED
        if (s.kind !in accept) return Verdict.WRONG_KIND
        if (!MessageDigest.isEqual(s.nonce, expectedNonce)) return Verdict.WRONG_NONCE
        if (kotlin.math.abs(nowMs - s.watchMs) > WatchProtocol.MAX_SKEW_MS) return Verdict.CLOCK_SKEW
        if (s.kind == Kind.APPROVE) return if (s.deviceLocked) Verdict.WATCH_LOCKED else Verdict.ALLOW
        if (!s.wristKnown) return Verdict.WRIST_UNKNOWN
        if (!s.onWrist) return Verdict.OFF_WRIST
        if (s.deviceLocked) return Verdict.WATCH_LOCKED
        return Verdict.ALLOW
    }
}

/** Accepts each PUSH / COMMAND only once and in order (seq must grow). Not thread-safe; guard externally. */
class SeqGuard(var lastSeq: Long = -1) {
    fun accept(s: WatchStatus): Boolean = if (s.seq > lastSeq) { lastSeq = s.seq; true } else false
}

/** Watch remote commands, relayed by the phone to the PC. Unknown strings are dropped by the phone. */
enum class WatchCommand(val wire: String) {
    LOCK_PC("lock_pc"), RING_PC("ring_pc"), WAKE_PC("wake_pc"), RESTART_PC("restart_pc"),
    MEDIA_PLAY_PAUSE("media_play_pause"), MEDIA_NEXT("media_next"), MEDIA_PREV("media_prev"),
    VOLUME_UP("volume_up"), VOLUME_DOWN("volume_down"),
    SLIDE_NEXT("slide_next"), SLIDE_PREV("slide_prev");

    companion object { fun of(s: String) = entries.firstOrNull { it.wire == s } }
}

/** PC state the phone forwards to the watch: "connected=1;locked=0;name=LAPTOP". Display only. */
data class PcState(val connected: Boolean, val locked: Boolean?, val name: String) {
    fun encode() = "connected=${if (connected) 1 else 0};locked=${when (locked) { true -> 1; false -> 0; null -> "?" }};name=${name.replace(";", "")}"
    companion object {
        fun decode(s: String): PcState {
            val m = s.split(';').mapNotNull { p -> p.split('=', limit = 2).takeIf { it.size == 2 }?.let { it[0] to it[1] } }.toMap()
            return PcState(m["connected"] == "1", when (m["locked"]) { "1" -> true; "0" -> false; else -> null }, m["name"] ?: "")
        }
    }
}

/**
 * Phone -> PC lock event, sent as ONE RFCOMM frame to the PC's link service 7c1e0030-5a1f-4e6b-9d2a-3c0b1a2f4e01
 * (autolock src\events.rs; same service the watch uses). MAC'd with the v2 AUTO (level 1) key of the
 * phone<->PC pairing: HMAC(secret,"continuity-unlock/v2/key/1"), the key unlock_pc::protocol::derive(secret).levels[1].
 * On the phone that is the Keystore entry UnlockCrypto.autoMac(). See test-vectors/pc-event-v2.txt.
 *
 *   "CU1E"(4) | ver(1) | event(1) | phone_ms(8) | counter(8) | pc_id(8) | hmac_sha256(32)  = 62 bytes
 *
 * A lock event can only ever LOCK the PC (always safe: the PIN still works), never unlock it.
 */
enum class PcEvent(val code: Byte) {
    LOCK_NOW(1),          // user tapped "Lock PC" on the watch
    HINT_OFF_WRIST(2),    // watch left the wrist
    HINT_OUT_OF_RANGE(3); // watch dropped off the phone's Data Layer (walked away)
    companion object { fun of(b: Byte) = entries.firstOrNull { it.code == b } }
}

object PcEventFrame {
    val MAGIC = "CU1E".toByteArray()
    const val LEN = 4 + 1 + 1 + 8 + 8 + 8 + 32

    /** RFCOMM wire form: u16 BE length | link ver 2 | T_EVENT 0x07 | frame (same as the watch's PcLink.send). */
    fun wire(frame: ByteArray): ByteArray {
        val n = frame.size + 2
        return byteArrayOf((n shr 8).toByte(), n.toByte(), 2, 0x07) + frame
    }

    /** [mac] is HmacSHA256 already initialised with the phone<->PC key. */
    fun build(mac: javax.crypto.Mac, e: PcEvent, phoneMs: Long, counter: Long, pcId: ByteArray): ByteArray {
        require(pcId.size == 8)
        val body = ByteBuffer.allocate(LEN - 32).put(MAGIC).put(WatchProtocol.VERSION).put(e.code)
            .putLong(phoneMs).putLong(counter).put(pcId).array()
        return body + mac.doFinal(body)
    }
}

/**
 * Pointer maths for "watch controls the phone": a cursor kept inside the screen, and scroll wheel units
 * (the PC INPUT unit, 120 = one notch, negative = scroll down) turned into a finger swipe.
 */
class PhonePointer(var w: Int, var h: Int, var x: Float = w / 2f, var y: Float = h / 2f) {
    /** Cursor speed: watch deltas are already scaled by the touchpad gain; phone pixels are denser than a PC's. */
    fun move(dx: Int, dy: Int, gain: Float = 1.6f) {
        x = (x + dx * gain).coerceIn(0f, (w - 1).toFloat()); y = (y + dy * gain).coerceIn(0f, (h - 1).toFloat())
    }
    /** (x0,y0,x1,y1) of the swipe for a wheel value, centred on the cursor, or null for 0. Fingers move WITH the sign (wheel -120 = content up = finger up). */
    fun scrollSwipe(wheel: Int, pxPerNotch: Float = 300f): List<Float>? {
        if (wheel == 0) return null
        val d = (wheel / 120f * pxPerNotch).coerceIn(-h * 0.8f, h * 0.8f)
        val y0 = (y - d / 2f).coerceIn(1f, (h - 2).toFloat()); val y1 = (y + d / 2f).coerceIn(1f, (h - 2).toFloat())
        return if (y0 == y1) null else listOf(x, y0, x, y1)
    }
}
