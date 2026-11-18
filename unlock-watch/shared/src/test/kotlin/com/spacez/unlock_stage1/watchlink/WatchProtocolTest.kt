package com.spacez.unlock_stage1.watchlink

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.security.KeyPair
import java.security.KeyPairGenerator
import java.security.Signature
import java.security.spec.ECGenParameterSpec

class WatchProtocolTest {
    private fun keyPair(): KeyPair = KeyPairGenerator.getInstance("EC").apply { initialize(ECGenParameterSpec("secp256r1")) }.generateKeyPair()
    private val kp = keyPair()
    private val signer = StatusSigner { d -> Signature.getInstance(WatchProtocol.SIG_ALG).run { initSign(kp.private); update(d); sign() } }
    private val nonce = ByteArray(16) { it.toByte() }
    private val now = 1_800_000_000_000L

    private fun status(kind: Kind = Kind.REPLY, onWrist: Boolean = true, known: Boolean = true, locked: Boolean = false,
                       ms: Long = now, n: ByteArray = nonce, seq: Long = 1) =
        WatchStatus(kind, n, known, onWrist, locked, false, ms, seq)

    @Test fun roundTrip() {
        val s = status()
        val d = WatchCodec.decodeVerified(WatchCodec.encode(s, signer), kp.public)
        assertNotNull(d); assertEquals(s, d!!.status); assertEquals(0, d.payload.size)
    }

    @Test fun payloadIsSigned() {
        val f = WatchCodec.encode(status(Kind.COMMAND), signer, "lock_pc".toByteArray())
        assertEquals("lock_pc", String(WatchCodec.decodeVerified(f, kp.public)!!.payload))
        f[f.size - 1] = 'x'.code.toByte()
        assertNull(WatchCodec.decodeVerified(f, kp.public))
    }

    @Test fun everyFlippedBitIsRejected() {
        val f = WatchCodec.encode(status(), signer)
        for (i in 0 until WatchProtocol.HEADER_LEN) {
            val t = f.copyOf(); t[i] = (t[i].toInt() xor 1).toByte()
            assertNull("byte $i", WatchCodec.decodeVerified(t, kp.public))
        }
    }

    @Test fun wrongKeyRejected() {
        val f = WatchCodec.encode(status(), signer)
        assertNull(WatchCodec.decodeVerified(f, keyPair().public))
        assertEquals(Verdict.BAD_SIGNATURE, WatchPolicy.evaluate(f, keyPair().public, nonce, now))
    }

    @Test fun truncatedAndGarbageRejected() {
        val f = WatchCodec.encode(status(), signer)
        for (n in listOf(0, 5, WatchProtocol.HEADER_LEN, WatchProtocol.HEADER_LEN + 2, f.size - 1))
            assertNull(WatchCodec.decodeVerified(f.copyOf(n), kp.public))
        assertNull(WatchCodec.decodeVerified(ByteArray(200) { 7 }, kp.public))
    }

    @Test fun policyAllowsOnlyWornAndUnlocked() {
        fun v(s: WatchStatus) = WatchPolicy.evaluate(WatchCodec.encode(s, signer), kp.public, nonce, now)
        assertEquals(Verdict.ALLOW, v(status()))
        assertEquals(Verdict.OFF_WRIST, v(status(onWrist = false)))
        assertEquals(Verdict.WRIST_UNKNOWN, v(status(known = false)))
        assertEquals(Verdict.WATCH_LOCKED, v(status(locked = true)))
        assertEquals(Verdict.WRONG_NONCE, v(status(n = ByteArray(16))))
        assertEquals(Verdict.WRONG_KIND, v(status(kind = Kind.PUSH)))
        assertEquals(Verdict.CLOCK_SKEW, v(status(ms = now - WatchProtocol.MAX_SKEW_MS - 1)))
        assertEquals(Verdict.ALLOW, v(status(ms = now + WatchProtocol.MAX_SKEW_MS)))
        assertEquals(Verdict.NO_REPLY, WatchPolicy.evaluate(null, kp.public, nonce, now))
    }

    @Test fun relayedApproval() {
        val acc = setOf(Kind.APPROVE)
        fun v(s: WatchStatus) = WatchPolicy.evaluate(WatchCodec.encode(s, signer), kp.public, nonce, now, acc)
        assertEquals(Verdict.ALLOW, v(status(Kind.APPROVE, onWrist = false, known = false)))
        assertEquals(Verdict.DENIED, v(status(Kind.DENY)))
        assertEquals(Verdict.WATCH_LOCKED, v(status(Kind.APPROVE, locked = true)))
        assertEquals(Verdict.WRONG_NONCE, v(status(Kind.APPROVE, n = ByteArray(16))))
        assertEquals(Verdict.WRONG_KIND, v(status(Kind.REPLY)))
    }

    @Test fun readyFlag() {
        assertTrue(status().readyForAutoUnlock)
        assertFalse(status(onWrist = false).readyForAutoUnlock)
        assertFalse(status(known = false).readyForAutoUnlock)
        assertFalse(status(locked = true).readyForAutoUnlock)
    }

    @Test fun seqGuardRejectsReplay() {
        val g = SeqGuard()
        assertTrue(g.accept(status(seq = 5)))
        assertFalse(g.accept(status(seq = 5)))
        assertFalse(g.accept(status(seq = 4)))
        assertTrue(g.accept(status(seq = 6)))
    }

    @Test fun fingerprintStable() {
        val fp = WatchProtocol.fingerprint(kp.public.encoded)
        assertTrue(fp.matches(Regex("[0-9A-F]{4}-[0-9A-F]{4}-[0-9A-F]{4}")))
        assertEquals(fp, WatchProtocol.fingerprint(WatchProtocol.publicKeyFromDer(kp.public.encoded)!!.encoded))
    }

    @Test fun commandsAndPcState() {
        WatchCommand.entries.forEach { assertEquals(it, WatchCommand.of(it.wire)) }
        assertNull(WatchCommand.of("rm -rf"))
        val p = PcState(true, false, "LAPTOP;x")
        assertEquals(PcState(true, false, "LAPTOPx"), PcState.decode(p.encode()))
        assertEquals(PcState(false, null, ""), PcState.decode("garbage"))
    }
}

class PcEventFrameTest {
    @Test fun knownVector() {
        // Same vector is checked by pc-patch (Rust), so both sides agree byte for byte.
        val mac = javax.crypto.Mac.getInstance("HmacSHA256").apply {
            init(javax.crypto.spec.SecretKeySpec(ByteArray(32) { 1 }, "HmacSHA256"))
        }
        val f = PcEventFrame.build(mac, PcEvent.LOCK_NOW, 1_700_000_000_000L, 7, ByteArray(8) { 2 })
        assertEquals(PcEventFrame.LEN, f.size)
        assertEquals("4355314501010000018bcfe56800000000000000000702020202020202029c499394b898eebb1427525c017c092e26842ed2f281bde86915f645db84d3da",
            f.joinToString("") { "%02x".format(it) })
    }

    /** v2 vector shared with the Rust side (test-vectors/pc-event-v2.txt): key = level-1 key from the pairing secret. */
    @Test fun v2VectorMatchesPcSide() {
        val v = java.io.File("test-vectors/pc-event-v2.txt").readLines()
            .filter { it.isNotBlank() && !it.startsWith("#") }.associate { it.substringBefore('=') to it.substringAfter('=') }
        fun hmac(key: ByteArray, m: String) = javax.crypto.Mac.getInstance("HmacSHA256")
            .apply { init(javax.crypto.spec.SecretKeySpec(key, "HmacSHA256")) }.doFinal(m.toByteArray())
        fun hex(b: ByteArray) = b.joinToString("") { "%02x".format(it) }
        val secret = v.getValue("secret").toByteArray()
        val key1 = hmac(secret, "continuity-unlock/v2/key/1")
        val pcId = hmac(secret, "continuity-unlock/v2/id").copyOf(8)
        assertEquals(v.getValue("key1"), hex(key1))
        assertEquals(v.getValue("pc_id"), hex(pcId))
        val mac = javax.crypto.Mac.getInstance("HmacSHA256").apply { init(javax.crypto.spec.SecretKeySpec(key1, "HmacSHA256")) }
        val f = PcEventFrame.build(mac, PcEvent.LOCK_NOW, 1_700_000_000_000L, 7, pcId)
        assertEquals(v.getValue("frame"), hex(f))
        assertEquals(v.getValue("wire"), hex(PcEventFrame.wire(f)))
        // a v1-style key (what the old patch used) must NOT produce the same MAC
        val v1 = hmac(secret, "continuity-unlock/v1/key")
        val other = PcEventFrame.build(javax.crypto.Mac.getInstance("HmacSHA256").apply { init(javax.crypto.spec.SecretKeySpec(v1, "HmacSHA256")) }, PcEvent.LOCK_NOW, 1_700_000_000_000L, 7, pcId)
        assert(hex(other) != hex(f))
    }
}
