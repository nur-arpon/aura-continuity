package com.spacez.unlock_stage1.away

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.content.Context
import com.spacez.aura.link.AuraLink
import com.spacez.aura.link.Conn
import com.spacez.aura.link.Kind
import com.spacez.aura.link.Message
import com.spacez.aura.link.RemoteUnlock
import com.spacez.unlock_stage1.AuraPhone
import com.spacez.unlock_stage1.UnlockCrypto
import com.spacez.unlock_stage1.UnlockState
import com.spacez.unlock_stage1.handoff.HandoffPhone
import com.spacez.unlock_stage1.watchlink.PcEvents
import java.security.SecureRandom
import javax.crypto.Mac
import kotlin.concurrent.thread

/**
 * Aura Anywhere (item 17), phone side. Everything rides on the SAME Aura Link session as at home
 * (PSK from the pairing, fresh ECDH keys, AES-GCM); only the pipe differs: Bluetooth first, the
 * owner's Tailscale tailnet when Bluetooth does not connect within 1.5 s.
 *
 * Battery: nothing here polls. The PC is asked for its last activity when the app / widget / tile
 * is opened or when the Tailscale VPN comes up (at most once a minute); the tailnet is dialled only
 * inside a short window after such a user action ([gate]).
 *
 * Remote unlock is deliberate only: [unlockPc] needs the Mac that a strong-biometric
 * BiometricPrompt just released for the Keystore FINGERPRINT key (RemoteUnlockActivity); nothing
 * else in the app can sign that frame.
 */
object AwayPhone {
    val gate = AwayLogic.Gate()
    private val sync = Object()
    @Volatile private var lastSeq = 0
    @Volatile private var resultSeq = 0
    @Volatile private var lastResult: AwayLogic.UnlockResult? = null
    @Volatile private var lastAutoFetch = 0L
    private const val CH = "aura_away"
    private const val N_RESULT = 4301

    private fun p(c: Context) = c.getSharedPreferences("away", 0)
    private fun now() = System.currentTimeMillis()
    fun log(s: String) = UnlockState.log("away: $s")

    // ------------------------------------------------------------------ addresses

    fun pcAddr(c: Context): Pair<String, Int>? {
        val ip = p(c).getString("pc_ip", null) ?: return null
        return ip to p(c).getInt("pc_port", AwayLogic.PORT)
    }

    fun onPcTailnet(c: Context, json: String) {
        val a = AwayLogic.parseTailnet(json) ?: return log("TAILNET from PC ignored")
        if (pcAddr(c) != a) {
            p(c).edit().putString("pc_ip", a.first).putInt("pc_port", a.second).apply()
            log("PC is on the tailnet at ${a.first}:${a.second} (learned over Aura Link)")
        }
    }

    /** At every session start: tell the PC where this phone listens on the tailnet. */
    fun hello(link: AuraLink) {
        val ip = Tailnet.ownIp() ?: return
        link.send(Message.json(Kind.TAILNET, AwayLogic.hello(ip)))
    }

    /** The tailnet leg of the connector (null = do not use the tailnet now). */
    fun tailnetConn(c: Context): Conn? {
        if (!gate.isOpen(now())) return null
        val (ip, port) = pcAddr(c) ?: return null
        if (Tailnet.ownIp() == null) return null
        return Tailnet.connect(ip, port, 3_000)
    }

    /** An explicit user action: allow the tailnet for a short while. */
    fun userAction(ms: Long = 30_000) = gate.open(now(), ms)

    fun link(c: Context): AuraLink? = AuraPhone.ensureLink(c.applicationContext)

    // ------------------------------------------------------------------ last activity

    /**
     * ONE source of truth for "what the PC is doing": the newer of the last FETCHED answer and the last item the PC PUSHED
     * (item 27). A push replaces the main item, keeping the fetched recent list / lock state, so the widget, the Home card
     * and the notification always agree.
     */
    fun last(c: Context): AwayLogic.Last? {
        val base = p(c).getString("last_json", null)?.let { AwayLogic.Last.parse(it, p(c).getLong("last_rx", 0)) }
        val pj = p(c).getString("pushed_json", null)
        val pAt = p(c).getLong("pushed_at", 0)
        if (pj != null && pAt >= p(c).getLong("last_rx", 0)) {
            val pushed = AwayLogic.Last.parse(pj, pAt)
            if (pushed != null) return if (base != null) base.copy(s = pushed.s, fresh = true) else pushed.copy(fresh = true)
        }
        return base
    }

    /** The PC pushed a "Continue on phone" item: remember it as the last item and redraw the widget. */
    fun onPushed(c: Context, json: String) {
        if (AwayLogic.Last.parse(json) == null) return
        p(c).edit().putString("pushed_json", json).putLong("pushed_at", now()).apply()
        AwayWidget.updateAll(c)
    }
    fun fetchedAt(c: Context) = p(c).getLong("last_at", 0)

    fun onLastActivity(c: Context, json: String) {
        val l = AwayLogic.Last.parse(json)
        p(c).edit().apply { if (l != null) putString("last_json", json) else remove("last_json") }.putLong("last_at", now()).putLong("last_rx", now()).apply()
        synchronized(sync) { lastSeq++; sync.notifyAll() }
        if (l != null) com.spacez.unlock_stage1.PcState.setLocked(c, l.pcLocked)
        log("PC's last activity: ${l?.let { "${it.s.app} ${com.spacez.unlock_stage1.handoff.HandoffLogic.host(it.s.url)}${if (it.fresh) " (live)" else ""}" } ?: "nothing"}")
        AwayWidget.updateAll(c)
        com.spacez.unlock_stage1.handoff.HandoffPhone.refreshMore(c)
    }

    /** Ask the PC what it was last doing. Blocking (<= ~12 s): call off the main thread. */
    fun fetch(c: Context, why: String): String {
        val l = link(c) ?: return "Not paired with a PC"
        userAction()
        val seq0 = lastSeq
        val err = l.waitAcked(l.send(Message(Kind.LAST_ACTIVITY_GET, ByteArray(0))), 10_000)
        if (err != null) { log("fetch ($why): $err"); return "PC not reachable" }
        val end = now() + 4_000
        synchronized(sync) { while (lastSeq == seq0 && now() < end) sync.wait(maxOf(1, end - now())) }
        log("fetch ($why) over ${l.status().substringAfter("session=").substringBefore(' ')}")
        return if (lastSeq != seq0) "ok" else "No answer from the PC"
    }

    /** VPN up / app opened: one quiet fetch, at most once a minute. */
    fun autoFetch(c: Context, why: String) {
        if (UnlockCrypto.pcId(c) == null || now() - lastAutoFetch < 20_000) return
        lastAutoFetch = now()
        thread(isDaemon = true) { fetch(c.applicationContext, why) }
    }

    /** Open item [index] of "More from your PC" on this phone (YouTube at its time, pages in the default browser). */
    fun openRecent(c: Context, index: Int, mode: com.spacez.unlock_stage1.handoff.LinkLogic.Mode? = null): String {
        val r = last(c)?.recent?.getOrNull(index) ?: return "That item is gone - ask your PC again"
        HandoffPhone.open(c, r.openUrl(now()), null, "PC (more from your PC)", mode)
        continued(c, r)
        return "Opening ${r.title.ifBlank { com.spacez.unlock_stage1.handoff.HandoffLogic.host(r.url) }}"
    }

    /** "Continue from laptop": a fresh answer if the last one is older than 30 s, then open at the exact second. */
    fun continueFromLaptop(c: Context): String {
        if (now() - fetchedAt(c) > 30_000) fetch(c, "continue")
        val l = last(c) ?: return "Nothing to continue from your PC"
        HandoffPhone.open(c, l.s.openUrl(now()), null, "PC (continue from laptop)")
        continued(c, l.s)
        return "Opening ${l.title()}"
    }

    /**
     * Item 19 rule "after continuing, pause the source device": tell the PC that [s] plays here now.
     * Only for something that was playing there (a paused video / a page needs nothing).
     */
    fun continued(c: Context, s: com.spacez.unlock_stage1.handoff.HandoffLogic.Suggestion, op: String = com.spacez.unlock_stage1.zerotap.ZeroTap.afterContinue(c)) {
        if (!s.playing && op != "play") return
        val l = link(c) ?: return
        val body = com.spacez.unlock_stage1.handoff.HandoffLogic.json(mapOf("op" to op, "title" to s.title, "url" to s.url, "app" to s.app))
        thread(isDaemon = true) {
            val err = l.waitAcked(l.send(Message(Kind.CONTINUED, body.toByteArray())), 8_000)
            log("told the PC '$op' for ${s.app}: ${err ?: "ok"}")
        }
    }

    // ------------------------------------------------------------------ remote lock / unlock

    /** Remote Lock PC: the existing signed phone lock event (lock-only key), over whatever pipe connects. */
    fun lockPc(c: Context): String {
        val l = link(c) ?: return "Not paired with a PC"
        userAction()
        val body = PcEvents.lockNowBody(c) ?: return "No lock key (pair again)"
        val t0 = now()
        val err = l.waitAcked(l.send(Message(Kind.LOCK, body)), 15_000)
        log("remote Lock PC: ${err ?: "accepted"} in ${now() - t0} ms")
        if (err == null) com.spacez.unlock_stage1.PcState.setLocked(c, true)
        return if (err == null) "Lock sent to your PC" else "PC not reachable: $err"
    }

    /**
     * Remote Unlock PC. [mac] MUST be the Keystore FINGERPRINT Mac released by BiometricPrompt for this
     * very operation (RemoteUnlockActivity). Blocking (<= ~45 s). Returns the line to show.
     */
    fun unlockPc(c: Context, mac: Mac): String {
        val pcId = UnlockCrypto.pcId(c) ?: return "Not paired with a PC"
        val l = link(c) ?: return "Not paired with a PC"
        userAction(90_000)
        val nonce = ByteArray(16).also { SecureRandom().nextBytes(it) }
        val frame = runCatching { RemoteUnlock.frame(mac, nonce, now(), pcId) }.getOrElse { return "Fingerprint key refused: ${it.message}" }
        val r0 = resultSeq
        val t0 = now()
        val err = l.waitAcked(l.send(Message(Kind.UNLOCK_REQUEST, frame)), 15_000)
        if (err != null) {
            log("remote Unlock PC refused or unreachable: $err")
            return notifyResult(c, AwayLogic.UnlockResult(false, if (err.contains("timed out")) "PC not reachable" else err))
        }
        log("remote Unlock PC: approval armed on the PC in ${now() - t0} ms; waiting for the sign-in")
        // the PC answers by ~32 s at the latest (its wait for the sign-in, item 20); never spin longer than 40 s
        val end = now() + 40_000
        synchronized(sync) { while (resultSeq == r0 && now() < end) sync.wait(maxOf(1, end - now())) }
        val res = if (resultSeq != r0) lastResult else null
        return notifyResult(c, res ?: AwayLogic.UnlockResult(false, "No answer from the PC - it may not have unlocked"))
    }

    /** true when the last remote unlock was confirmed by the PC. */
    fun lastResultOk() = lastResult?.ok == true

    fun onUnlockResult(c: Context, json: String) {
        val r = AwayLogic.parseUnlockResult(json) ?: return
        lastResult = r
        p(c).edit().putString("result", json).putLong("result_at", now()).apply()
        synchronized(sync) { resultSeq++; sync.notifyAll() }
        log("PC unlock result: ok=${r.ok} ${r.why}")
        if (r.ok) com.spacez.unlock_stage1.PcState.setLocked(c, false)
    }

    private fun notifyResult(c: Context, r: AwayLogic.UnlockResult): String {
        val text = AwayLogic.resultText(r)
        // the app shows every outcome (also a local timeout / refusal), not only the PC's own answer
        if (r !== lastResult) {
            lastResult = r
            p(c).edit().putString("result", com.spacez.unlock_stage1.handoff.HandoffLogic.json(mapOf("ok" to r.ok, "why" to r.why))).putLong("result_at", now()).apply()
        }
        runCatching {
            val nm = c.getSystemService(NotificationManager::class.java)
            nm.createNotificationChannel(NotificationChannel(CH, "Away mode", NotificationManager.IMPORTANCE_DEFAULT).apply { description = "Confirmations of Lock PC / Unlock PC from away" })
            nm.notify(N_RESULT, Notification.Builder(c, CH).setSmallIcon(if (r.ok) com.spacez.unlock_stage1.R.drawable.ic_aura_unlock else com.spacez.unlock_stage1.R.drawable.ic_aura_lock)
                .setContentTitle(if (r.ok) "PC unlocked" else "PC not unlocked").setContentText(text)
                .setStyle(Notification.BigTextStyle().bigText(text)).setAutoCancel(true).setTimeoutAfter(10 * 60_000L).build())
        }
        return text
    }

    // ------------------------------------------------------------------ status for the app

    fun status(c: Context): Map<String, Any?> {
        val own = Tailnet.ownIp()
        val installed = Tailnet.installed(c)
        val pc = pcAddr(c)
        val card = AwayLogic.card(UnlockCrypto.pcId(c) != null, installed, own != null, own, pc?.first)
        val l = last(c)
        return mapOf(
            "code" to card.code, "title" to card.title, "text" to card.text, "action" to card.action,
            "installed" to installed, "ownIp" to (own ?: ""), "pcIp" to (pc?.first ?: ""), "listening" to (Tailnet.listening() ?: ""),
            "lastTitle" to (l?.title() ?: ""), "lastSub" to (l?.subtitle(now()) ?: ""), "lastApp" to (l?.s?.app ?: ""),
            "recent" to (l?.recent ?: emptyList()).map { r -> AwayLogic.row(r, now()).let { mapOf("title" to it.title, "sub" to it.sub, "app" to it.app, "ago" to it.ago) } },
            "lastAt" to fetchedAt(c), "pcLocked" to (l?.pcLocked ?: false),
            "tsAutoOff" to TailnetAutoOff.enabled(c),
            "result" to (lastResult?.let { AwayLogic.resultText(it) } ?: ""),
            "resultOk" to (lastResult?.ok ?: false),
            "resultAt" to p(c).getLong("result_at", 0),
        )
    }
}
