package com.spacez.aura.link

import java.io.InputStream
import java.io.OutputStream

/** Mirror of aura-link/rust/src/handshake.rs (spec in its header). */
object Role {
    const val PC = 1
    const val PHONE = 2
    const val WATCH = 3
    fun name(r: Int) = when (r) { PC -> "pc"; PHONE -> "phone"; WATCH -> "watch"; else -> "?" }
}

object Caps {
    const val FILES = 1
    const val FAST_SERVER = 2
    const val FAST_CLIENT = 4
    const val LOCK_SINK = 8
    const val UNLOCK_SINK = 16
    const val MEDIA = 32
    const val RING = 64
    /** listens for Aura Link on the owner's tailnet (item 17, Aura Anywhere) */
    const val TAILNET = 128
    /** accepts the phone's fingerprint-signed remote Unlock PC (CUPU, AuraRemote.kt) */
    const val REMOTE_UNLOCK = 256
}

class Hello(
    val verMin: Int, val verMax: Int, val role: Int, val pcId: ByteArray, val nonce: ByteArray,
    val eph: ByteArray, val caps: Int, val name: String,
) {
    companion object {
        const val VERSION = 1
        val MAGIC = "AURA".toByteArray()
        fun create(role: Int, pcId: ByteArray, eph: Ephemeral, caps: Int, name: String) =
            Hello(VERSION, VERSION, role, pcId, AuraCrypto.random(32), eph.public, caps, name)

        fun decode(rec: ByteArray): Hello {
            val c = Cur(rec)
            if (c.u8() != Wire.R_HELLO.toInt()) throw AuraException("not a HELLO")
            if (!c.take(4).contentEquals(MAGIC)) throw AuraException("bad magic")
            val vmin = c.u8(); val vmax = c.u8(); val role = c.u8()
            val pcId = c.take(8); val nonce = c.take(32); val eph = c.take(65); val caps = c.u32()
            val name = String(c.take(c.u8()), Charsets.UTF_8)
            if (vmin > vmax) throw AuraException("bad version range")
            return Hello(vmin, vmax, role, pcId, nonce, eph, caps, name)
        }
    }

    fun encode(): ByteArray {
        val n = name.toByteArray(Charsets.UTF_8).let { it.copyOf(minOf(it.size, 64)) }
        return byteArrayOf(Wire.R_HELLO) + MAGIC + byteArrayOf(verMin.toByte(), verMax.toByte(), role.toByte()) +
            pcId + nonce + eph + be32(caps) + byteArrayOf(n.size.toByte()) + n
    }
}

sealed class Credential {
    /** phone <-> PC: PSK from the pairing's level-0 key */
    class Psk(val psk: ByteArray) : Credential()
    /** the watch: signs the transcript with its Keystore key (SHA256withECDSA, DER) */
    class WatchSigner(val sign: (ByteArray) -> ByteArray) : Credential()
}

class Established(val version: Int, val keys: AuraCrypto.Keys, val me: Hello, val peer: Hello, val initiator: Boolean, val th: ByteArray) {
    fun sealer() = Sealer(if (initiator) keys.i2r else keys.r2i)
    fun opener() = Opener(if (initiator) keys.r2i else keys.i2r)
}

object Handshake {
    const val AUTH_NONE = 0
    const val AUTH_PSK = 1
    const val AUTH_WATCH = 2

    fun negotiate(a: Hello, b: Hello): Int {
        val v = minOf(a.verMax, b.verMax)
        if (v < maxOf(a.verMin, b.verMin)) throw AuraException("no common version")
        return v
    }

    private fun salt(c: Credential) = if (c is Credential.Psk) c.psk else ByteArray(32)

    private fun expectedPeer(c: Credential, me: Int) = when (c) {
        is Credential.Psk -> if (me == Role.PC) Role.PHONE else Role.PC
        is Credential.WatchSigner -> Role.PC
    }

    private fun myAuth(c: Credential, k: AuraCrypto.Keys, th: ByteArray, myRole: Int): ByteArray = when (c) {
        is Credential.Psk -> byteArrayOf(Wire.R_AUTH, AUTH_PSK.toByte()) + AuraCrypto.pskProof(k.auth, myRole)
        is Credential.WatchSigner -> byteArrayOf(Wire.R_AUTH, AUTH_WATCH.toByte()) + c.sign(AuraCrypto.watchAuthMessage(th))
    }

    private fun checkPeerAuth(c: Credential, rec: ByteArray, k: AuraCrypto.Keys, peerRole: Int) {
        refusal(rec)?.let { throw AuraException(it) }
        if (rec.size < 2 || rec[0] != Wire.R_AUTH) throw AuraException("expected AUTH")
        val method = rec[1].toInt(); val proof = rec.copyOfRange(2, rec.size)
        when (c) {
            is Credential.Psk -> if (method != AUTH_PSK || !AuraCrypto.ctEq(proof, AuraCrypto.pskProof(k.auth, peerRole)))
                throw AuraException("peer does not hold the pairing key")
            is Credential.WatchSigner -> if (method != AUTH_NONE) throw AuraException("unexpected PC auth method")
        }
    }

    private fun refusal(rec: ByteArray) = if (rec.isNotEmpty() && rec[0] == Wire.R_ERROR) "peer refused: " + String(rec, 1, rec.size - 1) else null

    private fun refuse(out: OutputStream, why: String): Nothing {
        runCatching { Wire.write(out, byteArrayOf(Wire.R_ERROR) + why.toByteArray()) }
        throw AuraException(why)
    }

    private fun checkPeer(me: Hello, peer: Hello, c: Credential, out: OutputStream): Int {
        val v = runCatching { negotiate(me, peer) }.getOrElse { refuse(out, it.message ?: "version") }
        if (peer.role != expectedPeer(c, me.role)) refuse(out, "unexpected peer role ${Role.name(peer.role)}")
        if (!peer.pcId.contentEquals(me.pcId)) refuse(out, "paired with a different PC")
        return v
    }

    fun initiate(ins: InputStream, out: OutputStream, me: Hello, eph: Ephemeral, cred: Credential): Established {
        val hi = me.encode()
        Wire.write(out, hi)
        val hr = Wire.read(ins)
        refusal(hr)?.let { throw AuraException(it) }
        val peer = Hello.decode(hr)
        val v = checkPeer(me, peer, cred, out)
        val th = AuraCrypto.transcript(hi, hr)
        val keys = AuraCrypto.schedule(salt(cred), eph.agree(peer.eph), th)
        Wire.write(out, myAuth(cred, keys, th, me.role))
        checkPeerAuth(cred, Wire.read(ins), keys, peer.role)
        return Established(v, keys, me, peer, true, th)
    }

    /** `choose` returns our HELLO, ephemeral key and credential for this peer, or throws to refuse. */
    fun respond(ins: InputStream, out: OutputStream, choose: (Hello) -> Triple<Hello, Ephemeral, Credential>): Established {
        val hi = Wire.read(ins)
        val peer = runCatching { Hello.decode(hi) }.getOrElse { refuse(out, it.message ?: "bad hello") }
        val (me, eph, cred) = runCatching { choose(peer) }.getOrElse { refuse(out, it.message ?: "refused") }
        val v = checkPeer(me, peer, cred, out)
        val hr = me.encode()
        Wire.write(out, hr)
        val th = AuraCrypto.transcript(hi, hr)
        val keys = AuraCrypto.schedule(salt(cred), eph.agree(peer.eph), th)
        runCatching { checkPeerAuth(cred, Wire.read(ins), keys, peer.role) }.onFailure { refuse(out, it.message ?: "auth") }
        Wire.write(out, myAuth(cred, keys, th, me.role))
        return Established(v, keys, me, peer, false, th)
    }
}

/** Mirror of aura-link/rust/src/msg.rs */
object Kind {
    const val ACK = 0x01; const val ERROR = 0x03; const val PING = 0x04
    const val LOCK = 0x10; const val UNLOCK_REQUEST = 0x11; const val PC_STATE = 0x12; const val UNLOCK_RESULT = 0x13
    /** phone/watch -> PC: JSON {"action":"restart"|"shutdown"|"sleep","from":"<device>"}. Acked. */
    const val PC_POWER = 0x14
    const val TEXT = 0x20; const val URL = 0x21; const val CLIPBOARD = 0x22; const val ACTIVITY = 0x23
    const val LAST_ACTIVITY_GET = 0x24; const val LAST_ACTIVITY = 0x25; const val CONTINUED = 0x26
    const val FILE_OFFER = 0x30; const val FILE_ACCEPT = 0x31; const val FILE_CHUNK = 0x32; const val FILE_DONE = 0x33; const val FILE_CANCEL = 0x34
    const val MEDIA_STATE = 0x40; const val MEDIA_CMD = 0x41; const val RING = 0x42; const val MEDIA_ART = 0x43; const val PRESENT = 0x44
    /** phone/watch -> PC: remote mouse/keyboard JSON (see msg.rs INPUT). Never acked, ttl 2 s. */
    const val INPUT = 0x45
    /** chat-head tree: phone -> PC = ask (empty / {"ask":1}); PC -> phone = laptop recent apps JSON (see msg.rs LAPTOP_APPS). Never acked. */
    const val LAPTOP_APPS = 0x48
    /** Now playing everywhere (see msg.rs): PHONE_MEDIA phone -> PC state JSON, PHONE_MEDIA_CMD PC -> phone control, NOTIF phone -> PC mirror, NOTIF_ACT PC -> phone dismiss/reply. */
    const val PHONE_MEDIA = 0x49; const val PHONE_MEDIA_CMD = 0x4A; const val NOTIF = 0x4B; const val NOTIF_ACT = 0x4C
    const val FAST_REQ = 0x50; const val FAST_OFFER = 0x51; const val FAST_FAIL = 0x52
    const val TAILNET = 0x60
    /** PC -> phone: where the PC's Sunshine host listens (JSON, see msg.rs LAPTOP_INFO). Never acked. */
    const val LAPTOP_INFO = 0x61

    fun priority(k: Int) = when (k) { ACK, LOCK, UNLOCK_REQUEST, UNLOCK_RESULT, PC_STATE, PC_POWER, ERROR -> 0; FILE_CHUNK -> 2; else -> 1 }
    private val NO_ACK = setOf(ACK, FILE_CHUNK, FILE_ACCEPT, FILE_OFFER, FILE_DONE, FAST_REQ, FAST_OFFER, FAST_FAIL, ERROR, MEDIA_STATE, MEDIA_ART, ACTIVITY, LAST_ACTIVITY, TAILNET, LAPTOP_INFO, INPUT, LAPTOP_APPS, PHONE_MEDIA, NOTIF)
    /** How long an unsent message may wait for a session; live controls must never fire minutes later. */
    fun ttlMs(k: Int, defaultMs: Long): Long = when (k) {
        LOCK, UNLOCK_REQUEST -> 110_000
        MEDIA_CMD, PRESENT -> minOf(8_000, defaultMs)
        INPUT -> minOf(2_000, defaultMs)
        LAPTOP_APPS -> minOf(20_000, defaultMs)
        PHONE_MEDIA -> minOf(10_000, defaultMs)
        PHONE_MEDIA_CMD -> minOf(8_000, defaultMs)
        NOTIF -> minOf(30_000, defaultMs)
        NOTIF_ACT -> minOf(20_000, defaultMs)
        RING -> minOf(60_000, defaultMs)
        PC_POWER -> minOf(20_000, defaultMs) // a power command must never fire minutes later
        else -> defaultMs
    }
    fun defaultFlags(k: Int) = if (k in NO_ACK) 0 else Message.FLAG_ACK
}

class Message(val kind: Int, val flags: Int, var id: Int, val body: ByteArray) {
    constructor(kind: Int, body: ByteArray) : this(kind, Kind.defaultFlags(kind), 0, body)
    val wantsAck get() = flags and FLAG_ACK != 0
    fun encode() = byteArrayOf(kind.toByte(), flags.toByte()) + be32(id) + body

    companion object {
        const val FLAG_ACK = 1
        fun decode(b: ByteArray): Message { val c = Cur(b); return Message(c.u8(), c.u8(), c.u32(), c.rest()) }
        fun text(s: String) = Message(Kind.TEXT, s.toByteArray(Charsets.UTF_8))
        fun ack(id: Int) = Message(Kind.ACK, be32(id))
        fun url(url: String, positionMs: Long?) = Message(Kind.URL, be64(positionMs ?: -1L) + url.toByteArray(Charsets.UTF_8))
        fun ring(on: Boolean) = Message(Kind.RING, byteArrayOf(if (on) 1 else 0))
        fun mediaCmd(cmd: Int) = Message(Kind.MEDIA_CMD, byteArrayOf(cmd.toByte()))
        fun present(cmd: Int) = Message(Kind.PRESENT, byteArrayOf(cmd.toByte()))
        fun laptopApps(json: String) = Message(Kind.LAPTOP_APPS, json.toByteArray(Charsets.UTF_8))
        fun input(json: String) = Message(Kind.INPUT, json.toByteArray(Charsets.UTF_8))
        fun mediaArt(jpeg: ByteArray) = Message(Kind.MEDIA_ART, jpeg.copyOf(minOf(jpeg.size, MEDIA_ART_MAX)))
        const val MEDIA_ART_MAX = 40 * 1024
        /** body: mime_len u8 | mime | bytes; at most [CLIPBOARD_MAX] bytes inline, bigger goes as a file named aura-clipboard* */
        fun clipboard(mime: String, data: ByteArray): Message {
            val m = mime.toByteArray(Charsets.UTF_8).let { it.copyOf(minOf(it.size, 255)) }
            return Message(Kind.CLIPBOARD, byteArrayOf(m.size.toByte()) + m + data)
        }
        fun activity(json: String) = Message(Kind.ACTIVITY, json.toByteArray(Charsets.UTF_8))
        fun json(kind: Int, json: String) = Message(kind, json.toByteArray(Charsets.UTF_8))
        const val CLIPBOARD_MAX = 60 * 1024
        const val CLIPBOARD_FILE_PREFIX = "aura-clipboard"
        /** (url, positionMs or null) */
        fun parseUrl(body: ByteArray): Pair<String, Long?> {
            val c = Cur(body); val p = c.u64()
            return String(c.rest(), Charsets.UTF_8) to (if (p == -1L) null else p)
        }
        /** (mime, bytes) */
        fun parseClipboard(body: ByteArray): Pair<String, ByteArray> {
            val c = Cur(body); val n = c.u8()
            return String(c.take(n), Charsets.UTF_8) to c.rest()
        }
    }
}

/** Bodies of Kind.MEDIA_CMD (mirror of msg.rs media_cmd). */
object MediaCmd {
    const val PLAY_PAUSE = 1; const val NEXT = 2; const val PREV = 3; const val VOL_UP = 4; const val VOL_DOWN = 5
    const val MUTE = 6; const val REFRESH = 7; const val STOP_UPDATES = 8
}

/** Bodies of Kind.PRESENT (mirror of msg.rs present_cmd). */
object PresentCmd {
    const val NEXT = 1; const val PREV = 2; const val BLACK = 3; const val WHITE = 4; const val START = 5; const val END = 6
}

/** [volume] 0..100 and [muted] are appended only when the PC knows them (older peers ignore them). */
class MediaState(val playing: Boolean, val positionMs: Long, val durationMs: Long, val title: String, val artist: String,
                 val volume: Int? = null, val muted: Boolean = false) {
    fun encode(): ByteArray {
        fun s(x: String) = x.toByteArray(Charsets.UTF_8).let { it.copyOf(minOf(it.size, 512)) }.let { be16(it.size) + it }
        return byteArrayOf(if (playing) 1 else 0) + be64(positionMs) + be64(durationMs) + s(title) + s(artist) +
            (volume?.let { byteArrayOf(it.coerceIn(0, 100).toByte(), if (muted) 1 else 0) } ?: ByteArray(0))
    }
    companion object {
        fun decode(b: ByteArray): MediaState {
            val c = Cur(b)
            val playing = c.u8() != 0; val pos = c.u64(); val dur = c.u64()
            val title = String(c.take(c.u16()), Charsets.UTF_8); val artist = String(c.take(c.u16()), Charsets.UTF_8)
            val r = c.rest()
            return if (r.size >= 2) MediaState(playing, pos, dur, title, artist, (r[0].toInt() and 0xff).coerceAtMost(100), r[1].toInt() != 0)
            else MediaState(playing, pos, dur, title, artist)
        }
    }
}

class FastOffer(val port: Int, val ticket: ByteArray, val addrs: List<ByteArray>) {
    fun encode(): ByteArray = be16(port) + ticket + byteArrayOf(addrs.size.coerceAtMost(16).toByte()) + addrs.take(16).fold(ByteArray(0)) { a, x -> a + x }
    companion object {
        fun decode(b: ByteArray): FastOffer {
            val c = Cur(b)
            val port = c.u16(); val t = c.take(16); val n = c.u8()
            return FastOffer(port, t, List(n) { c.take(4) })
        }
    }
}

class FileOffer(val id: ByteArray, val size: Long, val sha256: ByteArray, val name: String, val mime: String) {
    fun encode(): ByteArray {
        val n = name.toByteArray(Charsets.UTF_8).let { it.copyOf(minOf(it.size, 1024)) }
        val m = mime.toByteArray(Charsets.UTF_8).let { it.copyOf(minOf(it.size, 255)) }
        return id + be64(size) + sha256 + be16(n.size) + n + byteArrayOf(m.size.toByte()) + m
    }
    companion object {
        fun decode(b: ByteArray): FileOffer {
            val c = Cur(b)
            return FileOffer(c.take(16), c.u64(), c.take(32), String(c.take(c.u16()), Charsets.UTF_8), String(c.take(c.u8()), Charsets.UTF_8))
        }
    }
}
