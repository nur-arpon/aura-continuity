package com.spacez.aura.link

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File
import java.net.ServerSocket
import java.net.Socket
import java.security.KeyFactory
import java.security.Signature
import java.security.spec.X509EncodedKeySpec
import java.util.concurrent.CopyOnWriteArrayList
import kotlin.concurrent.thread

/** Shared vectors (aura-link/test-vectors/aura-link-v1.txt, produced by the Rust side) + runtime tests. */
class AuraLinkTest {
    private val v: Map<String, String> by lazy {
        val path = System.getProperty("aura.vectors") ?: "../../aura-link/test-vectors/aura-link-v1.txt"
        File(path).readLines().filter { "=" in it && !it.startsWith("#") }.associate { it.substringBefore('=') to it.substringAfter('=').trim() }
    }
    private fun b(k: String) = v.getValue(k).unhex()

    @Test fun handoff_messages_encode_like_rust() {
        // same bytes are asserted in aura-link/rust/src/msg.rs (handoff_bodies_are_fixed)
        val c = Message.clipboard("text/plain", "hi".toByteArray())
        assertEquals("0a746578742f706c61696e6869", c.body.hex())
        assertEquals(Kind.CLIPBOARD, c.kind); assertTrue(c.wantsAck)
        val (mime, data) = Message.parseClipboard(c.body)
        assertEquals("text/plain", mime); assertEquals("hi", String(data))
        val u = Message.url("https://youtu.be/x", 42_000)
        assertEquals("000000000000a410" + "https://youtu.be/x".toByteArray().hex(), u.body.hex())
        assertEquals("https://youtu.be/x" to 42_000L, Message.parseUrl(u.body))
        assertEquals(null, Message.parseUrl(Message.url("a", null).body).second)
        val a = Message.activity("{}")
        assertEquals(0x23, a.kind); assertTrue(!a.wantsAck)
    }

    @Test fun unsent_message_can_be_withdrawn() {
        val l = AuraLink(LinkConfig(Role.PHONE, Role.PC, ByteArray(8), Credential.Psk(ByteArray(32)), File(System.getProperty("java.io.tmpdir")), backoffMinMs = 30_000),
            { throw AuraException("pc away") }, object : LinkHandler {})
        val id = l.send(Message.activity("{}"))
        assertTrue(l.queued(id)); assertTrue(l.cancel(id)); assertTrue(!l.cancel(id)); assertTrue(!l.queued(id))
        l.stop()
    }

    @Test fun hkdf_rfc5869_case1() {
        val prk = AuraCrypto.hkdfExtract("000102030405060708090a0b0c".unhex(), ByteArray(22) { 0x0b })
        assertEquals("077709362c2e32df0ddc3f0dc47bba6390b6c73bb50f9c3122ec844ad7c2b3e5", prk.hex())
        assertEquals("3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf34007208d5b887185865",
            AuraCrypto.hkdfExpand(prk, "f0f1f2f3f4f5f6f7f8f9".unhex(), 42).hex())
    }

    @Test fun psk_hello_transcript_and_key_schedule_match_rust() {
        assertArrayEquals(b("psk"), AuraCrypto.pskFromK0(b("k0")))
        val hi = Hello.decode(b("hello_i"))
        assertEquals(Role.PC, hi.role); assertEquals("PC", hi.name); assertArrayEquals(b("pc_id"), hi.pcId)
        assertArrayEquals(b("hello_i"), hi.encode())
        assertArrayEquals(b("hello_r"), Hello.decode(b("hello_r")).encode())
        val th = AuraCrypto.transcript(b("hello_i"), b("hello_r"))
        assertArrayEquals(b("th"), th)
        val ei = Ephemeral.fromParts(b("eph_i_scalar"), b("eph_i_pub"))
        val er = Ephemeral.fromParts(b("eph_r_scalar"), b("eph_r_pub"))
        assertArrayEquals(b("z"), ei.agree(b("eph_r_pub")))
        assertArrayEquals(b("z"), er.agree(b("eph_i_pub")))
        val k = AuraCrypto.schedule(b("psk"), b("z"), th)
        assertArrayEquals(b("k_auth"), k.auth); assertArrayEquals(b("k_i2r"), k.i2r); assertArrayEquals(b("k_r2i"), k.r2i); assertArrayEquals(b("k_fast"), k.fast)
        assertArrayEquals(b("auth_i"), byteArrayOf(Wire.R_AUTH, 1) + AuraCrypto.pskProof(k.auth, Role.PC))
        assertArrayEquals(b("auth_r"), byteArrayOf(Wire.R_AUTH, 1) + AuraCrypto.pskProof(k.auth, Role.PHONE))
    }

    @Test fun records_messages_and_bodies_match_rust() {
        val s = Sealer(b("k_i2r"))
        val m1 = Message.text("hello aura").also { it.id = 1 }
        assertArrayEquals(b("msg1_plain"), m1.encode())
        assertArrayEquals(b("msg1_record"), s.seal(m1.encode()))
        val m2 = Message.url("https://youtu.be/dQw4w9WgXcQ", 42_000).also { it.id = 2 }
        assertArrayEquals(b("msg2_plain"), m2.encode())
        assertArrayEquals(b("msg2_record"), s.seal(m2.encode()))
        val o = Opener(b("k_i2r"))
        assertEquals("hello aura", String(Message.decode(o.open(b("msg1_record"))).body))
        assertEquals(Kind.URL, Message.decode(o.open(b("msg2_record"))).kind)
        val a = Message.ack(1).also { it.id = 0x01020304 }
        assertArrayEquals(b("ack_plain"), a.encode())
        assertArrayEquals(b("ack_record"), Sealer(b("k_r2i")).seal(a.encode()))
        val (c2s, s2c) = AuraCrypto.fastKeys(b("k_fast"), b("fast_ticket"))
        assertArrayEquals(b("fast_c2s"), c2s); assertArrayEquals(b("fast_s2c"), s2c)
        assertArrayEquals(b("fast_offer"), FastOffer.decode(b("fast_offer")).encode())
        assertEquals(47900, FastOffer.decode(b("fast_offer")).port)
        val fo = FileOffer.decode(b("file_offer"))
        assertEquals("photo.jpg", fo.name); assertEquals(5_242_880L, fo.size); assertArrayEquals(b("file_offer"), fo.encode())
        val ms = MediaState.decode(b("media_state"))
        assertEquals("Song", ms.title); assertEquals(61_000L, ms.positionMs); assertArrayEquals(b("media_state"), ms.encode())
        // item 15 vectors
        assertArrayEquals(b("ring_on_plain"), Message.ring(true).also { it.id = 7 }.encode())
        assertArrayEquals(b("media_cmd_plain"), Message.mediaCmd(MediaCmd.NEXT).also { it.id = 8 }.encode())
        assertArrayEquals(b("present_plain"), Message.present(PresentCmd.BLACK).also { it.id = 9 }.encode())
        assertArrayEquals(b("media_art_plain"), Message.mediaArt(byteArrayOf(-1, -40, -1, -39)).also { it.id = 10 }.encode())
        val mv = MediaState.decode(b("media_state_vol"))
        assertEquals(42, mv.volume); assertTrue(mv.muted); assertEquals("T", mv.title); assertArrayEquals(b("media_state_vol"), mv.encode())
        assertEquals(null, ms.volume)
        assertEquals(8_000L, Kind.ttlMs(Kind.PRESENT, 300_000)); assertEquals(8_000L, Kind.ttlMs(Kind.MEDIA_CMD, 300_000))
        assertEquals(60_000L, Kind.ttlMs(Kind.RING, 300_000)); assertEquals(300_000L, Kind.ttlMs(Kind.TEXT, 300_000))
        assertTrue(Message.present(1).wantsAck && Message.mediaCmd(1).wantsAck && Message.ring(true).wantsAck && !Message.mediaArt(byteArrayOf(1)).wantsAck)
        // tamper / replay
        val t = b("msg1_record").also { it[5] = (it[5].toInt() xor 1).toByte() }
        assertTrue(runCatching { Opener(b("k_i2r")).open(t) }.isFailure)
    }

    @Test fun watch_session_vectors_and_rust_signature_verify() {
        val th = AuraCrypto.transcript(b("w_hello_i"), b("w_hello_r"))
        assertArrayEquals(b("w_th"), th)
        val k = AuraCrypto.schedule(ByteArray(32), b("z"), th)
        assertArrayEquals(b("w_k_i2r"), k.i2r); assertArrayEquals(b("w_k_r2i"), k.r2i)
        assertArrayEquals(b("w_auth_msg"), AuraCrypto.watchAuthMessage(th))
        val pub = KeyFactory.getInstance("EC").generatePublic(X509EncodedKeySpec(b("watch_pub_der")))
        val ver = Signature.getInstance("SHA256withECDSA").apply { initVerify(pub); update(b("w_auth_msg")) }
        assertTrue("Rust (RFC 6979) signature verifies with the JCA", ver.verify(b("w_auth_sig_der")))
    }

    @Test fun bad_points_are_refused() {
        val p = b("eph_i_pub").also { it[40] = (it[40].toInt() xor 1).toByte() }
        assertTrue(runCatching { Ephemeral.generate().agree(p) }.isFailure)
    }

    // ---------------------------------------------------------------- runtime over loopback TCP

    private class Rec(val tag: String) : LinkHandler {
        val texts = CopyOnWriteArrayList<String>()
        val files = CopyOnWriteArrayList<File>()
        override fun onMessage(link: AuraLink, m: Message): String? {
            if (m.kind == Kind.TEXT) {
                val t = String(m.body)
                if (t == "refuse") return "nope"
                texts += t
                if (t.startsWith("ping:")) link.send(Message.text("pong:" + t.removePrefix("ping:")))
            }
            return null
        }
        override fun onFile(link: AuraLink, file: File, offer: FileOffer) { files += file }
        override fun log(line: String) = println("$tag $line")
    }

    private fun tmp(tag: String) = File(System.getProperty("java.io.tmpdir"), "aura-kt-$tag-${System.nanoTime()}").apply { mkdirs() }

    private fun listen(slot: () -> AuraLink?): Int {
        val ss = ServerSocket(0)
        thread(isDaemon = true) {
            while (true) { val s = ss.accept(); thread(isDaemon = true) { runCatching { AuraLink.respondAndAttach(Conn.tcp(s, "tcp-in"), 5000) { slot() } } } }
        }
        return ss.localPort
    }

    private class P(val pc: AuraLink, val phone: AuraLink, val pcRec: Rec, val phoneRec: Rec, val pcDir: File, val phoneDir: File)

    private fun pair(tweak: LinkConfig.() -> Unit = {}): P {
        val psk = AuraCrypto.pskFromK0(ByteArray(32) { 7 })
        val pcId = ByteArray(8) { 0x5a }
        var pc: AuraLink? = null; var phone: AuraLink? = null
        val pcPort = listen { pc }; val phonePort = listen { phone }
        val (pcDir, phoneDir) = tmp("pc") to tmp("phone")
        val pcRec = Rec("PC   "); val phoneRec = Rec("PHONE")
        pc = AuraLink(LinkConfig(Role.PC, Role.PHONE, pcId, Credential.Psk(psk), pcDir, caps = Caps.FILES or Caps.FAST_CLIENT, name = "kt-pc", fastRole = FastRole.CLIENT).apply(tweak),
            { Conn.tcp(Socket("127.0.0.1", phonePort), "tcp-out") }, pcRec)
        phone = AuraLink(LinkConfig(Role.PHONE, Role.PC, pcId, Credential.Psk(psk), phoneDir, caps = Caps.FILES or Caps.FAST_SERVER, name = "kt-phone",
            fastRole = FastRole.SERVER, fastAddrs = { listOf(byteArrayOf(127, 0, 0, 1)) }).apply(tweak),
            { Conn.tcp(Socket("127.0.0.1", pcPort), "tcp-out") }, phoneRec)
        return P(pc, phone, pcRec, phoneRec, pcDir, phoneDir)
    }

    private fun file(dir: File, name: String, n: Int) = File(dir, name).apply { writeBytes(ByteArray(n) { (it * 31 + 7).toByte() }) }

    @Test fun text_round_trip_and_refusal() {
        val p = pair()
        assertNull(p.pc.waitAcked(p.pc.send(Message.text("ping:7")), 5000))
        val end = System.currentTimeMillis() + 5000
        while ("pong:7" !in p.pcRec.texts && System.currentTimeMillis() < end) Thread.sleep(10)
        assertTrue("pong:7" in p.pcRec.texts)
        assertEquals("nope", p.pc.waitAcked(p.pc.send(Message.text("refuse")), 5000))
        assertNull(p.phone.waitAcked(p.phone.send(Message.text("from phone")), 5000))
        assertTrue("from phone" in p.pcRec.texts)
    }

    @Test fun files_both_ways_over_fast_path_and_bluetooth_path() {
        val p = pair()
        val up = file(p.pcDir, "up.bin", 1_500_000)
        val r = p.pc.waitFile(p.pc.sendFile(up), 30_000)
        assertTrue("fast used: $r", r.bytesFast > 1_000_000)
        val down = file(p.phoneDir, "down.bin", 1_500_000)
        val r2 = p.phone.waitFile(p.phone.sendFile(down), 30_000)
        assertTrue("fast used: $r2", r2.bytesFast > 1_000_000)
        assertArrayEquals(down.readBytes(), p.pcRec.files.first().readBytes())
        val bt = file(p.pcDir, "bt.bin", 300_000)
        val r3 = p.pc.waitFile(p.pc.sendFile(bt, preferFast = false), 30_000)
        assertEquals(0L, r3.bytesFast)
        assertArrayEquals(bt.readBytes(), p.phoneRec.files.last().readBytes())
    }

    @Test fun transfer_resumes_after_drop() {
        val p = pair { chunkBt = 8192; backoffMinMs = 50 }
        val f = file(p.pcDir, "resume.bin", 1_000_000)
        val id = p.pc.sendFile(f, preferFast = false)
        val part = File(p.phoneDir, ".aura-partial")
        val end = System.currentTimeMillis() + 10_000
        while ((part.listFiles()?.firstOrNull()?.length() ?: 0) < 200_000 && System.currentTimeMillis() < end) Thread.sleep(5)
        p.phone.close()
        val r = p.pc.waitFile(id, 30_000)
        assertTrue("resumed, not restarted: $r", r.bytesBt < 1_000_000 + 400_000)
        assertArrayEquals(f.readBytes(), p.phoneRec.files.first().readBytes())
    }

    // ------------------------------------------------------------ item 17 (Aura Anywhere)

    @Test fun remote_unlock_frame_matches_rust_byte_for_byte() {
        val mac = javax.crypto.Mac.getInstance("HmacSHA256").apply { init(javax.crypto.spec.SecretKeySpec(b("k3"), "HmacSHA256")) }
        val f = RemoteUnlock.frame(mac, ByteArray(16) { 0x5a }, 1_700_000_000_123L, b("pc_id"))
        assertEquals(v.getValue("cupu_frame"), f.hex())
        assertEquals(RemoteUnlock.FRAME_LEN, f.size)
        assertTrue(RemoteUnlock.isPhoneUnlock(f))
        assertTrue(!RemoteUnlock.isPhoneUnlock(byteArrayOf(2, 7) + "CUW1".toByteArray()))
    }

    @Test fun item17_kinds_flags_and_tailnet_message() {
        val t = Message.json(Kind.TAILNET, "{\"ip\":\"100.101.102.103\",\"port\":47140}")
        assertEquals(v.getValue("tailnet_msg"), t.encode().hex())
        assertTrue(!Message(Kind.LAST_ACTIVITY, ByteArray(0)).wantsAck)
        assertTrue(Message(Kind.LAST_ACTIVITY_GET, ByteArray(0)).wantsAck)
        assertTrue(Message(Kind.UNLOCK_RESULT, ByteArray(0)).wantsAck)
        assertEquals(0, Kind.priority(Kind.UNLOCK_RESULT))
    }

    private fun fakeConn(label: String, closed: java.util.concurrent.atomic.AtomicInteger) =
        Conn(java.io.ByteArrayInputStream(ByteArray(0)), java.io.ByteArrayOutputStream(), { closed.incrementAndGet() }, label)

    @Test fun race_bluetooth_in_head_start_wins_tailnet_untouched() {
        val closed = java.util.concurrent.atomic.AtomicInteger()
        val tried = java.util.concurrent.atomic.AtomicInteger()
        val c = raceConnect({ Thread.sleep(50); fakeConn("rfcomm", closed) }, { tried.incrementAndGet(); null }, 400)
        assertEquals("rfcomm", c.label)
        Thread.sleep(500)
        assertEquals(0, tried.get()); assertEquals(0, closed.get())
    }

    @Test fun race_slow_bluetooth_loses_and_is_closed() {
        val closed = java.util.concurrent.atomic.AtomicInteger()
        val t0 = System.currentTimeMillis()
        val c = raceConnect({ Thread.sleep(700); fakeConn("rfcomm", closed) }, { fakeConn("tailnet", closed) }, 150)
        assertEquals("tailnet", c.label)
        assertTrue(System.currentTimeMillis() - t0 < 600)
        Thread.sleep(800)
        assertEquals(1, closed.get())
    }

    @Test fun race_failures() {
        val t0 = System.currentTimeMillis()
        val c = raceConnect({ throw AuraException("Bluetooth off") }, { fakeConn("tailnet", java.util.concurrent.atomic.AtomicInteger()) }, 5_000)
        assertEquals("tailnet", c.label); assertTrue(System.currentTimeMillis() - t0 < 1_000)
        val e = runCatching { raceConnect({ throw AuraException("page timeout") }, { throw AuraException("refused") }, 50) }.exceptionOrNull()!!
        assertTrue(e.message!!, e.message!!.contains("bluetooth: page timeout") && e.message!!.contains("tailnet: refused"))
        val e2 = runCatching { raceConnect({ Thread.sleep(100); throw AuraException("page timeout") }, { null }, 10) }.exceptionOrNull()!!
        assertEquals("bluetooth: page timeout", e2.message)
    }
}
