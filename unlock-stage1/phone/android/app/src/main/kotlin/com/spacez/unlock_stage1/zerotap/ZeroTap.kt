package com.spacez.unlock_stage1.zerotap

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import com.spacez.aura.link.Message
import com.spacez.unlock_stage1.AuraPhone
import com.spacez.unlock_stage1.UnlockState
import com.spacez.unlock_stage1.away.AwayLogic
import com.spacez.unlock_stage1.away.AwayPhone
import com.spacez.unlock_stage1.handoff.HandoffLogic
import com.spacez.unlock_stage1.handoff.HandoffPhone
import com.spacez.unlock_stage1.handoff.MediaWatch
import kotlin.concurrent.thread

/**
 * Zero-tap continue (item 19), Android glue. The decisions are [ContinueRules] (pure, unit-tested).
 *
 * PC -> phone: the owner opens YouTube / the browser / Facebook / Messenger / WhatsApp / Claude on
 * the phone ([onForeground], from the accessibility service's window event or the Usage-access
 * fallback); if the PC used a matching item within the last 2 minutes, it opens here at the exact
 * second with no tap, the PC pauses, and a 2-second "Undo / Pick another" chip shows.
 *
 * Phone -> PC: what plays here (MediaWatch) and what the supported apps show (accessibility) is
 * pushed to the PC as a small "state" on change, only while Aura Link is already up (no dialing,
 * no polling). The PC decides when the owner focuses a browser / WhatsApp / Claude there.
 */
object ZeroTap {
    private fun p(c: Context) = c.getSharedPreferences("zerotap", 0)
    fun log(s: String) = UnlockState.log("zero-tap: $s")
    private val main by lazy { Handler(Looper.getMainLooper()) }
    private fun now() = System.currentTimeMillis()

    /** Master switch "Zero-tap continue" (default ON) and the two directions (default ON). */
    fun enabled(c: Context) = p(c).getBoolean("on", true)
    fun pcToPhone(c: Context) = enabled(c) && p(c).getBoolean("pc_to_phone", true)
    fun phoneToPc(c: Context) = enabled(c) && p(c).getBoolean("phone_to_pc", true)
    fun set(c: Context, key: String, on: Boolean) { if (key in setOf("on", "pc_to_phone", "phone_to_pc")) p(c).edit().putBoolean(key, on).apply() }

    /** After continuing, what happens on the source device: pause (default) / mute / leave. */
    fun afterContinue(c: Context): String = p(c).getString("after", "pause")?.takeIf { it in ContinueRules.AFTER } ?: "pause"
    fun setAfterContinue(c: Context, v: String) { if (v in ContinueRules.AFTER) p(c).edit().putString("after", v).apply() }

    fun explained(c: Context) = p(c).getBoolean("explained", false)
    fun setExplained(c: Context) = p(c).edit().putBoolean("explained", true).apply()

    fun status(c: Context): Map<String, Any?> = mapOf(
        "on" to enabled(c), "pcToPhone" to p(c).getBoolean("pc_to_phone", true), "phoneToPc" to p(c).getBoolean("phone_to_pc", true),
        "after" to afterContinue(c), "a11y" to AuraAccessibility.on(c), "usage" to Foreground.granted(c),
        "contacts" to Contacts.granted(c), "last" to lastLine,
    )

    @Volatile private var lastLine: String = ""
    private fun note(s: String) { lastLine = s; log(s) }

    /** The PC continued what this phone was playing (or the owner pressed Undo on the PC). */
    fun onContinued(c: Context, json: String) {
        val m = HandoffLogic.parseFlat(json)
        val op = m["op"] as? String ?: return
        val r = when (op) {
            "pause", "mute" -> if (MediaWatch.pauseMatching(m["title"] as? String, op == "mute")) "done" else "nothing matching played"
            "play" -> if (MediaWatch.playMatching(m["title"] as? String)) "resumed" else "nothing to resume"
            else -> "left playing"
        }
        note("the PC continued ${m["app"] ?: "media"} -> $op: $r")
    }

    // ------------------------------------------------------------------ PC -> phone

    private var lastFg: Pair<String, Long>? = null
    /** what we continued here last (for Undo / not twice) */
    @Volatile private var done: Pair<String, Long>? = null
    @Volatile private var lastContinued: ContinueRules.Item? = null

    /** A supported app came to the front ([via] = "accessibility" | "usage"). */
    fun onForeground(c: Context, pkg: String, via: String) {
        if (!pcToPhone(c)) return
        if (SystemClock.elapsedRealtime() < HandoffPhone.quietUntil) { log("PC -> phone skipped ($pkg): the owner just opened something from Aura"); return }
        if (ContinueRules.appKind(pkg) == null || ContinueRules.appKind(pkg) == "spotify") return
        val t = SystemClock.elapsedRealtime()
        lastFg?.let { (lp, lt) -> if (lp == pkg && t - lt < 20_000) return }
        lastFg = pkg to t
        val fgAt = now()
        val before = MediaWatch.now()
        thread(isDaemon = true, name = "zero-tap") { runCatching { pcToPhoneNow(c.applicationContext, pkg, fgAt, before, via) }.onFailure { log("failed: ${it.message}") } }
    }

    private fun pcToPhoneNow(c: Context, pkg: String, fgAt: Long, before: MediaWatch.Now?, via: String) {
        // a fresh answer from the PC (fetched now, or one from the last 10 s)
        if (now() - AwayPhone.fetchedAt(c) > 10_000) {
            val r = AwayPhone.fetch(c, "zero-tap ($via)")
            if (r != "ok") { log("PC not asked: $r"); return }
        }
        val last = AwayPhone.last(c) ?: return
        val idleMs = last.idleMs
        val items = (listOf(last.s) + last.recent).map { s -> toItem(s, idleMs) }
        val source = ContinueRules.pickSource(items.filter { ContinueRules.fits(pkg, it) })
        // what plays here now; did the owner start something himself since he opened the app?
        val phoneNow = MediaWatch.now()
        val targetNow = phoneNow?.let { ContinueRules.Item("youtube", it.title, it.url, it.posMs, it.playing) }
        val ownerStarted = if (phoneNow != null && phoneNow.playing && phoneNow.pkg == pkg && (before == null || before.title != phoneNow.title || !before.playing)) MediaWatch.startedAt() else 0L
        val alreadyOpen = phoneNow?.takeIf { it.pkg == pkg }?.let { ContinueRules.Item("youtube", it.title, it.url) }
        val d0 = ContinueRules.decide(source, targetNow, ownerStarted, alreadyOpen, now())
        var d = d0
        if (d is ContinueRules.Decision.Go && done?.first == d.item.key() && now() - (done?.second ?: 0) < 10 * 60_000) d = ContinueRules.Decision.Skip("already continued")
        if (d !is ContinueRules.Decision.Go) { note("PC -> phone skipped (${ContinueRules.appKind(pkg)}): ${(d as? ContinueRules.Decision.Skip)?.why ?: d}"); return }
        val go = d
        val installed = (ContinueRules.YT_VIDEO_APPS + ContinueRules.YT_MUSIC_APPS + listOf(ContinueRules.FACEBOOK, ContinueRules.FACEBOOK_LITE, ContinueRules.MESSENGER, ContinueRules.WHATSAPP, ContinueRules.CLAUDE, ContinueRules.SPOTIFY))
            .filter { HandoffPhone.installed(c, it) }
        // the app the owner just opened is, by definition, the one he used last for this kind
        val target = ContinueRules.phoneApp(go.url, installed, mapOf(pkg to now()))
        if (go.seekOnly && HandoffLogic.isYouTube(go.url)) {
            MediaWatch.seekTo(ContinueRules.positionNow(go.item, now()))
            note("PC -> phone: same video already open, seeked")
        } else {
            val link = ContinueRules.phoneLink(go.url)
            val i = Intent(Intent.ACTION_VIEW, Uri.parse(link)).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            if (target != null) i.setPackage(target) else if (ContinueRules.appKind(pkg) == "web") i.setPackage(pkg)
            runCatching { c.startActivity(i) }.onFailure { note("could not open: ${it.message}"); return }
            HandoffPhone.lastOpened = go.url to SystemClock.elapsedRealtime()
            note("PC -> phone: ${ContinueRules.urlKind(go.url)} in ${i.`package` ?: "default app"} at ${ContinueRules.positionNow(go.item, now())?.div(1000)} s, ${now() - fgAt} ms after the app opened")
        }
        done = go.item.key() to now()
        lastContinued = go.item
        com.spacez.unlock_stage1.Feedback.play(c, com.spacez.unlock_stage1.FeedbackPlan.Cue.ARRIVE)
        AwayPhone.continued(c, HandoffLogic.Suggestion(go.item.app, go.item.title, go.item.url ?: go.url, go.item.posMs, go.item.playing, go.item.touchedMs), afterContinue(c))
        chip(c, go.item.title.ifBlank { HandoffLogic.host(go.url) })
    }

    fun toItem(s: HandoffLogic.Suggestion, idleMs: Long?, nowMs: Long = System.currentTimeMillis()) = ContinueRules.fromPc(s, idleMs, nowMs)

    private fun chip(c: Context, title: String) {
        val a = AuraAccessibility.instance
        val undo = { undo(c) }
        val pick = { pickAnother(c) }
        if (a != null) a.showChip(title, undo, pick, ContinueRules.CHIP_MS)
        else chipNotification(c, title)
    }

    /** Undo: leave the app we opened (Back), resume the PC, and do not offer it again. */
    fun undo(c: Context) {
        val it = lastContinued ?: return
        AuraAccessibility.instance?.back()
        AwayPhone.continued(c, HandoffLogic.Suggestion(it.app, it.title, it.url ?: "", it.posMs, true, it.touchedMs), "play")
        note("undone: back, the PC resumes")
    }

    /** Pick another: the Aura app opens on "More from your PC". */
    fun pickAnother(c: Context) {
        undoPcOnly(c)
        c.startActivity(Intent(c, com.spacez.unlock_stage1.MainActivity::class.java).putExtra("route", "more").addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP))
    }

    private fun undoPcOnly(c: Context) { /* the PC stays paused: the owner picks something else here */ }

    private const val CH = "aura_zerotap"
    private const val N_CHIP = 4301

    /** Without the accessibility service: the chip is a short heads-up notification with the two actions. */
    private fun chipNotification(c: Context, title: String) {
        val nm = c.getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(NotificationChannel(CH, "Zero-tap continue", NotificationManager.IMPORTANCE_HIGH).apply { setSound(null, null); enableVibration(false) })
        fun pi(what: String, code: Int) = PendingIntent.getBroadcast(c, code, Intent(c, ChipReceiver::class.java).setAction(what), PendingIntent.FLAG_IMMUTABLE)
        val n = Notification.Builder(c, CH).setSmallIcon(com.spacez.unlock_stage1.R.drawable.ic_aura_laptop)
            .setContentTitle("Continued from your PC").setContentText(title).setTimeoutAfter(4_000).setAutoCancel(true)
            .addAction(Notification.Action.Builder(null, "Undo", pi("undo", 1)).build())
            .addAction(Notification.Action.Builder(null, "Pick another", pi("pick", 2)).build()).build()
        runCatching { nm.notify(N_CHIP, n) }
    }

    class ChipReceiver : BroadcastReceiver() {
        override fun onReceive(c: Context, i: Intent) {
            c.getSystemService(NotificationManager::class.java).cancel(N_CHIP)
            when (i.action) { "undo" -> undo(c); "pick" -> pickAnother(c) }
        }
    }

    // ------------------------------------------------------------------ phone -> PC (state reports)

    @Volatile private var lastSentKey: String? = null
    @Volatile private var lastSentAt = 0L
    fun forceNext() { lastSentKey = null }

    /** What a supported app shows (accessibility). WhatsApp chats get their number from Contacts. */
    fun onScreen(c: Context, pkg: String, s: ScreenReader.Screen) {
        val number = if (s.app == "whatsapp") Contacts.numberFor(c, s.title) else null
        pushState(c, mapOf("app" to s.app, "title" to s.title, "url" to s.url, "number" to number, "pkg" to pkg, "playing" to false))
    }

    /** MediaWatch: YouTube / YT Music / Spotify play / pause / seek / new item. */
    fun onMedia(c: Context, n: MediaWatch.Now, channel: String?, durationMs: Long?) {
        val app = when (n.pkg) { in ContinueRules.YT_MUSIC_APPS -> "music"; ContinueRules.SPOTIFY -> "spotify"; else -> "youtube" }
        var url = n.url
        if (url == null && app == "youtube" && n.title.isNotBlank()) {
            url = YouTubeLookup.cached(n.title, channel, durationMs)
            if (url == null) { YouTubeLookup.resolveAsync(n.title, channel, durationMs) { u -> onMedia(c, n.copy(url = u), channel, durationMs) }; }
        }
        pushState(c, mapOf("app" to app, "title" to n.title, "url" to url, "pos_ms" to n.posMs, "playing" to n.playing, "channel" to channel,
            "duration_ms" to durationMs, "pkg" to n.pkg))
    }

    /**
     * "Continue on PC" for a YouTube-app video the phone has no link for: send the PC the MediaSession identity
     * (title, channel, length, position) flagged continue_now; the PC resolves it exactly as `aura-pc --continue`
     * does and answers `aura:toast <why>` only when it cannot. Blocking, returns the toast line (null = sent).
     */
    fun continueNow(c: Context, n: MediaWatch.Now, channel: String?, durationMs: Long?): String? {
        val l = HandoffPhone.link(c) ?: return "Not paired with a PC"
        val t = now()
        val json = HandoffLogic.json(mapOf("app" to "youtube", "title" to n.title, "url" to n.url, "pos_ms" to n.posMs, "playing" to n.playing,
            "channel" to channel, "duration_ms" to durationMs, "pkg" to n.pkg, "state" to true, "continue_now" to true, "at_ms" to t, "sent_ms" to t))
        val err = l.waitAcked(l.send(Message.activity(json)), 10_000)
        HandoffPhone.log("Continue on PC by identity (${n.title.take(40)}, pos ${n.posMs?.div(1000)} s): ${err ?: "sent, the PC resolves it"}")
        return if (err == null) null else "PC not reachable"
    }

    private fun pushState(c: Context, m: Map<String, Any?>) {
        if (!phoneToPc(c)) return
        // item 26: the list behind "More from your phone" menu (debounced push of its own)
        com.spacez.unlock_stage1.handoff.RecentPush.note(c, m)
        val l = AuraPhone.link ?: return
        if (!l.connected) return // never dial for this: the state waits for the next session
        val key = "${m["app"]}|${m["title"]}|${m["url"]}|${m["playing"]}|${(m["pos_ms"] as? Long)?.div(5000)}"
        if (key == lastSentKey && now() - lastSentAt < 30_000) return
        lastSentKey = key; lastSentAt = now()
        val t = now()
        val json = HandoffLogic.json(m + mapOf("state" to true, "at_ms" to t, "sent_ms" to t))
        l.send(Message.activity(json))
    }
}
