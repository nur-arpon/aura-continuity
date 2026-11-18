package com.spacez.aura.link

import java.io.File
import java.io.InputStream
import java.io.OutputStream
import java.io.RandomAccessFile
import java.net.Inet4Address
import java.net.InetAddress
import java.net.InetSocketAddress
import java.net.NetworkInterface
import java.net.ServerSocket
import java.net.Socket
import java.net.SocketTimeoutException
import java.security.MessageDigest
import java.util.ArrayDeque
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.locks.ReentrantLock
import kotlin.concurrent.thread
import kotlin.concurrent.withLock

/**
 * Aura Link runtime, mirror of aura-link/rust/src/link.rs (same rules, same wire bytes):
 * one session per paired device, either side connects, PC-initiated wins a simultaneous connect,
 * one driver thread writes in priority order (lock/unlock/acks > control > file chunks),
 * acked messages resent on the next session, receivers drop duplicate ids, reconnect only while
 * something waits (backoff 0.5 s .. 60 s), idle close, resumable files, LAN fast path.
 * With nothing queued and no session the driver thread sleeps in Condition.await(): no timers,
 * no polling, no wakelocks.
 */
class Conn(val input: InputStream, val output: OutputStream, val close: () -> Unit, val label: String) {
    companion object {
        fun tcp(s: Socket, label: String): Conn {
            s.tcpNoDelay = true
            return Conn(s.getInputStream(), s.getOutputStream(), { runCatching { s.close() } }, label)
        }
    }
}

enum class FastRole { NONE, SERVER, CLIENT }

interface LinkHandler {
    /** null = ok (ACK), else the reason sent back as ERROR */
    fun onMessage(link: AuraLink, m: Message): String? = null
    fun onFile(link: AuraLink, file: File, offer: FileOffer) {}
    fun onSession(link: AuraLink, up: Boolean) {}
    fun log(line: String) {}
}

class LinkConfig(
    val myRole: Int, val peerRole: Int, val pcId: ByteArray, val cred: Credential, val inbox: File,
    var caps: Int = Caps.FILES, var name: String = "",
    var idleCloseMs: Long = 30_000, var warmHoldMs: Long = 180_000, var ackTimeoutMs: Long = 15_000, var handshakeTimeoutMs: Long = 10_000,
    var backoffMinMs: Long = 500, var backoffMaxMs: Long = 60_000, var msgTtlMs: Long = 300_000,
    var chunkBt: Int = 16 * 1024, var chunkFast: Int = 60 * 1024,
    var fastRole: FastRole = FastRole.NONE, var fastMinSize: Long = 256 * 1024, var fastWaitMs: Long = 4_000,
    var fastAddrs: () -> List<ByteArray> = { FastPath.localIpv4() },
)

class FileReport(val bytes: Long, val elapsedMs: Long, val bytesFast: Long, val bytesBt: Long) {
    val mbitS get() = bytes * 8.0 / 1e6 / (maxOf(elapsedMs, 1) / 1000.0)
    override fun toString() = "%d bytes in %d ms = %.2f Mbit/s (fast %d, bt %d)".format(bytes, elapsedMs, mbitS, bytesFast, bytesBt)
}

object FastPath {
    /** IPv4 addresses of up, non-loopback interfaces (Wi-Fi, USB tethering, VPN). */
    fun localIpv4(): List<ByteArray> = runCatching {
        NetworkInterface.getNetworkInterfaces().toList().filter { it.isUp && !it.isLoopback }
            .flatMap { it.inetAddresses.toList() }.filterIsInstance<Inet4Address>().map { it.address }
    }.getOrDefault(emptyList())

    class FastConn(val socket: Socket, val sealer: Sealer, val opener: Opener)

    class Listener(addrs: List<ByteArray>) {
        val server = ServerSocket().apply { reuseAddress = true; bind(InetSocketAddress(0)) }
        val offer = FastOffer(server.localPort, AuraCrypto.random(16), addrs)
        private val expires = System.currentTimeMillis() + 10_000

        fun accept(kFast: ByteArray): FastConn {
            try {
                while (true) {
                    val left = expires - System.currentTimeMillis()
                    if (left <= 0) throw AuraException("nobody connected to the fast path in 10 s")
                    server.soTimeout = left.toInt()
                    val s = try { server.accept() } catch (_: SocketTimeoutException) { continue }
                    try {
                        s.soTimeout = 3000
                        val h = Wire.read(s.getInputStream())
                        if (h.size == 17 && h[0] == Wire.R_FAST_HELLO && AuraCrypto.ctEq(h.copyOfRange(1, 17), offer.ticket)) {
                            Wire.write(s.getOutputStream(), byteArrayOf(Wire.R_FAST_OK))
                            s.soTimeout = 0; s.tcpNoDelay = true
                            val (c2s, s2c) = AuraCrypto.fastKeys(kFast, offer.ticket)
                            return FastConn(s, Sealer(s2c), Opener(c2s))
                        }
                    } catch (_: Exception) {}
                    runCatching { s.close() }
                }
            } finally {
                runCatching { server.close() }
            }
        }
    }

    fun connect(offer: FastOffer, kFast: ByteArray, timeoutMs: Int): FastConn {
        val errs = mutableListOf<String>()
        for (a in offer.addrs) {
            try {
                val s = Socket()
                s.connect(InetSocketAddress(InetAddress.getByAddress(a), offer.port), timeoutMs)
                s.tcpNoDelay = true; s.soTimeout = 3000
                Wire.write(s.getOutputStream(), byteArrayOf(Wire.R_FAST_HELLO) + offer.ticket)
                val ok = Wire.read(s.getInputStream())
                if (!(ok.size == 1 && ok[0] == Wire.R_FAST_OK)) { s.close(); throw AuraException("refused") }
                s.soTimeout = 0
                val (c2s, s2c) = AuraCrypto.fastKeys(kFast, offer.ticket)
                return FastConn(s, Sealer(c2s), Opener(s2c))
            } catch (e: Exception) { errs += "${a.joinToString(".") { (it.toInt() and 0xff).toString() }}: ${e.message}" }
        }
        throw AuraException("fast path unreachable: ${errs.joinToString("; ")}")
    }
}

class AuraLink(val cfg: LinkConfig, private val connector: (() -> Conn)?, private val handler: LinkHandler) {
    private class Out(val m: Message, val deadline: Long, var sentAt: Long = 0)
    private class Pipe(val out: OutputStream, val sealer: Sealer)
    private class Session(val gen: Long, val pipe: Pipe, val close: () -> Unit, val started: Long, val pcInitiated: Boolean,
                          val peer: Hello, val kFast: ByteArray, val label: String)
    private enum class FastState { IDLE, NEGOTIATING, UP, FAILED }

    private class SendJob(val offer: FileOffer, val file: File, val preferFast: Boolean?) {
        var next = 0L; var accepted = false; var done: String? = null; var ok = false
        val created = System.currentTimeMillis(); var firstChunk = 0L; var finished = 0L
        var bytesFast = 0L; var bytesBt = 0L
        val remaining get() = offer.size - next
        private var raf: RandomAccessFile? = null
        fun nextChunk(max: Int): Pair<Long, ByteArray>? {
            if (!accepted || done != null || next >= offer.size) return null
            val f = raf ?: RandomAccessFile(file, "r").also { raf = it }
            val n = minOf(remaining, max.toLong()).toInt()
            val b = ByteArray(n); f.seek(next); f.readFully(b)
            val off = next; next += n
            if (firstChunk == 0L) firstChunk = System.currentTimeMillis()
            return off to b
        }
        fun closeFile() { runCatching { raf?.close() }; raf = null }
    }

    private class RecvJob(val offer: FileOffer, val part: File, var received: Long, val raf: RandomAccessFile)

    private val lock = ReentrantLock()
    private val cv = lock.newCondition()
    private val queue = Array(3) { ArrayDeque<Out>() }
    private val unacked = ArrayList<Out>()
    private val outcomes = ArrayDeque<Pair<Int, String?>>()
    private var session: Session? = null
    private var gen = 0L
    private var nextId = (AuraCrypto.random(4).fold(0) { a, x -> (a shl 8) or (x.toInt() and 0xff) }) or 1
    private val seen = ArrayDeque<Int>()
    private val sends = ArrayList<SendJob>()
    private val recvs = HashMap<String, RecvJob>()
    private val doneRecv = ArrayDeque<String>()
    private var fast: Pair<Pipe, () -> Unit>? = null
    private var fastState = FastState.IDLE
    private var fastUntil = 0L
    private var fastLastUse = 0L
    private var backoff = cfg.backoffMinMs
    private var nextTry = 0L
    private var lastActivity = System.currentTimeMillis()
    private var warmUntil = 0L
    private var stopped = false
    var sessions = 0L; private set

    init { thread(name = "aura-link-${Role.name(cfg.peerRole)}", isDaemon = true) { driver() } }

    private fun log(s: String) = handler.log("[${Role.name(cfg.peerRole)}] $s")
    private fun now() = System.currentTimeMillis()
    private fun out(m: Message) = Out(m, now() + 300_000)

    // ------------------------------------------------------------ API

    /** Keep the session open at least [ms] from now (e.g. while a chat head is visible). */
    fun keepWarm(ms: Long) = lock.withLock { warmUntil = maxOf(warmUntil, now() + ms); cv.signalAll() }

    /** Mirrors Rust msg::is_user_facing: a person did something, so the next tap should not pay a reconnect. */
    private fun userFacing(k: Int) = when (k) {
        Kind.LOCK, Kind.UNLOCK_REQUEST, Kind.PC_POWER, Kind.TEXT, Kind.URL, Kind.CLIPBOARD, Kind.CONTINUED,
        Kind.LAST_ACTIVITY_GET, Kind.FILE_OFFER, Kind.MEDIA_CMD, Kind.PRESENT, Kind.INPUT, Kind.LAPTOP_APPS, Kind.PHONE_MEDIA_CMD, Kind.NOTIF_ACT -> true
        else -> false
    }

    fun send(m: Message): Int = lock.withLock {
        m.id = nextId; nextId = (nextId + 1).let { if (it == 0) 1 else it }
        val ttl = Kind.ttlMs(m.kind, cfg.msgTtlMs)
        if (userFacing(m.kind)) warmUntil = maxOf(warmUntil, now() + cfg.warmHoldMs)
        queue[Kind.priority(m.kind)].addLast(Out(m, now() + ttl)); cv.signalAll(); m.id
    }

    fun sendFile(f: File, preferFast: Boolean? = null): ByteArray {
        val sha = MessageDigest.getInstance("SHA-256").let { d -> f.inputStream().use { i -> val b = ByteArray(1 shl 16); while (true) { val n = i.read(b); if (n < 0) break; d.update(b, 0, n) } }; d.digest() }
        val job = SendJob(FileOffer(AuraCrypto.random(16), f.length(), sha, f.name, mime(f.name)), f, preferFast)
        lock.withLock {
            sends += job
            queue[1].addLast(Out(Message(Kind.FILE_OFFER, job.offer.encode()), now() + 3_600_000)); cv.signalAll()
        }
        return job.offer.id
    }

    fun waitFile(id: ByteArray, timeoutMs: Long): FileReport = lock.withLock {
        val end = now() + timeoutMs
        while (true) {
            val j = sends.find { it.offer.id.contentEquals(id) } ?: throw AuraException("unknown file")
            j.done?.let { err ->
                sends.remove(j); j.closeFile()
                if (!j.ok) throw AuraException(err)
                return FileReport(j.offer.size, (if (j.finished > 0) j.finished else now()) - (if (j.firstChunk > 0) j.firstChunk else j.created), j.bytesFast, j.bytesBt)
            }
            val left = end - now()
            if (left <= 0) throw AuraException("timed out")
            cv.await(left, TimeUnit.MILLISECONDS)
        }
        @Suppress("UNREACHABLE_CODE") throw IllegalStateException()
    }

    /** null = acked, else the error (or "timed out"). */
    fun waitAcked(id: Int, timeoutMs: Long): String? = lock.withLock {
        val end = now() + timeoutMs
        while (true) {
            outcomes.find { it.first == id }?.let { return it.second }
            val left = end - now()
            if (left <= 0) return "no ack (timed out)"
            cv.await(left, TimeUnit.MILLISECONDS)
        }
        @Suppress("UNREACHABLE_CODE") null
    }

    /** Withdraw a queued, never-written message (mirror of Rust Link::cancel). true = it will not be delivered. */
    fun cancel(id: Int): Boolean = lock.withLock {
        for (q in queue) { val o = q.firstOrNull { it.m.id == id } ?: continue; q.remove(o); return true }
        false
    }

    /** true while message [id] still waits in the queue (not written yet). */
    fun queued(id: Int): Boolean = lock.withLock { queue.any { q -> q.any { it.m.id == id } } }

    val connected get() = lock.withLock { session != null }
    val fastUp get() = lock.withLock { fastState == FastState.UP }
    val peer get() = lock.withLock { session?.peer }

    fun status(): String = lock.withLock {
        "${Role.name(cfg.peerRole)}: session=${session?.let { "${it.label}#${it.gen}" } ?: "none"} fast=$fastState queued=${queue[0].size}/${queue[1].size}/${queue[2].size} unacked=${unacked.size} files out=${sends.size} in=${recvs.size} sessions=$sessions"
    }

    fun close() { val g = lock.withLock { session?.gen } ?: return; teardown(g, "closed by the application") }
    fun stop() { close(); lock.withLock { stopped = true; cv.signalAll() } }

    fun responderParts(): Triple<Hello, Ephemeral, Credential> {
        val e = Ephemeral.generate()
        return Triple(Hello.create(cfg.myRole, cfg.pcId, e, cfg.caps, cfg.name), e, cfg.cred)
    }

    fun accept(conn: Conn, est: Established) = install(conn, est)

    // ------------------------------------------------------------ sessions

    private fun install(conn: Conn, est: Established): Boolean {
        val pcInit = (est.initiator && cfg.myRole == Role.PC) || (!est.initiator && est.peer.role == Role.PC)
        var old: Long? = null
        lock.withLock {
            session?.let { cur ->
                if (now() - cur.started < 5_000 && cur.pcInitiated && !pcInit) {
                    conn.close(); log("simultaneous connect: kept the PC-initiated session"); return false
                }
                old = cur.gen
            }
        }
        old?.let { teardown(it, "replaced by a newer session") }
        val g: Long
        lock.withLock {
            gen++; sessions++; g = gen
            session = Session(g, Pipe(conn.output, est.sealer()), conn.close, now(), pcInit, est.peer, est.keys.fast, conn.label)
            fastState = FastState.IDLE; backoff = cfg.backoffMinMs; nextTry = 0; lastActivity = now()
            for (j in sends) if (j.done == null) {
                j.accepted = false
                val o = j.offer.encode()
                if (queue[1].none { it.m.kind == Kind.FILE_OFFER && it.m.body.contentEquals(o) }) queue[1].addLast(Out(Message(Kind.FILE_OFFER, o), now() + 3_600_000))
            }
            cv.signalAll()
        }
        log("session #$g up over ${conn.label} (${if (est.initiator) "to" else "from"} ${est.peer.name}, v${est.version}, peer caps 0x${Integer.toHexString(est.peer.caps)})")
        val opener = est.opener()
        thread(name = "aura-link-read", isDaemon = true) { reader(conn.input, opener, g, false) }
        handler.onSession(this, true)
        return true
    }

    private fun teardown(g: Long, why: String) {
        lock.withLock {
            val s = session ?: return
            if (s.gen != g) return
            session = null
            runCatching { s.close() }
            fast?.second?.let { runCatching { it() } }; fast = null; fastState = FastState.IDLE
            unacked.sortedByDescending { it.m.id }.forEach { it.sentAt = 0; queue[Kind.priority(it.m.kind)].addFirst(it) }
            unacked.clear(); queue[2].clear()
            sends.forEach { it.accepted = false }
            cv.signalAll()
        }
        log("session #$g down: $why")
        handler.onSession(this, false)
    }

    private fun reader(ins: InputStream, opener: Opener, g: Long, isFast: Boolean) {
        val why = try {
            val d = java.io.DataInputStream(java.io.BufferedInputStream(ins, 1 shl 16))
            while (true) incoming(g, Message.decode(opener.open(Wire.read(d))), isFast)
            @Suppress("UNREACHABLE_CODE") ""
        } catch (e: Exception) { "read: ${e.message}" }
        if (isFast) {
            lock.withLock {
                if (session?.gen == g && fastState == FastState.UP) {
                    fast?.second?.let { runCatching { it() } }; fast = null; fastState = FastState.IDLE; cv.signalAll()
                }
            }
        } else teardown(g, why)
    }

    // ------------------------------------------------------------ incoming

    private fun incoming(g: Long, m: Message, viaFast: Boolean) {
        var deliver = false
        var finished: Pair<File, FileOffer>? = null
        var openServer: ByteArray? = null
        lock.withLock {
            val s = session ?: return
            if (s.gen != g) return
            lastActivity = now(); if (viaFast) fastLastUse = now()
            if (userFacing(m.kind)) warmUntil = maxOf(warmUntil, now() + cfg.warmHoldMs)
            if (m.wantsAck && seen.contains(m.id)) { queue[0].addLast(out(Message.ack(m.id))); cv.signalAll(); return }
            if (m.wantsAck) { seen.addLast(m.id); while (seen.size > 1024) seen.removeFirst() }
            when (m.kind) {
                Kind.ACK, Kind.ERROR -> runCatching {
                    val c = Cur(m.body); val id = c.u32()
                    val res = if (m.kind == Kind.ACK) null else String(c.rest())
                    if (res != null) log("peer refused message $id: $res")
                    unacked.removeAll { it.m.id == id }
                    outcomes.addLast(id to res); while (outcomes.size > 256) outcomes.removeFirst()
                }
                Kind.FILE_OFFER -> runCatching { FileOffer.decode(m.body) }.onSuccess { o ->
                    val key = o.id.hex()
                    if (doneRecv.contains(key)) queue[1].addLast(out(Message(Kind.FILE_DONE, o.id + byteArrayOf(0))))
                    else {
                        val j = recvs[key] ?: runCatching { openRecv(o) }.getOrNull()?.also { recvs[key] = it }
                        if (j == null) queue[1].addLast(out(Message(Kind.FILE_DONE, o.id + byteArrayOf(2))))
                        else {
                            log("file offer ${o.name} (${o.size} bytes), resuming at ${j.received}")
                            if (o.size == 0L) { recvs.remove(key); finished = complete(j) }
                            else queue[1].addLast(out(Message(Kind.FILE_ACCEPT, o.id + be64(j.received))))
                        }
                    }
                }
                Kind.FILE_ACCEPT -> runCatching {
                    val c = Cur(m.body); val id = c.take(16); val off = c.u64()
                    sends.find { it.offer.id.contentEquals(id) && it.done == null }?.let { it.next = minOf(off, it.offer.size); it.accepted = true }
                }
                Kind.FILE_CHUNK -> runCatching {
                    val c = Cur(m.body); val id = c.take(16); val off = c.u64(); val data = c.rest()
                    val key = id.hex()
                    recvs[key]?.let { j ->
                        if (off == j.received && j.received + data.size <= j.offer.size) {
                            j.raf.seek(off); j.raf.write(data); j.received += data.size
                            if (j.received == j.offer.size) { recvs.remove(key); finished = complete(j) }
                        } else if (off > j.received) {
                            val body = id + be64(j.received)
                            if (queue[1].none { it.m.kind == Kind.FILE_ACCEPT && it.m.body.contentEquals(body) }) {
                                log("file resync: chunk at $off, have ${j.received}")
                                queue[1].addLast(out(Message(Kind.FILE_ACCEPT, body)))
                            }
                        }
                    }
                }
                Kind.FILE_DONE -> runCatching {
                    val c = Cur(m.body); val id = c.take(16); val st = c.u8()
                    sends.find { it.offer.id.contentEquals(id) && it.done == null }?.let { j ->
                        when (st) {
                            0 -> { j.done = ""; j.ok = true; j.finished = now() }
                            1 -> { j.next = 0; j.accepted = false; log("file hash mismatch on the peer: sending again from 0"); queue[1].addLast(out(Message(Kind.FILE_OFFER, j.offer.encode()))) }
                            else -> j.done = "peer refused the file"
                        }
                    }
                }
                Kind.FILE_CANCEL -> if (m.body.size >= 16) recvs.remove(m.body.copyOf(16).hex())?.let { runCatching { it.raf.close() }; it.part.delete() }
                Kind.FAST_REQ -> if (cfg.fastRole == FastRole.SERVER && (fastState == FastState.IDLE || fastState == FastState.FAILED)) {
                    fastState = FastState.NEGOTIATING; fastUntil = now() + cfg.fastWaitMs; openServer = s.kFast
                } else if (cfg.fastRole != FastRole.SERVER) queue[1].addLast(out(Message(Kind.FAST_FAIL, "no fast path here".toByteArray())))
                Kind.FAST_OFFER -> if (cfg.fastRole == FastRole.CLIENT) {
                    runCatching { FastOffer.decode(m.body) }.onSuccess { offer ->
                        fastState = FastState.NEGOTIATING; fastUntil = now() + cfg.fastWaitMs
                        val k = s.kFast
                        thread(isDaemon = true) { fastResult(g, runCatching { FastPath.connect(offer, k, 800) }) }
                    }
                } else queue[1].addLast(out(Message(Kind.FAST_FAIL, "no fast client here".toByteArray())))
                Kind.FAST_FAIL -> { log("peer: fast path failed (${String(m.body)})"); fastState = FastState.FAILED }
                Kind.PING -> {}
                else -> deliver = true
            }
            if (m.wantsAck && !deliver) queue[0].addLast(out(Message.ack(m.id)))
            cv.signalAll()
        }
        openServer?.let { openFastServer(g, it) }
        finished?.let { (f, o) -> log("file received: ${f.path} (${o.size} bytes)"); handler.onFile(this, f, o) }
        if (deliver) {
            val r = runCatching { handler.onMessage(this, m) }.getOrElse { it.message ?: "handler failed" }
            if (m.wantsAck) lock.withLock {
                queue[0].addLast(out(if (r == null) Message.ack(m.id) else Message(Kind.ERROR, be32(m.id) + r.toByteArray())))
                cv.signalAll()
            }
        }
    }

    private fun partFile(o: FileOffer) = File(File(cfg.inbox, ".aura-partial"), "${o.id.hex()}.part")

    private fun openRecv(o: FileOffer): RecvJob {
        val part = partFile(o); part.parentFile?.mkdirs()
        val raf = RandomAccessFile(part, "rw")
        var len = raf.length()
        if (len > o.size) { raf.setLength(0); len = 0 }
        return RecvJob(o, part, len, raf)
    }

    private fun complete(j: RecvJob): Pair<File, FileOffer>? {
        runCatching { j.raf.fd.sync(); j.raf.close() }
        val sha = MessageDigest.getInstance("SHA-256").let { d -> j.part.inputStream().use { i -> val b = ByteArray(1 shl 16); while (true) { val n = i.read(b); if (n < 0) break; d.update(b, 0, n) } }; d.digest() }
        if (!sha.contentEquals(j.offer.sha256)) {
            j.part.delete(); log("file ${j.offer.name} failed: sha256 mismatch")
            queue[1].addLast(out(Message(Kind.FILE_DONE, j.offer.id + byteArrayOf(1)))); return null
        }
        val dest = unique(cfg.inbox, sanitize(j.offer.name))
        if (!j.part.renameTo(dest)) { queue[1].addLast(out(Message(Kind.FILE_DONE, j.offer.id + byteArrayOf(2)))); return null }
        doneRecv.addLast(j.offer.id.hex()); while (doneRecv.size > 64) doneRecv.removeFirst()
        queue[1].addLast(out(Message(Kind.FILE_DONE, j.offer.id + byteArrayOf(0))))
        return dest to j.offer
    }

    // ------------------------------------------------------------ fast path

    private fun openFastServer(g: Long, k: ByteArray) {
        val addrs = cfg.fastAddrs()
        if (addrs.isEmpty()) { fastResult(g, Result.failure(AuraException("no LAN address (Wi-Fi off?)"))); return }
        val l = runCatching { FastPath.Listener(addrs) }.getOrElse { fastResult(g, Result.failure(it)); return }
        log("fast path offered on port ${l.offer.port} (${addrs.size} address(es))")
        lock.withLock { queue[1].addLast(out(Message(Kind.FAST_OFFER, l.offer.encode()))); cv.signalAll() }
        thread(isDaemon = true) { fastResult(g, runCatching { l.accept(k) }) }
    }

    private fun fastResult(g: Long, r: Result<FastPath.FastConn>) {
        lock.withLock {
            if (session?.gen != g) { r.getOrNull()?.socket?.let { runCatching { it.close() } }; return }
            val c = r.getOrNull()
            if (c != null) {
                fast = Pipe(java.io.BufferedOutputStream(c.socket.getOutputStream(), 1 shl 16), c.sealer) to { runCatching { c.socket.close() }; Unit }
                fastState = FastState.UP; fastLastUse = now()
                log("fast path up (${c.socket.remoteSocketAddress})")
                thread(isDaemon = true) { reader(c.socket.getInputStream(), c.opener, g, true) }
            } else {
                fastState = FastState.FAILED
                if (cfg.fastRole == FastRole.CLIENT) queue[1].addLast(out(Message(Kind.FAST_FAIL, (r.exceptionOrNull()?.message ?: "?").toByteArray())))
                log("fast path not available: ${r.exceptionOrNull()?.message} (staying on Bluetooth)")
            }
            cv.signalAll()
        }
    }

    private fun wantsFast(j: SendJob, peerCaps: Int): Boolean {
        val ok = when (cfg.fastRole) {
            FastRole.CLIENT -> peerCaps and Caps.FAST_SERVER != 0
            FastRole.SERVER -> peerCaps and Caps.FAST_CLIENT != 0
            FastRole.NONE -> false
        }
        return ok && (j.preferFast ?: (j.offer.size >= cfg.fastMinSize))
    }

    // ------------------------------------------------------------ driver

    private sealed class Job {
        class Write(val pipe: Pipe, val m: Message, val gen: Long, val fast: Boolean) : Job()
        object Connect : Job()
        class Teardown(val gen: Long, val why: String) : Job()
        class OpenFast(val gen: Long, val k: ByteArray) : Job()
        object CloseFast : Job()
        object Exit : Job()
    }

    /** Under the lock. Returns null after waiting (caller loops). */
    private fun nextJob(): Job? {
        val t = now()
        if (stopped) return Job.Exit
        for (q in queue) { val b = q.size; q.removeAll { it.deadline <= t }; if (q.size != b) log("${b - q.size} message(s) expired unsent") }
        val hasWork = queue.any { it.isNotEmpty() } || unacked.isNotEmpty() || sends.any { it.done == null }
        val s = session
        if (s == null) {
            if (!hasWork || connector == null) { cv.await(); return null }
            if (nextTry > t) { cv.await(nextTry - t, TimeUnit.MILLISECONDS); return null }
            return Job.Connect
        }
        unacked.filter { it.sentAt > 0 }.minOfOrNull { it.sentAt }?.let { if (t - it > cfg.ackTimeoutMs) return Job.Teardown(s.gen, "no ACK in time") }
        for (p in 0..1) queue[p].pollFirst()?.let { o ->
            if (o.m.wantsAck) { o.sentAt = t; unacked += o }
            return Job.Write(s.pipe, o.m, s.gen, false)
        }
        if (fastState == FastState.NEGOTIATING && t > fastUntil) { fastState = FastState.FAILED; log("fast path negotiation timed out") }
        var waitFast = -1L
        sends.firstOrNull { it.accepted && it.done == null && it.remaining > 0 }?.let { j ->
            val want = wantsFast(j, s.peer.caps)
            val useFast = want && fastState == FastState.UP
            when {
                want && fastState == FastState.IDLE -> {
                    fastState = FastState.NEGOTIATING; fastUntil = t + cfg.fastWaitMs
                    if (cfg.fastRole == FastRole.CLIENT) queue[1].addLast(out(Message(Kind.FAST_REQ, ByteArray(0)))) else return Job.OpenFast(s.gen, s.kFast)
                    return@let
                }
                want && fastState == FastState.NEGOTIATING -> waitFast = fastUntil - t
                else -> {
                    val max = minOf(if (useFast) cfg.chunkFast else cfg.chunkBt, Wire.MAX_RECORD - 64)
                    val r = runCatching { j.nextChunk(max) }
                    r.exceptionOrNull()?.let { j.done = "read ${j.file}: ${it.message}" }
                    r.getOrNull()?.let { (off, data) ->
                        if (useFast) j.bytesFast += data.size else j.bytesBt += data.size
                        val m = Message(Kind.FILE_CHUNK, j.offer.id + be64(off) + data)
                        val fp = fast
                        if (useFast && fp != null) { fastLastUse = t; return Job.Write(fp.first, m, s.gen, true) }
                        return Job.Write(s.pipe, m, s.gen, false)
                    }
                }
            }
        }
        if (queue[1].isNotEmpty()) return null
        val anySend = sends.any { it.done == null }
        if (fastState == FastState.UP && !anySend && recvs.isEmpty() && t - fastLastUse > 5_000) return Job.CloseFast
        val receiving = recvs.isNotEmpty() && t - lastActivity < 60_000
        val busy = unacked.isNotEmpty() || anySend || receiving
        val idleAt = maxOf(lastActivity + cfg.idleCloseMs, warmUntil)
        if (!busy && t >= idleAt) return Job.Teardown(s.gen, "idle")
        var wake = if (busy) 1_000 else idleAt - t
        if (waitFast >= 0) wake = minOf(wake, waitFast + 10)
        if (fastState == FastState.UP) wake = minOf(wake, 5_000)
        cv.await(maxOf(wake, 1), TimeUnit.MILLISECONDS)
        return null
    }

    private fun driver() {
        while (true) {
            val job = lock.withLock { nextJob() } ?: continue
            when (job) {
                is Job.Exit -> return
                is Job.Teardown -> teardown(job.gen, job.why)
                is Job.OpenFast -> openFastServer(job.gen, job.k)
                is Job.CloseFast -> lock.withLock { fast?.second?.let { runCatching { it() } }; fast = null; fastState = FastState.IDLE }.also { log("fast path idle: closed") }
                is Job.Write -> {
                    val err = runCatching {
                        synchronized(job.pipe) { Wire.write(job.pipe.out, job.pipe.sealer.seal(job.m.encode())) }
                    }.exceptionOrNull()
                    when {
                        err == null -> lock.withLock { lastActivity = now() }
                        job.fast -> { lock.withLock { fast?.second?.let { runCatching { it() } }; fast = null; fastState = FastState.FAILED }; log("fast path write failed (${err.message}); continuing over Bluetooth") }
                        else -> teardown(job.gen, "write: ${err.message}")
                    }
                }
                is Job.Connect -> {
                    val e = runCatching { connectOnce() }.exceptionOrNull()
                    if (e != null) {
                        val b = lock.withLock { val b = backoff; nextTry = now() + b; backoff = minOf(b * 2, cfg.backoffMaxMs); b }
                        log("connect failed: ${e.message}; retry in $b ms")
                    }
                }
            }
        }
    }

    private fun connectOnce() {
        val c = connector ?: throw AuraException("no connector")
        val t0 = now()
        val conn = c()
        val e = Ephemeral.generate()
        val me = Hello.create(cfg.myRole, cfg.pcId, e, cfg.caps, cfg.name)
        val done = AtomicBoolean(false)
        watchdog(done, conn.close, cfg.handshakeTimeoutMs)
        val est = try {
            Handshake.initiate(conn.input, conn.output, me, e, cfg.cred)
        } catch (x: Exception) { conn.close(); throw x } finally { done.set(true) }
        log("connected + authenticated in ${now() - t0} ms")
        install(conn, est)
    }

    companion object {
        fun watchdog(done: AtomicBoolean, close: () -> Unit, ms: Long) {
            thread(isDaemon = true) { Thread.sleep(ms); if (!done.get()) runCatching { close() } }
        }

        /** Listener side: responder handshake, then attach to the link `pick` returns. */
        fun respondAndAttach(conn: Conn, timeoutMs: Long, pick: (Hello) -> AuraLink?): AuraLink {
            val done = AtomicBoolean(false)
            watchdog(done, conn.close, timeoutMs)
            var chosen: AuraLink? = null
            try {
                val est = Handshake.respond(conn.input, conn.output) { h ->
                    val l = pick(h) ?: throw AuraException("no link for a ${Role.name(h.role)} here")
                    chosen = l; l.responderParts()
                }
                done.set(true)
                chosen!!.accept(conn, est)
                return chosen!!
            } catch (x: Exception) { done.set(true); conn.close(); throw x }
        }

        fun sanitize(name: String): String {
            val base = name.substringAfterLast('/').substringAfterLast('\\')
            val s = base.map { if (it.isISOControl() || it in "<>:\"/\\|?*") '_' else it }.joinToString("").trim().trimStart('.')
            return if (s.isEmpty()) "file" else s.take(200)
        }

        fun unique(dir: File, name: String): File {
            dir.mkdirs()
            var f = File(dir, name)
            if (!f.exists()) return f
            val dot = name.lastIndexOf('.').takeIf { it > 0 }
            val stem = dot?.let { name.substring(0, it) } ?: name
            val ext = dot?.let { name.substring(it) } ?: ""
            var i = 1
            while (f.exists()) f = File(dir, "$stem (${i++})$ext")
            return f
        }

        fun mime(name: String) = when (name.substringAfterLast('.', "").lowercase()) {
            "jpg", "jpeg" -> "image/jpeg"; "png" -> "image/png"; "gif" -> "image/gif"; "webp" -> "image/webp"
            "mp4" -> "video/mp4"; "mp3" -> "audio/mpeg"; "pdf" -> "application/pdf"; "txt" -> "text/plain"; "zip" -> "application/zip"
            else -> "application/octet-stream"
        }
    }
}
