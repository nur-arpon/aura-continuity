package com.spacez.unlock_stage1.wear

import android.Manifest
import android.annotation.SuppressLint
import android.bluetooth.BluetoothManager
import android.bluetooth.BluetoothServerSocket
import android.content.Context
import android.content.pm.PackageManager
import android.util.Log
import com.spacez.aura.link.AuraLink
import com.spacez.aura.link.Caps
import com.spacez.aura.link.Conn
import com.spacez.aura.link.Credential
import com.spacez.aura.link.Kind
import com.spacez.aura.link.LinkConfig
import com.spacez.aura.link.LinkHandler
import com.spacez.aura.link.MediaCmd
import com.spacez.aura.link.MediaState
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import com.spacez.aura.link.Message
import com.spacez.aura.link.WakeOnLan
import com.spacez.unlock_stage1.watchlink.WatchCommand
import org.json.JSONArray
import org.json.JSONObject
import com.spacez.aura.link.Role
import java.io.File
import java.util.UUID
import kotlin.concurrent.thread

/**
 * Aura Link, watch end (item 11; BUILD-ONLY, not yet run on the Galaxy Watch7).
 * Same session protocol as the phone, authenticated by the watch's existing Keystore key
 * (the one the PC pinned at pair-watch) signing the handshake transcript. Carries:
 *  watch -> PC: LOCK (the same signed CUW1 lock_pc frame), UNLOCK_REQUEST (the same signed
 *               unlock_pc push frame); PC -> watch: RING, MEDIA_STATE, PC_STATE.
 * OFF by default ("aura_on" adb hook / a later settings switch): while off, nothing here runs and
 * the existing direct RFCOMM paths (7c1e0030 / 7c1e0031) are used exactly as before. While on, an
 * extra blocked accept() on 7c1e0040 (no CPU) and an outgoing connection only when something
 * is sent; idle sessions close after 30 s.
 */
object WatchAura {
    val AURA_UUID: UUID = UUID.fromString("7c1e0040-5a1f-4e6b-9d2a-3c0b1a2f4e01")
    @Volatile var link: AuraLink? = null; private set
    @Volatile var lastMedia: MediaState? = null; private set

    /** What the PC last said is playing (item 15). Compose collects it; it changes only when the PC pushes. */
    class Media(val state: MediaState? = null, val art: ByteArray? = null, val at: Long = 0)
    private val _media = MutableStateFlow(Media())
    val media: StateFlow<Media> get() = _media

    /** One media session on the phone (the PC relays the list, art stripped). */
    class PhoneSession(val id: String, val app: String, val title: String, val artist: String, val playing: Boolean, val posMs: Long, val durMs: Long)
    class PhoneMedia(val sessions: List<PhoneSession> = emptyList(), val at: Long = 0)
    private val _phone = MutableStateFlow(PhoneMedia())
    val phone: StateFlow<PhoneMedia> get() = _phone

    /** PHONE_MEDIA body -> sessions (pure; unit-testable). */
    fun parsePhoneMedia(body: ByteArray): List<PhoneSession> = runCatching {
        val a = JSONObject(String(body, Charsets.UTF_8)).optJSONArray("sessions") ?: return emptyList()
        (0 until a.length()).map { a.getJSONObject(it) }.filter { it.optString("id").isNotEmpty() }.map {
            PhoneSession(it.getString("id"), it.optString("app", "Phone"), it.optString("title"), it.optString("artist"), it.optBoolean("playing"), it.optLong("pos"), it.optLong("dur"))
        }
    }.getOrDefault(emptyList())

    /** Ask the PC (which relays to the phone) for the phone's sessions; the answer is the PHONE_MEDIA push. */
    fun askPhoneList(ctx: Context): String? = send(ctx, Message.json(Kind.PHONE_MEDIA_CMD, """{"cmd":"list"}"""), 8_000)
    /** cmd = toggle | next | prev for one phone session. */
    fun phoneCmd(ctx: Context, id: String, cmd: String): String? = send(ctx, Message.json(Kind.PHONE_MEDIA_CMD, JSONObject().put("id", id).put("cmd", cmd).toString()), 8_000)

    /** Screenshot / unit tests only. */
    fun injectPhoneForTest(s: List<PhoneSession>) { _phone.value = PhoneMedia(s, System.currentTimeMillis()) }
    /** Screenshot / unit tests only. */
    fun injectMediaForTest(s: MediaState) { _media.value = Media(s, null, System.currentTimeMillis()) }

    /** Item 15 (ring laptop, music, slides): on by default, independent of the lock / unlock switch below. */
    fun extrasOn(c: Context) = p(c).getBoolean("aura_extras", true)
    fun setExtras(c: Context, on: Boolean) { p(c).edit().putBoolean("aura_extras", on).apply(); if (on) start(c) else if (!enabled(c)) stop() }
    private fun active(c: Context) = enabled(c) || extrasOn(c)

    /** null = the PC acked it; else the reason in plain words. Opens the session if needed. */
    fun send(ctx: Context, m: Message, waitMs: Long = 4_000): String? {
        if (PcLinkPrefs(ctx).pcAddress == null) return "Pair with PC first"
        if (!extrasOn(ctx)) return "Remote extras are off in Settings"
        start(ctx)
        val l = link ?: return "Pair with PC first"
        val err = l.waitAcked(l.send(m), waitMs) ?: return null
        return if (err.contains("timed out", true) || err.isBlank()) "PC not reachable" else err
    }
    fun ring(ctx: Context, on: Boolean) = send(ctx, Message.ring(on), 6_000)

    /** Restart / shut down / sleep: PC_POWER over the watch's own Aura link; Restart falls back to the phone relaying it. */
    suspend fun power(ctx: Context, action: String): String? {
        val body = JSONObject().put("action", action).put("from", "Watch").toString().toByteArray()
        val err = send(ctx, Message(Kind.PC_POWER, body), 8_000) ?: return null
        if (action == "restart" && PhoneLink.command(ctx, WatchCommand.RESTART_PC) > 0) return null
        return err
    }

    // ---- Wake PC: the watch sends the magic packet itself when it knows the PC and is on a remembered Wi-Fi; the
    // phone is always asked too (it also reaches the PC through the tailnet relay when you are away).
    private fun w(c: Context) = c.getSharedPreferences("cu_wake", Context.MODE_PRIVATE)

    /** LAPTOP_INFO "wake" block from the PC (latest MACs/relays win; networks accumulate by SSID). */
    fun saveWakeInfo(c: Context, json: String) {
        val o = runCatching { JSONObject(json).optJSONObject("wake") }.getOrNull() ?: return
        val e = w(c).edit()
        o.optJSONArray("macs")?.let { e.putString("macs", it.toString()) }
        o.optJSONArray("relays")?.let { e.putString("relays", it.toString()) }
        o.optJSONObject("network")?.let { n ->
            val ssid = n.optString("ssid"); val bc = n.optString("broadcast")
            if (ssid.isNotBlank() && bc.isNotBlank()) {
                val nets = runCatching { JSONObject(w(c).getString("nets", "{}")!!) }.getOrDefault(JSONObject())
                nets.put(ssid, JSONObject().put("ip", n.optString("ip")).put("broadcast", bc)); e.putString("nets", nets.toString())
            }
        }
        e.apply()
    }

    private fun list(j: String?) = runCatching { JSONArray(j ?: "[]").let { a -> (0 until a.length()).map { a.getString(it) } } }.getOrDefault(emptyList())

    /** True when packets went out from the watch itself. */
    @Suppress("DEPRECATION")
    private fun wakeDirect(c: Context): Boolean {
        val macs = list(w(c).getString("macs", null)).mapNotNull { WakeOnLan.parseMac(it) }
        if (macs.isEmpty()) return false
        val cm = c.getSystemService(Context.CONNECTIVITY_SERVICE) as android.net.ConnectivityManager
        val net = cm.allNetworks.firstOrNull { cm.getNetworkCapabilities(it)?.hasTransport(android.net.NetworkCapabilities.TRANSPORT_WIFI) == true } ?: return false
        val ip = cm.getLinkProperties(net)?.linkAddresses?.firstOrNull { it.address is java.net.Inet4Address }?.address?.hostAddress ?: return false
        val nets = runCatching { JSONObject(w(c).getString("nets", "{}")!!) }.getOrDefault(JSONObject())
        val known = nets.keys().asSequence().map { nets.getJSONObject(it) }.filter { WakeOnLan.sameSubnet(it.optString("ip"), ip) }.toList()
        if (known.isEmpty()) return false
        val t = known.flatMap { n -> intArrayOf(9, 7).flatMap { p -> listOf(WakeOnLan.Target(n.optString("broadcast"), p, true), WakeOnLan.Target("255.255.255.255", p, true)) } }.distinct()
        return WakeOnLan.send(macs, t, 3, 300) { s -> runCatching { net.bindSocket(s) } } > 0
    }

    /** null = something was sent; else the reason. */
    suspend fun wake(ctx: Context): String? {
        val direct = runCatching { wakeDirect(ctx) }.getOrDefault(false)
        val viaPhone = PhoneLink.command(ctx, WatchCommand.WAKE_PC) > 0
        return if (direct || viaPhone) null else "Phone not reachable"
    }
    // the link cold-starts over Bluetooth (several seconds): wait long enough before calling the PC unreachable
    fun mediaCmd(ctx: Context, cmd: Int) = send(ctx, Message.mediaCmd(cmd), 8_000)
    fun present(ctx: Context, cmd: Int) = send(ctx, Message.present(cmd), 8_000)

    /** While a remote screen is open: a PING opens (or keeps) the session, so INPUT's 2 s lifetime and the first
     *  music/slide tap do not race a several-second Bluetooth cold start (live bug 2026-10-30: touchpad did nothing). */
    fun keepWarm(ctx: Context) {
        if (InputTarget.phone(ctx)) { PhoneInput.warm(ctx); return }
        if (!extrasOn(ctx) || PcLinkPrefs(ctx).pcAddress == null) return
        start(ctx)
        link?.send(Message(Kind.PING, ByteArray(0)))
    }

    /** Remote mouse / keyboard (INPUT): fire and forget, no ack, 2 s lifetime. */
    fun input(ctx: Context, json: String) {
        if (InputTarget.phone(ctx)) { PhoneInput.send(ctx, json); return }   // item 7: "Phone" target on the touchpad / Air mouse
        if (!extrasOn(ctx) || PcLinkPrefs(ctx).pcAddress == null) return
        start(ctx)
        link?.send(Message.input(json))
    }
    @Volatile private var server: BluetoothServerSocket? = null
    @Volatile private var stopping = false

    private fun p(c: Context) = c.getSharedPreferences("cu_pclink", Context.MODE_PRIVATE)
    fun enabled(c: Context) = p(c).getBoolean("aura_link", false)
    fun setEnabled(c: Context, on: Boolean) { p(c).edit().putBoolean("aura_link", on).apply(); if (on) start(c) else stop() }

    @Synchronized
    fun start(ctx: Context) {
        val app = ctx.applicationContext
        if (!active(app)) return
        stopping = false
        val lp = PcLinkPrefs(app)
        val pcId = lp.pcId?.chunked(2)?.map { it.toInt(16).toByte() }?.toByteArray() ?: return
        if (link == null) {
            val cfg = LinkConfig(Role.WATCH, Role.PC, pcId, Credential.WatchSigner { WatchKeys.signer.sign(it) },
                File(app.filesDir, "aura"), caps = Caps.RING or Caps.MEDIA, name = "Watch")
            link = AuraLink(cfg, { connectPc(app) }, H(app))
        }
        startServer(app)
    }

    @Synchronized
    fun stop() {
        stopping = true
        runCatching { server?.close() }; server = null
        link?.stop(); link = null
    }

    /** LOCK over Aura Link only if the PC said it owns locking; false = use the legacy direct path. */
    fun trySendLock(frameWithHeader: ByteArray): Boolean {
        val l = link ?: return false
        if ((l.peer?.caps ?: 0) and Caps.LOCK_SINK == 0) return false
        return l.waitAcked(l.send(Message(Kind.LOCK, frameWithHeader)), 4_000) == null
    }

    /** UNLOCK_REQUEST over Aura Link (opens the session if needed); false = use the legacy 7c1e0031 path. */
    fun trySendUnlock(frameWithHeader: ByteArray): Boolean {
        val l = link ?: return false
        val id = l.send(Message(Kind.UNLOCK_REQUEST, frameWithHeader))
        return l.waitAcked(id, 4_000) == null
    }

    @SuppressLint("MissingPermission")
    private fun connectPc(ctx: Context): Conn {
        val addr = PcLinkPrefs(ctx).pcAddress ?: throw IllegalStateException("PC not paired")
        if (ctx.checkSelfPermission(Manifest.permission.BLUETOOTH_CONNECT) != PackageManager.PERMISSION_GRANTED) throw IllegalStateException("no Bluetooth permission")
        val ad = ctx.getSystemService(BluetoothManager::class.java)?.adapter?.takeIf { it.isEnabled } ?: throw IllegalStateException("Bluetooth off")
        val s = ad.getRemoteDevice(addr).createRfcommSocketToServiceRecord(AURA_UUID)
        s.connect()
        return Conn(s.inputStream, s.outputStream, { runCatching { s.close() } }, "rfcomm")
    }

    @SuppressLint("MissingPermission")
    private fun startServer(ctx: Context) {
        if (server != null || stopping) return
        if (ctx.checkSelfPermission(Manifest.permission.BLUETOOTH_CONNECT) != PackageManager.PERMISSION_GRANTED) return
        val ad = ctx.getSystemService(BluetoothManager::class.java)?.adapter?.takeIf { it.isEnabled } ?: return
        thread(name = "aura-accept", isDaemon = true) {
            try {
                val srv = ad.listenUsingRfcommWithServiceRecord("Aura Link", AURA_UUID)
                server = srv
                Log.i(TAG, "Aura Link listening (7c1e0040)")
                while (!stopping) {
                    val s = srv.accept() ?: break
                    thread(isDaemon = true) {
                        val c = Conn(s.inputStream, s.outputStream, { runCatching { s.close() } }, "rfcomm-in")
                        runCatching { AuraLink.respondAndAttach(c, 10_000) { h -> if (h.role == Role.PC) link else null } }
                            .onFailure { Log.w(TAG, "Aura Link refused: ${it.message}") }
                    }
                }
            } catch (t: Throwable) {
                if (!stopping) Log.w(TAG, "Aura Link listener ended: ${t.message}")
            } finally {
                server = null
            }
        }
    }

    private class H(private val ctx: Context) : LinkHandler {
        override fun log(line: String) { Log.i(TAG, "aura $line") }
        override fun onMessage(link: AuraLink, m: Message): String? {
            when (m.kind) {
                Kind.LAPTOP_INFO -> saveWakeInfo(ctx, String(m.body, Charsets.UTF_8))
                Kind.RING -> if (m.body.firstOrNull()?.toInt() == 1) { Log.i(TAG, "Aura: ring from PC"); repeat(3) { Feedback.play(ctx, Feedback.Cue.COUNTDOWN) } }
                Kind.MEDIA_STATE -> {
                    Log.i(TAG, "Aura: MEDIA_STATE ${m.body.size} bytes")
                    lastMedia = runCatching { MediaState.decode(m.body) }.getOrNull()
                    val old = _media.value.state
                    val keepArt = old != null && lastMedia != null && old.title == lastMedia!!.title && old.artist == lastMedia!!.artist
                    _media.value = Media(lastMedia, if (keepArt) _media.value.art else null, System.currentTimeMillis())
                }
                Kind.PHONE_MEDIA -> _phone.value = PhoneMedia(parsePhoneMedia(m.body), System.currentTimeMillis())
                Kind.MEDIA_ART -> _media.value = Media(_media.value.state, m.body.takeIf { it.isNotEmpty() }, _media.value.at)
                Kind.PC_STATE -> Log.i(TAG, "Aura: PC ${if (m.body.firstOrNull()?.toInt() == 1) "locked" else "unlocked"}")
                Kind.TEXT -> if (String(m.body).startsWith("ping:")) link.send(Message.text("pong:" + String(m.body).removePrefix("ping:")))
            }
            return null
        }
    }
}
