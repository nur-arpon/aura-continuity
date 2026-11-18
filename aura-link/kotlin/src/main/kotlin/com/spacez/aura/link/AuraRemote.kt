package com.spacez.aura.link

import java.nio.ByteBuffer
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.TimeUnit
import javax.crypto.Mac
import kotlin.concurrent.thread

/**
 * Aura Anywhere (item 17), mirror of aura-link/rust/src/remote.rs and link.rs `race_connect`.
 *
 * [RemoteUnlock]: the phone's deliberate remote "Unlock PC" frame
 *   `02 07 | "CUPU" | ver 1 | nonce16 | phone_ms u64 | pc_id 8 | HMAC(key[3], "CUPU"|ver|nonce|ms|pc_id)`.
 * key[3] is the pairing's FINGERPRINT key; on the phone its Keystore entry needs a strong-biometric
 * BiometricPrompt for EVERY use, so the [Mac] handed in here is the one the prompt released
 * (CryptoObject). The PC takes phone_ms as the anti-replay sequence (must grow) and checks +-120 s.
 */
object RemoteUnlock {
    val MAGIC = "CUPU".toByteArray()
    const val VERSION = 1
    const val FRAME_LEN = 2 + 4 + 1 + 16 + 8 + 8 + 32

    fun signedPart(nonce: ByteArray, phoneMs: Long, pcId: ByteArray): ByteArray {
        require(nonce.size == 16 && pcId.size == 8)
        return ByteBuffer.allocate(4 + 1 + 16 + 8 + 8).put(MAGIC).put(VERSION.toByte()).put(nonce).putLong(phoneMs).put(pcId).array()
    }

    /** `mac` must be initialised with key[3] (on the phone: the BiometricPrompt-authenticated Keystore Mac). */
    fun frame(mac: Mac, nonce: ByteArray, phoneMs: Long, pcId: ByteArray): ByteArray {
        val sp = signedPart(nonce, phoneMs, pcId)
        val tag = mac.doFinal(sp)
        return byteArrayOf(2, 7) + sp + tag
    }

    fun isPhoneUnlock(f: ByteArray) = f.size >= 6 && f[0] == 2.toByte() && f[1] == 7.toByte() && f.copyOfRange(2, 6).contentEquals(MAGIC)
}

/**
 * Bluetooth first, tailnet after [headStartMs] (or at once when Bluetooth already failed). The first
 * pipe that opens wins; a pipe that opens later is closed. [second] returning null = no tailnet path.
 */
fun raceConnect(first: () -> Conn, second: () -> Conn?, headStartMs: Long): Conn {
    val q = LinkedBlockingQueue<Pair<Int, Result<Conn?>>>()
    val lock = Any()
    var won = false
    fun start(n: Int, f: () -> Conn?) = thread(isDaemon = true, name = "aura-race-$n") {
        val r = runCatching { f() }
        synchronized(lock) {
            if (won) { r.getOrNull()?.close?.invoke(); return@thread }
            if (r.getOrNull() != null) won = true
        }
        q.put(n to r)
    }
    start(1, first)
    var pending = 1
    var secondLeft = true
    val errs = ArrayList<String>()
    val deadline = System.currentTimeMillis() + headStartMs
    while (true) {
        val got = when {
            secondLeft -> q.poll(maxOf(0, deadline - System.currentTimeMillis()), TimeUnit.MILLISECONDS)
            pending > 0 -> q.take()
            else -> break
        }
        if (got == null) { secondLeft = false; start(2, second); pending++; continue }
        pending--
        val (n, r) = got
        r.getOrNull()?.let { return it }
        r.exceptionOrNull()?.let { errs.add("${if (n == 1) "bluetooth" else "tailnet"}: ${it.message}") }
        if (n == 1 && secondLeft) { secondLeft = false; start(2, second); pending++ }
    }
    throw AuraException(if (errs.isEmpty()) "no path to the peer" else errs.joinToString("; "))
}
