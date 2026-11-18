package com.spacez.unlock_stage1

import android.content.Context
import android.security.keystore.KeyProperties
import android.security.keystore.KeyProtection
import java.nio.ByteBuffer
import java.security.KeyStore
import java.security.MessageDigest
import javax.crypto.Mac
import javax.crypto.SecretKey
import javax.crypto.spec.SecretKeySpec

/**
 * Mirror of pc/src/protocol.rs (v2) and pc/src/pairing.rs.
 *
 * One pairing secret -> one HMAC key per approval level, each imported into the Android
 * Keystore under a DIFFERENT protection, so the PC can tell from the MAC alone which
 * protection released the answer:
 *   0 deny         no auth
 *   1 auto         two copies: usable for AUTO_WINDOW_S after the user unlocked the phone
 *                  with PIN / fingerprint ("recent"), or while the phone is unlocked
 *   2 tap          no auth (notification button; also the stage-1 TEST MODE)
 *   3 fingerprint  per-use BiometricPrompt, strong biometric only
 * The keys never leave the Keystore; the raw secret is wiped after import.
 */
object UnlockCrypto {
    const val VERSION: Byte = 2
    const val T_CHALLENGE: Byte = 1
    const val T_RESPONSE: Byte = 2
    const val T_WAITING: Byte = 3
    const val T_READY: Byte = 4
    const val T_NOT_NOW: Byte = 5
    /** min_level flag: hands-free only, never prompt (see pc/src/protocol.rs HANDS_FREE) */
    const val HANDS_FREE = 0x80
    const val CHALLENGE_LEN = 35
    const val RESPONSE_LEN = 59
    const val DENY = 0
    const val AUTO = 1
    const val TAP = 2
    const val FINGERPRINT = 3
    const val AUTO_WINDOW_S = 300
    private const val P = "cu2-"
    private const val ALPH = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567"
    private const val PREFS = "unlock"

    val WAITING_FRAME = byteArrayOf(VERSION, T_WAITING)
    val READY_FRAME = byteArrayOf(VERSION, T_READY)
    val NOT_NOW_FRAME = byteArrayOf(VERSION, T_NOT_NOW)

    class Challenge(val nonce: ByteArray, val issuedMs: Long, val pcId: ByteArray, val minLevel: Int)

    fun hmac(key: ByteArray, vararg parts: ByteArray): ByteArray {
        val m = Mac.getInstance("HmacSHA256")
        m.init(SecretKeySpec(key, "HmacSHA256"))
        parts.forEach { m.update(it) }
        return m.doFinal()
    }

    fun levelKey(secret: ByteArray, level: Int) = hmac(secret, "continuity-unlock/v2/key/$level".toByteArray())
    fun pcIdOf(secret: ByteArray) = hmac(secret, "continuity-unlock/v2/id".toByteArray()).copyOf(8)

    /** Decode the typed pairing code; null when the length or check letters are wrong. */
    fun decodeCode(code: String): ByteArray? {
        val vals = ArrayList<Int>()
        for (raw in code.uppercase()) {
            if (raw == '-' || raw.isWhitespace()) continue
            val c = when (raw) { '0' -> 'O'; '1' -> 'I'; '8' -> 'B'; else -> raw }
            val v = ALPH.indexOf(c)
            if (v < 0) return null
            vals.add(v)
        }
        if (vals.size != 34) return null
        val out = ByteArray(20)
        var bits = 0L
        var n = 0
        var i = 0
        for (v in vals.subList(0, 32)) {
            bits = (bits shl 5) or v.toLong()
            n += 5
            if (n >= 8) {
                n -= 8
                out[i++] = ((bits shr n) and 0xff).toByte()
            }
        }
        val h = MessageDigest.getInstance("SHA-256").digest(out)
        val want = byteArrayOf(((h[0].toInt() and 0xff) shr 3).toByte(), ((h[1].toInt() and 0xff) shr 3).toByte())
        return if (MessageDigest.isEqual(want, byteArrayOf(vals[32].toByte(), vals[33].toByte()))) out else null
    }

    private fun ks() = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }

    private fun import(alias: String, key: ByteArray, prot: KeyProtection): Boolean = runCatching {
        ks().setEntry(alias, KeyStore.SecretKeyEntry(SecretKeySpec(key, "HmacSHA256")), prot)
    }.onFailure { UnlockState.log("key $alias not created: ${it.javaClass.simpleName} ${it.message}") }.isSuccess

    /** Returns null on a bad code, else which levels this phone can offer. */
    fun pair(ctx: Context, code: String): String? {
        val secret = decodeCode(code) ?: return null
        unpair(ctx)
        val sign = KeyProperties.PURPOSE_SIGN
        val k = (0..3).map { levelKey(secret, it) }
        import("${P}0", k[0], KeyProtection.Builder(sign).build())
        import("${P}2", k[2], KeyProtection.Builder(sign).build())
        // AUTO, copy 1: released for AUTO_WINDOW_S after the user unlocks the phone (PIN or
        // strong biometric). Needs a secure lock screen. Face unlock on Samsung is "weak" and
        // does NOT count.
        val recent = import("${P}1r", k[1], KeyProtection.Builder(sign)
            .setUserAuthenticationRequired(true)
            .setUserAuthenticationParameters(AUTO_WINDOW_S, KeyProperties.AUTH_DEVICE_CREDENTIAL or KeyProperties.AUTH_BIOMETRIC_STRONG)
            .build())
        // AUTO, copy 2: usable only while the phone is unlocked
        val unlocked = import("${P}1u", k[1], KeyProtection.Builder(sign).setUnlockedDeviceRequired(true).build())
        val bio = import("${P}3", k[3], KeyProtection.Builder(sign)
            .setUserAuthenticationRequired(true)
            .setUserAuthenticationParameters(0, KeyProperties.AUTH_BIOMETRIC_STRONG)
            .setInvalidatedByBiometricEnrollment(true)
            .build())
        k.forEach { it.fill(0) }
        val id = pcIdOf(secret)
        secret.fill(0)
        val levels = listOfNotNull("tap", if (recent || unlocked) "auto" else null, if (bio) "fingerprint" else null).joinToString(",")
        ctx.getSharedPreferences(PREFS, 0).edit().putString("pc_id", id.toHex()).putString("levels", levels).apply()
        return levels
    }

    fun unpair(ctx: Context) {
        runCatching { val s = ks(); listOf("0", "1r", "1u", "2", "3").forEach { s.deleteEntry("$P$it") } }
        ctx.getSharedPreferences(PREFS, 0).edit().remove("pc_id").remove("levels").apply()
    }

    fun pcId(ctx: Context): ByteArray? = ctx.getSharedPreferences(PREFS, 0).getString("pc_id", null)?.fromHex()

    /** A Mac ready to sign at `alias`, or null when the Keystore refuses (not authenticated, locked). */
    fun mac(alias: String): Mac? = runCatching {
        val key = ks().getKey("$P$alias", null) as SecretKey? ?: return null
        Mac.getInstance("HmacSHA256").apply { init(key) }
    }.getOrNull()

    /** AUTO: the Keystore itself enforces "unlocked recently / unlocked now". */
    fun autoMac(): Mac? = mac("1r") ?: mac("1u")

    /**
     * Key for a phone -> PC LOCK event (lock-only, never an unlock answer): the AUTO key when the
     * Keystore releases it, else the no-auth level-0 key, so a watch "Lock PC" / off-wrist hint is
     * not dropped while the phone has been locked for more than [AUTO_WINDOW_S]. autolock's
     * events.rs accepts both, with one shared counter. Returns the Mac and a log label.
     */
    fun lockEventMac(): Pair<Mac, String>? = pickLockEventKey(autoMac(), mac("0"))

    /** Pure part of [lockEventMac] (unit-tested): AUTO first, level 0 as the fallback. */
    fun <M> pickLockEventKey(auto: M?, noAuth: M?): Pair<M, String>? =
        auto?.let { it to "auto key" } ?: noAuth?.let { it to "no-auth key" }

    /** For BiometricPrompt.CryptoObject: init succeeds only after the prompt authenticates it. */
    fun fingerprintMacUninit(): Mac? = runCatching {
        val key = ks().getKey("${P}3", null) as SecretKey? ?: return null
        Mac.getInstance("HmacSHA256").apply { init(key) }
    }.getOrNull()

    fun parseChallenge(b: ByteArray): Challenge? {
        if (b.size != CHALLENGE_LEN || b[0] != VERSION || b[1] != T_CHALLENGE) return null
        return Challenge(b.copyOfRange(2, 18), ByteBuffer.wrap(b, 18, 8).long, b.copyOfRange(26, 34), b[34].toInt() and 0xff)
    }

    fun buildResponse(m: Mac, c: Challenge, level: Int, phoneMs: Long): ByteArray {
        m.update("CU2R".toByteArray())
        m.update(byteArrayOf(VERSION, level.toByte()))
        m.update(c.nonce)
        m.update(ByteBuffer.allocate(8).putLong(c.issuedMs).array())
        m.update(c.pcId)
        m.update(byteArrayOf(c.minLevel.toByte()))
        m.update(ByteBuffer.allocate(8).putLong(phoneMs).array())
        val mac = m.doFinal()
        return ByteBuffer.allocate(RESPONSE_LEN).put(VERSION).put(T_RESPONSE).put(level.toByte()).put(c.nonce).putLong(phoneMs).put(mac).array()
    }

    /** Same vector as pc/tests/known_mac.txt, so a protocol drift shows up on the phone too. */
    fun selfTest(): Boolean {
        val s = "0123456789abcdefghij".toByteArray()
        val m = Mac.getInstance("HmacSHA256").apply { init(SecretKeySpec(levelKey(s, 1), "HmacSHA256")) }
        val r = buildResponse(m, Challenge(ByteArray(16) { 7 }, 1_700_000_000_000L, pcIdOf(s), 1), 1, 1_700_000_000_123L)
        return r.copyOfRange(27, 59).toHex() == "28641abd99e7dc05a49f5403aebe1073bd06fd9bb4383bd56270f21f18a6de55"
    }
}

fun ByteArray.toHex() = joinToString("") { "%02x".format(it) }
fun String.fromHex(): ByteArray = chunked(2).map { it.toInt(16).toByte() }.toByteArray()
