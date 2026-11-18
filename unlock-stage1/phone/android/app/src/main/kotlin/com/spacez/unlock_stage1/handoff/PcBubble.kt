package com.spacez.unlock_stage1.handoff

import android.app.Activity
import android.app.AlertDialog
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Person
import android.widget.RemoteViews
import android.content.Context
import android.content.Intent
import android.content.pm.ShortcutInfo
import android.content.pm.ShortcutManager
import android.graphics.Color
import android.graphics.drawable.Icon
import android.os.Build
import android.os.Bundle
import android.provider.Settings
import android.view.View
import android.widget.Button
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import com.spacez.unlock_stage1.R
import com.spacez.unlock_stage1.away.AwayLogic
import com.spacez.unlock_stage1.away.AwayPhone

/** Item 24: per-app "open in the app / in the browser" settings (the pure rules are in [LinkLogic]). */
object LinkPrefs {
    private fun p(c: Context) = c.getSharedPreferences("handoff", 0)
    fun all(c: Context): Map<String, LinkLogic.Mode> =
        LinkLogic.APPS.mapNotNull { a -> LinkLogic.Mode.parse(p(c).getString("link_${a.key}", null))?.let { a.key to it } }.toMap()
    fun mode(c: Context, key: String): LinkLogic.Mode = all(c)[key] ?: LinkLogic.DEFAULT_MODE
    fun set(c: Context, key: String, m: LinkLogic.Mode) {
        if (LinkLogic.APPS.none { it.key == key }) return
        p(c).edit().putString("link_$key", m.wire).apply()
    }
    fun status(c: Context): Map<String, Any?> = mapOf(
        "linkApps" to LinkLogic.APPS.map { mapOf("key" to it.key, "label" to it.label, "mode" to mode(c, it.key).wire) },
        "ytHandlers" to YtHandlers.list(c).map { mapOf("pkg" to it.pkg, "label" to it.label, "browser" to it.browser) },
        "ytChosen" to (p(c).getString("yt_handler", "") ?: ""),
        "ytEffective" to YtHandlers.effective(c),
    )
    fun setYtHandler(c: Context, pkg: String) { p(c).edit().putString("yt_handler", pkg).apply() }
    fun ytChosen(c: Context): String? = p(c).getString("yt_handler", "")?.ifEmpty { null }
}

/**
 * Item 24: a chat-head style bubble for "Continue from PC" (Android Bubbles API, Android 11+),
 * plus the expandable "More from your PC" rows of the quiet notification.
 */
object PcBubble {
    private const val CH = "aura_bubble2" // v2: the first channel was created at DEFAULT importance, which cannot be raised later
    private const val N_BUBBLE = 4210
    private const val SHORTCUT = "aura_pc"

    private fun p(c: Context) = c.getSharedPreferences("handoff", 0)
    fun enabled(c: Context) = p(c).getBoolean("bubble", true)
    fun setEnabled(c: Context, on: Boolean) { p(c).edit().putBoolean("bubble", on).apply(); if (!on) cancel(c) }

    /** The system's say: the global bubbles switch, the per-app preference (Android 12+) and the channel. */
    fun allowedBySystem(c: Context): Boolean = blockedReason(c) == null

    /** Why Android will not float our bubble, or null when it will. */
    fun blockedReason(c: Context): String? {
        if (Build.VERSION.SDK_INT < 30) return "Android 11 or newer is needed for bubbles"
        val nm = c.getSystemService(NotificationManager::class.java)
        if (!nm.areNotificationsEnabled()) return "Notifications are off for Aura"
        val global = runCatching { Settings.Secure.getInt(c.contentResolver, "notification_bubbles", 1) }.getOrDefault(1)
        if (global == 0) return "Bubbles are turned off in the system settings"
        val pref = if (Build.VERSION.SDK_INT >= 31) nm.bubblePreference
            else @Suppress("DEPRECATION") if (nm.areBubblesAllowed()) NotificationManager.BUBBLE_PREFERENCE_ALL else NotificationManager.BUBBLE_PREFERENCE_NONE
        if (pref == NotificationManager.BUBBLE_PREFERENCE_NONE) return "Bubbles are turned off for Aura"
        val ch = nm.getNotificationChannel(CH)
        if (ch != null && !ch.canBubble()) return "Bubbles are turned off for the Aura bubble channel"
        return null
    }

    /** Fresh content for the bubble view: the last pushed item (kept for [CONTENT_TTL_MS]) and what the PC sent last. */
    @Volatile var latest: Pair<String, Long>? = null
    const val CONTENT_TTL_MS = 15 * 60_000L

    /** Settings: post a bubble now from what the PC last sent. Returns a one-line result. */
    fun test(c: Context): String {
        val last = AwayPhone.last(c)
        val s = HandoffLogic.Suggestion.parse(latest?.first ?: "") ?: last?.recent?.firstOrNull()
            ?: return "Nothing from your PC yet: open a page or a video on it first"
        show(c, s, s.toJson(), force = true)
        return blockedReason(c) ?: "Bubble sent: look at the screen edge"
    }

    fun status(c: Context): Map<String, Any?> = mapOf(
        "bubble" to enabled(c), "bubbleSupported" to (Build.VERSION.SDK_INT >= 30), "bubbleAllowed" to allowedBySystem(c), "bubbleBlocked" to (blockedReason(c) ?: ""))

    /** The system's "Bubbles" page for Aura (Android 12+), else Aura's notification settings. */
    fun settingsIntent(c: Context): Intent {
        val i = if (Build.VERSION.SDK_INT >= 31) Intent(Settings.ACTION_APP_NOTIFICATION_BUBBLE_SETTINGS) else Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS)
        return i.putExtra(Settings.EXTRA_APP_PACKAGE, c.packageName).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
    }

    fun cancel(c: Context) = runCatching { c.getSystemService(NotificationManager::class.java).cancel(N_BUBBLE) }

    /** Post (or replace) the bubble for [s]; it disappears by itself after [LinkLogic.BUBBLE_TTL_MS]. */
    fun show(c: Context, s: HandoffLogic.Suggestion, json: String, force: Boolean = false) {
        latest = json to System.currentTimeMillis()
        if (!force && ChatHead.enabled(c) && ChatHead.permitted(c)) { cancel(c); return } // the chat head carries this now
        if (!force && !LinkLogic.bubbleWanted(enabled(c), HandoffPhone.suggest(c), allowedBySystem(c))) {
            HandoffPhone.log("bubble not shown: ${if (!enabled(c)) "off in Aura" else blockedReason(c) ?: "suggestions off"}"); cancel(c); return
        }
        runCatching {
            val nm = c.getSystemService(NotificationManager::class.java)
            nm.deleteNotificationChannel("aura_bubble")
            nm.createNotificationChannel(NotificationChannel(CH, "Continue bubble", NotificationManager.IMPORTANCE_HIGH).apply {
                description = "A floating bubble to carry on what your PC shows"; setSound(null, null); enableVibration(false)
                setShowBadge(false); setAllowBubbles(true) })
            val person = Person.Builder().setName("Your PC").setImportant(true).build()
            val icon = Icon.createWithResource(c, R.drawable.ic_aura_laptop)
            c.getSystemService(ShortcutManager::class.java).pushDynamicShortcut(
                ShortcutInfo.Builder(c, SHORTCUT).setShortLabel("Your PC").setLongLived(true).setPerson(person).setIcon(icon)
                    .setLocusId(android.content.LocusId(SHORTCUT))
                    .setIntent(Intent(Intent.ACTION_MAIN).setClassName(c, "com.spacez.unlock_stage1.MainActivity")).build())
            val open = Intent(c, ContinueBubbleActivity::class.java).putExtra("json", json).putExtra("at", System.currentTimeMillis())
            // bubble PendingIntents must be mutable
            val pi = PendingIntent.getActivity(c, N_BUBBLE, open, PendingIntent.FLAG_MUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
            val md = Notification.BubbleMetadata.Builder(pi, icon).setDesiredHeight(600).setAutoExpandBubble(false).setSuppressNotification(true).build()
            val what = s.title.ifBlank { HandoffLogic.host(s.url) }
            val n = Notification.Builder(c, CH).setSmallIcon(R.drawable.ic_aura_laptop)
                .setContentTitle("Continue on phone").setContentText(what).setShortcutId(SHORTCUT)
                .setLocusId(android.content.LocusId(SHORTCUT))
                .setStyle(Notification.MessagingStyle(person).addMessage("Continue: $what", System.currentTimeMillis(), person))
                .setCategory(Notification.CATEGORY_MESSAGE).setBubbleMetadata(md)
                .setOnlyAlertOnce(true).setAutoCancel(false).setTimeoutAfter(LinkLogic.BUBBLE_TTL_MS).build()
            nm.notify(N_BUBBLE, n)
            val promoted = nm.activeNotifications.firstOrNull { it.id == N_BUBBLE }?.let { it.notification.flags and Notification.FLAG_BUBBLE != 0 }
            HandoffPhone.log("bubble ${if (promoted == true) "shown" else "posted but Android did not make it a bubble ($promoted)"} (${HandoffLogic.host(s.url)})")
        }.onFailure { HandoffPhone.log("bubble failed: ${it.message}") }
    }

    /** Rows that fit the notification's expanded height (the system caps it near 256 dp: about 6 compact rows with the heading). */
    const val NOTIF_ROWS = 6

    /** How many rows to draw and whether a final "See all N" row is needed, for [n] items. */
    fun fit(n: Int): Pair<Int, Boolean> = if (n <= NOTIF_ROWS) n to false else (NOTIF_ROWS - 1) to true

    /** The expanded "More from your PC" view of the notification: every item that fits, then "See all N" opening the sheet. */
    fun moreView(c: Context, rows: List<HandoffLogic.Suggestion>, nowMs: Long): RemoteViews {
        val rv = RemoteViews(c.packageName, R.layout.aura_more_notif)
        val (shown, seeAll) = fit(rows.size)
        for ((i, r) in rows.take(shown).withIndex()) {
            val row = AwayLogic.row(r, nowMs)
            val rr = RemoteViews(c.packageName, R.layout.aura_more_row)
            rr.setTextViewText(R.id.mr_t, row.title)
            rr.setTextViewText(R.id.mr_s, listOf(row.sub, row.ago).filter { it.isNotEmpty() }.joinToString(" · "))
            val tap = Intent(c, LinkOpenActivity::class.java).putExtra("json", r.toJson())
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_NO_HISTORY)
            rr.setOnClickPendingIntent(R.id.mr_row, PendingIntent.getActivity(c, 4220 + i, tap, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT))
            rv.addView(R.id.mn_rows, rr)
        }
        if (seeAll) {
            val rr = RemoteViews(c.packageName, R.layout.aura_more_row)
            rr.setTextViewText(R.id.mr_t, "See all ${rows.size}")
            rr.setTextViewText(R.id.mr_s, ">")
            val tap = Intent(c, PcListActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            rr.setOnClickPendingIntent(R.id.mr_row, PendingIntent.getActivity(c, 4219, tap, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT))
            rv.addView(R.id.mn_rows, rr)
        }
        return rv
    }

    /** Open [s] now (YouTube at its second) in the app or browser per the settings, or per the long-press [mode]. */
    fun openNow(c: Context, s: HandoffLogic.Suggestion, mode: LinkLogic.Mode?, continued: Boolean) {
        val now = System.currentTimeMillis()
        HandoffPhone.open(c, s.url, HandoffLogic.positionNow(s.posMs, s.playing, s.atMs, now), "PC suggestion", mode)
        if (continued) AwayPhone.continued(c, s)
        cancel(c)
    }
}

/** Tap on a "More from your PC" row of the notification: open it (no UI). */
class LinkOpenActivity : Activity() {
    override fun onCreate(b: Bundle?) {
        super.onCreate(b)
        val s = intent.getStringExtra("json")?.let(HandoffLogic.Suggestion::parse)
        val mode = LinkLogic.Mode.parse(intent.getStringExtra("mode"))
        if (s != null) PcBubble.openNow(applicationContext, s, mode, true)
        finish()
    }
}

/** Builds the "main item + everything from the PC" list used by the bubble and the "See all" sheet. */
internal fun buildPcList(a: Activity, main: HandoffLogic.Suggestion?, more: List<HandoffLogic.Suggestion>, title: String): View {
    val dp = a.resources.displayMetrics.density
    fun px(v: Int) = (v * dp).toInt()
    val dark = (a.resources.configuration.uiMode and android.content.res.Configuration.UI_MODE_NIGHT_MASK) == android.content.res.Configuration.UI_MODE_NIGHT_YES
    val fg = if (dark) Color.WHITE else Color.rgb(16, 24, 40)
    val dim = if (dark) Color.rgb(170, 180, 200) else Color.rgb(90, 100, 120)
    val box = LinearLayout(a).apply {
        orientation = LinearLayout.VERTICAL
        setPadding(px(18), px(16), px(18), px(12))
        setBackgroundColor(if (dark) Color.rgb(10, 18, 36) else Color.rgb(244, 247, 252))
    }
    fun label(t: String, size: Float, color: Int, bold: Boolean = false) = TextView(a).apply {
        text = t; textSize = size; setTextColor(color); if (bold) setTypeface(typeface, android.graphics.Typeface.BOLD)
    }
    val now = System.currentTimeMillis()
    fun choose(s: HandoffLogic.Suggestion, continued: Boolean) {
        val app = LinkLogic.appFor(s.url)
        AlertDialog.Builder(a).setTitle("Open " + (app?.label ?: HandoffLogic.host(s.url)))
            .setItems(arrayOf("In the app", "In the browser")) { _, w ->
                PcBubble.openNow(a.applicationContext, s, if (w == 0) LinkLogic.Mode.APP else LinkLogic.Mode.BROWSER, continued); a.finish()
            }.show()
    }
    fun row(s: HandoffLogic.Suggestion, big: Boolean, continued: Boolean) {
        val r = AwayLogic.row(s, now)
        val v = LinearLayout(a).apply {
            orientation = LinearLayout.VERTICAL; setPadding(0, px(if (big) 10 else 8), 0, px(8))
            isClickable = true; isLongClickable = true
            setOnClickListener { PcBubble.openNow(a.applicationContext, s, null, continued); a.finish() }
            setOnLongClickListener { choose(s, continued); true }
        }
        v.addView(label(r.title, if (big) 18f else 15f, fg, true).apply { maxLines = 2; ellipsize = android.text.TextUtils.TruncateAt.END })
        v.addView(label(listOf(r.sub, r.ago).filter { it.isNotEmpty() }.joinToString(" · "), 13f, dim))
        box.addView(v)
    }
    if (main == null && more.isEmpty()) {
        box.addView(label("Nothing to continue right now.", 16f, fg))
        box.addView(label("Open a page or a video on your PC and it shows up here.", 13f, dim).apply { setPadding(0, px(6), 0, 0) })
        return ScrollView(a).apply { addView(box) }
    }
    if (main != null) {
        box.addView(label(title, 13f, dim))
        row(main, true, false)
        box.addView(Button(a).apply {
            text = if (main.app == "youtube") "Watch on phone" else "Open on phone"
            setOnClickListener { PcBubble.openNow(a.applicationContext, main, null, false); a.finish() }
            setOnLongClickListener { choose(main, false); true }
        })
    }
    if (more.isNotEmpty()) {
        box.addView(label("More from your PC (${more.size})", 13f, dim).apply { setPadding(0, px(14), 0, 0) })
        more.forEach { row(it, false, true) }
    }
    box.addView(label("Tap to open. Long-press to choose app or browser.", 12f, dim).apply { setPadding(0, px(10), 0, 0) })
    return ScrollView(a).apply { addView(box) }
}

/** What the bubble shows when expanded: the newest item and everything else from the PC, built fresh each time it is shown. */
class ContinueBubbleActivity : Activity() {
    override fun onCreate(b: Bundle?) { super.onCreate(b); render() }
    override fun onNewIntent(i: Intent) { super.onNewIntent(i); setIntent(i); render() }
    override fun onResume() { super.onResume(); render() }

    private fun render() {
        val now = System.currentTimeMillis()
        val fresh = PcBubble.latest?.takeIf { now - it.second < PcBubble.CONTENT_TTL_MS }?.first
        val main = HandoffLogic.Suggestion.parse(fresh ?: intent.getStringExtra("json") ?: "")
            ?.takeIf { fresh != null || LinkLogic.bubbleAlive(intent.getLongExtra("at", 0), now) }
        val recent = AwayPhone.last(this)?.recent ?: emptyList()
        // no live push (it expired): the PC's last answer still has the list, so the bubble is never an empty dead end
        val head = main ?: recent.firstOrNull()
        val more = LinkLogic.moreRows(head, recent, Int.MAX_VALUE)
        setContentView(buildPcList(this, head, more, "Continue on phone"))
    }
}

/** Notification "See all N": an Aura sheet listing everything the PC sent. */
class PcListActivity : Activity() {
    override fun onCreate(b: Bundle?) {
        super.onCreate(b)
        val recent = AwayPhone.last(this)?.recent ?: emptyList()
        setContentView(buildPcList(this, null, LinkLogic.moreRows(null, recent, Int.MAX_VALUE), "From your PC"))
    }
}
