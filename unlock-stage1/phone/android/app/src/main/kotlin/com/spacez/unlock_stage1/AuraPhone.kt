package com.spacez.unlock_stage1

import android.Manifest
import android.annotation.SuppressLint
import android.bluetooth.BluetoothManager
import android.bluetooth.BluetoothServerSocket
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.util.Log
import com.spacez.aura.link.AuraCrypto
import com.spacez.aura.link.AuraLink
import com.spacez.aura.link.Caps
import com.spacez.aura.link.Conn
import com.spacez.aura.link.Credential
import com.spacez.aura.link.FastRole
import com.spacez.aura.link.FileOffer
import com.spacez.aura.link.Kind
import com.spacez.aura.link.LinkConfig
import com.spacez.aura.link.LinkHandler
import com.spacez.aura.link.Message
import com.spacez.aura.link.Role
import com.spacez.aura.link.raceConnect
import com.spacez.unlock_stage1.away.AwayPhone
import com.spacez.unlock_stage1.away.Tailnet
import com.spacez.unlock_stage1.handoff.HandoffPhone
import java.io.File
import java.util.UUID
import kotlin.concurrent.thread

/**
 * Aura Link, phone end (item 11). Lives inside the existing CDM-started [UnlockService]: started
 * and stopped with it, no service of its own, no wakelock, no timer.
 *  - RFCOMM server 7c1e0040: a blocked accept() (same idle cost as the unlock listener) so the
 *    PC can open the session when IT has something to send;
 *  - client: connects to the PC's 7c1e0040 only when the phone has something queued;
 *  - LAN fast path: this phone is the TCP listener (only while a big transfer negotiates).
 * Auth: the PSK is HMAC(Keystore cu2-0, "aura-link/v1/psk") - the existing pairing, no new
 * secret, works while the phone is locked. Received files: Android/data/<pkg>/files/Aura.
 */
object AuraPhone {
    private const val TAG = "AuraLink"
    val AURA_UUID: UUID = UUID.fromString("7c1e0040-5a1f-4e6b-9d2a-3c0b1a2f4e01")
    /** The primary laptop's link (the one paired through the unlock flow). Lock events, the listener and the service use this. */
    @Volatile var primary: AuraLink? = null; private set
    /**
     * "The laptop" for user actions (Continue, clipboard, share, chat head): the picked laptop among the primary
     * and the secondary companions ([Laptops.target]). With no secondary laptop this is exactly [primary].
     */
    val link: AuraLink? get() = appCtx?.let { Laptops.target(it, primary) } ?: primary
    @Volatile private var appCtx: Context? = null
    @Volatile private var server: BluetoothServerSocket? = null
    @Volatile private var stopping = false
    private var backoffMs = 1000L
    private val main = Handler(Looper.getMainLooper())

    private fun p(c: Context) = c.getSharedPreferences("unlock", 0)
    /** adb-only test switch for `aura link-test` (lets the PC ask for a test file). Expires by itself. */
    fun testOn(c: Context) = System.currentTimeMillis() < p(c).getLong("aura_test_until", 0)
    fun setTest(c: Context, on: Boolean) = p(c).edit().putLong("aura_test_until", if (on) System.currentTimeMillis() + 30 * 60_000L else 0).apply()

    fun inbox(c: Context) = File(c.getExternalFilesDir(null) ?: c.filesDir, "Aura")

    /**
     * The link itself needs no radio (item 17: Bluetooth may be off on a train; the tailnet still
     * works). Created on demand by the service, the app, the widget or a tile.
     */
    @Synchronized
    fun ensureLink(ctx: Context): AuraLink? {
        primary?.let { return it }
        val app = ctx.applicationContext
        appCtx = app
        val pcId = UnlockCrypto.pcId(app) ?: return null.also { UnlockState.log("Aura Link: not paired") }
        val k0 = UnlockCrypto.mac("0") ?: return null.also { UnlockState.log("Aura Link: no level-0 key") }
        val cfg = LinkConfig(Role.PHONE, Role.PC, pcId, Credential.Psk(AuraCrypto.pskFromMac(k0)), inbox(app),
            caps = Caps.FILES or Caps.FAST_SERVER or Caps.MEDIA or Caps.RING or Caps.TAILNET, name = Build.MODEL, fastRole = FastRole.SERVER)
        // Bluetooth first; the owner's tailnet when Bluetooth has not connected within 1.5 s (or is off),
        // and only right after a user action (AwayPhone.gate). Same session, same encryption either way.
        val l = AuraLink(cfg, { raceConnect({ connectPc(app) }, { AwayPhone.tailnetConn(app) }, 1_500) }, PhoneHandler(app))
        primary = l
        UnlockState.log("Aura Link ready (PSK from the pairing, inbox ${cfg.inbox})")
        return l
    }

    /** The primary slot was re-paired with a new key: drop the old link (its pc id / PSK are stale) and dial with the new one. */
    @Synchronized
    fun repair(ctx: Context) {
        primary?.stop(); primary = null
        ensureLink(ctx)?.send(Message.text("ping:pair"))
    }

    @Synchronized
    fun start(ctx: Context) {
        val app = ctx.applicationContext
        stopping = false
        Laptops.ensureAll(app)
        ensureLink(app) ?: return
        startServer(app)
        // Away mode: listen on the tailnet while Tailscale is up; one fetch when it comes up
        Tailnet.watch(app, { primary }) { AwayPhone.autoFetch(app, "tailnet came up") }
        // a "turn Tailscale off after the stream" watch that was running when the process died carries on
        com.spacez.unlock_stage1.away.TailnetAutoOff.resume(app)
    }

    /** Bluetooth went off: only the RFCOMM listener stops; the link (and the tailnet) stay. */
    @Synchronized
    fun stopServer() {
        runCatching { server?.close() }; server = null
    }

    @Synchronized
    fun stop() {
        stopping = true
        runCatching { server?.close() }; server = null
        primary?.stop(); primary = null
    }

    fun stopAll(ctx: Context) {
        stop()
        Tailnet.unwatch(ctx.applicationContext)
    }

    fun status() = primary?.status() ?: "off"

    /** Lock event over Aura Link, only when the PC said it owns locking (LOCK_SINK). Else false = use the legacy path. */
    fun trySendLock(frame: ByteArray): Boolean {
        val l = primary ?: return false
        val peer = l.peer ?: return false
        if (peer.caps and Caps.LOCK_SINK == 0) return false
        val id = l.send(Message(Kind.LOCK, frame))
        return l.waitAcked(id, 3_000) == null
    }

    @SuppressLint("MissingPermission")
    private fun connectPc(ctx: Context): Conn {
        if (!PermCompat.btConnect(ctx)) throw IllegalStateException("no Bluetooth permission")
        val ad = ctx.getSystemService(BluetoothManager::class.java)?.adapter?.takeIf { it.isEnabled } ?: throw IllegalStateException("Bluetooth off")
        val known = PcCompanion.pcAddress(ctx) ?: PcCompanion.associations(ctx).firstNotNullOfOrNull { it.deviceMacAddress?.toString() }
        var last: Exception? = null
        if (known != null) try {
            val s = ad.getRemoteDevice(known).createRfcommSocketToServiceRecord(AURA_UUID)
            s.connect()
            return Conn(s.inputStream, s.outputStream, { runCatching { s.close() } }, "rfcomm")
        } catch (e: Exception) { last = e } // stale/wrong address: fall back to the paired computers below
        // paired by typing the code (no QR, so no PC address): try the paired computers; only a PC running Aura offers
        // the Aura Link service, so the first that connects is the one - remember it
        val pcs = ad.bondedDevices.filter { it.bluetoothClass?.majorDeviceClass == android.bluetooth.BluetoothClass.Device.Major.COMPUTER && it.address.uppercase() != known?.uppercase() }
        if (pcs.isEmpty()) throw last ?: IllegalStateException("PC Bluetooth address not known yet (pair the PC in Bluetooth settings)")
        for (d in pcs) {
            val s = runCatching { d.createRfcommSocketToServiceRecord(AURA_UUID) }.getOrNull() ?: continue
            try {
                s.connect()
                PcCompanion.rememberPc(ctx, d.address)
                return Conn(s.inputStream, s.outputStream, { runCatching { s.close() } }, "rfcomm")
            } catch (e: Exception) { last = e; runCatching { s.close() } }
        }
        throw IllegalStateException("no paired PC answered (${last?.message})")
    }

    @SuppressLint("MissingPermission")
    private fun startServer(ctx: Context) {
        if (server != null || stopping) return
        if (ctx.getSystemService(BluetoothManager::class.java)?.adapter?.isEnabled != true) return
        if (!PermCompat.btConnect(ctx)) return
        val ad = ctx.getSystemService(BluetoothManager::class.java)?.adapter?.takeIf { it.isEnabled } ?: return
        thread(name = "aura-accept", isDaemon = true) {
            var ok = false
            try {
                val srv = ad.listenUsingRfcommWithServiceRecord("Aura Link", AURA_UUID)
                server = srv; ok = true; backoffMs = 1000
                UnlockState.log("Aura Link listening (RFCOMM 7c1e0040)")
                while (true) {
                    val s = srv.accept() ?: break
                    val peer = runCatching { s.remoteDevice.address }.getOrNull()
                    thread(isDaemon = true) {
                        val conn = Conn(s.inputStream, s.outputStream, { runCatching { s.close() } }, "rfcomm-in")
                        runCatching { AuraLink.respondAndAttach(conn, 10_000) { h -> if (h.role == Role.PC) (if (primary != null && h.pcId.contentEquals(primary!!.cfg.pcId)) primary else Laptops.pickIncoming(h.pcId)) else null } }
                            .onFailure { UnlockState.log("Aura Link from $peer refused: ${it.message}") }
                    }
                }
            } catch (t: Throwable) {
                if (!stopping) UnlockState.log("Aura Link listener ended: ${t.message}")
            } finally {
                server = null
                if (!stopping && ad.isEnabled) {
                    val d = if (ok) 1000 else backoffMs
                    backoffMs = minOf(backoffMs * 2, 60_000)
                    main.postDelayed({ startServer(ctx) }, d)
                }
            }
        }
    }

    private class PhoneHandler(private val ctx: Context) : LinkHandler {
        override fun log(line: String) { Log.i(TAG, line); if (!line.contains("FILE_CHUNK")) UnlockState.log("aura $line") }

        override fun onSession(link: AuraLink, up: Boolean) {
            UnlockState.log("Aura Link to PC ${if (up) "UP" else "down"}")
            if (up) {
                AwayPhone.hello(link)
                Laptops.sessionUp(UnlockCrypto.pcId(ctx)?.toHex() ?: "")
                com.spacez.unlock_stage1.handoff.PhoneHub.pushNow() // Now playing everywhere: a new PC session knows nothing yet
                PcState.sessionUp(ctx, link.status().substringAfter("session=").substringBefore('#').substringBefore(' '))
                link.peer?.name?.let { PcState.setName(ctx, it) }
                com.spacez.unlock_stage1.buds.BudsHandoff.start(ctx); com.spacez.unlock_stage1.buds.BudsHandoff.report(ctx, true)
            }
        }

        override fun onFile(link: AuraLink, file: File, offer: FileOffer) {
            if (testOn(ctx) && offer.name.startsWith("aura-test-")) { UnlockState.log("Aura file received: ${file.name} (${offer.size} bytes)"); return }
            if (com.spacez.unlock_stage1.update.AppUpdate.isUpdateName(offer.name)) { com.spacez.unlock_stage1.update.AppUpdate.onReceived(ctx, file, offer); return }
            HandoffPhone.onFile(ctx, file, offer)
        }

        override fun onMessage(link: AuraLink, m: Message): String? {
            when (m.kind) {
                Kind.TEXT -> {
                    val t = String(m.body, Charsets.UTF_8)
                    if (t.startsWith("ping:") || t.startsWith("aura-test:")) UnlockState.log("Aura text from PC: ${t.take(80)}")
                    else UnlockState.log("Aura text from PC (${t.length} chars)")
                    when {
                        t.startsWith("ping:") -> link.send(Message.text("pong:" + t.removePrefix("ping:")))
                        // the PC asks what plays here (tray "Continue from phone", Brave focus): re-send the state
                        // the PC could not find the video the "Continue on PC" button sent by identity
                        t.startsWith("aura:toast ") -> HandoffPhone.toastOnPc(ctx, t.removePrefix("aura:toast "))
                        t.startsWith("aura:buds ") -> com.spacez.unlock_stage1.buds.BudsHandoff.onPcMessage(ctx, t.removePrefix("aura:buds "))
                        t == "aura:state?" -> com.spacez.unlock_stage1.handoff.MediaWatch.reportNow()
                        t.startsWith("aura-test:send ") -> return testSend(link, t)
                    }
                }
                Kind.PC_STATE -> {
                    val locked = m.body.firstOrNull()?.toInt() == 1
                    UnlockState.log("Aura: PC is ${if (locked) "locked" else "unlocked"}")
                    PcState.setLocked(ctx, locked)
                }
                Kind.URL -> { val (u, pos) = Message.parseUrl(m.body); HandoffPhone.onUrl(ctx, u, pos) }
                Kind.CLIPBOARD -> { val (mime, data) = Message.parseClipboard(m.body); HandoffPhone.onClipboard(ctx, mime, data) }
                Kind.ACTIVITY -> HandoffPhone.onActivity(ctx, String(m.body, Charsets.UTF_8))
                // Aura Anywhere (item 17)
                Kind.TAILNET -> AwayPhone.onPcTailnet(ctx, String(m.body, Charsets.UTF_8))
                // remote control: where the PC's Sunshine host listens (no typing on the phone)
                Kind.LAPTOP_INFO -> String(m.body, Charsets.UTF_8).let { j -> com.spacez.unlock_stage1.extras.Moonlight.save(ctx, j); com.spacez.unlock_stage1.extras.PcWake.save(ctx, j) }
                Kind.LAST_ACTIVITY -> AwayPhone.onLastActivity(ctx, String(m.body, Charsets.UTF_8))
                Kind.UNLOCK_RESULT -> AwayPhone.onUnlockResult(ctx, String(m.body, Charsets.UTF_8))
                // item 19: the PC continued what this phone played (pause it here) / Undo (play)
                Kind.CONTINUED -> com.spacez.unlock_stage1.zerotap.ZeroTap.onContinued(ctx, String(m.body, Charsets.UTF_8))
                // item 15: the PC rings the phone; now-playing state and artwork for the Remote screen
                Kind.RING -> com.spacez.unlock_stage1.extras.PcRemote.onRingFromPc(ctx, m.body.firstOrNull()?.toInt() == 1)
                Kind.MEDIA_STATE -> { com.spacez.unlock_stage1.extras.PcRemote.onMediaState(m.body); com.spacez.unlock_stage1.extras.LaptopCard.refresh(ctx) }
                // Now playing everywhere / notification mirror: the PC controls a phone session, dismisses or replies to a mirrored notification
                Kind.PHONE_MEDIA_CMD -> com.spacez.unlock_stage1.handoff.PhoneHub.onCmd(String(m.body, Charsets.UTF_8))
                Kind.NOTIF_ACT -> com.spacez.unlock_stage1.handoff.NotifMirror.onAct(String(m.body, Charsets.UTF_8))
                Kind.LAPTOP_APPS -> com.spacez.unlock_stage1.handoff.ChatHead.onLaptopApps(ctx, String(m.body, Charsets.UTF_8))
                Kind.MEDIA_ART -> { com.spacez.unlock_stage1.extras.PcRemote.onMediaArt(m.body); com.spacez.unlock_stage1.extras.LaptopCard.refresh(ctx) }
            }
            return null
        }

        /** "aura-test:send <mb> <bt|fast>": only while the adb test switch is on. */
        private fun testSend(link: AuraLink, t: String): String? {
            if (!testOn(ctx)) return "aura test switch is off on the phone"
            val parts = t.split(' ')
            val mb = parts.getOrNull(1)?.toIntOrNull()?.coerceIn(1, 50) ?: return "bad size"
            val via = parts.getOrNull(2) ?: "bt"
            thread(isDaemon = true) {
                val f = File(ctx.cacheDir, "aura-test-phone-${mb}MB-$via.bin")
                f.outputStream().buffered().use { o -> val r = java.util.Random(42); val b = ByteArray(65536); repeat(mb * 16) { r.nextBytes(b); o.write(b) } }
                val rep = runCatching { link.waitFile(link.sendFile(f, via == "fast"), 900_000).toString() }.getOrElse { "FAILED: ${it.message}" }
                f.delete()
                UnlockState.log("Aura test file -> PC via $via: $rep")
                link.send(Message.text("aura-test:report $via $rep"))
            }
            return null
        }
    }
}
