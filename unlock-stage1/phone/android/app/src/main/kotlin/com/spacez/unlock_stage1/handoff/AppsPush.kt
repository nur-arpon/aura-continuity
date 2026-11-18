package com.spacez.unlock_stage1.handoff

import android.app.usage.UsageEvents
import android.app.usage.UsageStatsManager
import android.content.Context
import android.content.Intent
import com.spacez.aura.link.Message
import com.spacez.unlock_stage1.AuraPhone

/** Pure part (JVM-tested): newest-first distinct launchable packages from (package, time) events. */
object AppsLogic {
    const val MAX = 8
    data class App(val pkg: String, val label: String)

    fun pick(events: List<Pair<String, Long>>, launchable: Map<String, String>, skip: Set<String>): List<App> =
        events.sortedByDescending { it.second }.map { it.first }.distinct()
            .filter { it !in skip && launchable.containsKey(it) }.take(MAX).map { App(it, launchable.getValue(it)) }

    /** ACTIVITY JSON: {"state":true,"apps_list":true,"w","h","dpi","apps":[{pkg,label,n?},...]}; n = unread notifications of that app (omitted when 0). */
    fun toJson(apps: List<App>, w: Int, h: Int, dpi: Int, nowMs: Long, counts: Map<String, Int> = emptyMap()): String =
        "{\"state\":true,\"apps_list\":true,\"at_ms\":$nowMs,\"sent_ms\":$nowMs,\"w\":$w,\"h\":$h,\"dpi\":$dpi,\"apps\":[" +
            apps.joinToString(",") { a ->
                HandoffLogic.json(linkedMapOf<String, Any?>("pkg" to a.pkg, "label" to a.label).also { m -> counts[a.pkg]?.takeIf { it > 0 }?.let { m["n"] = it } })
            } + "]}"
    /** ACTIVITY JSON for one app icon: {"state":true,"app_icon":true,"pkg","png":base64}. */
    fun iconJson(pkg: String, b64: String): String =
        "{\"state\":true,\"app_icon\":true,\"pkg\":\"$pkg\",\"png\":\"$b64\"}"
}

/**
 * "Continue any phone app on the laptop": the apps used recently (Usage access, already granted for zero-tap), sent to the PC
 * next to the recent list. Event-driven only (rides RecentPush's sends); never polled, never dials.
 */
object AppsPush {
    private var lastSig: String? = null
    private var lastAt = 0L
    private val iconsSent = HashSet<String>()
    private var iconLink: Any? = null

    fun send(c: Context, force: Boolean) {
        val now = System.currentTimeMillis()
        if (!force && now - lastAt < 30_000) return
        if (!com.spacez.unlock_stage1.zerotap.Foreground.granted(c)) return
        val l = AuraPhone.link ?: return
        if (!l.connected) return
        val apps = runCatching { recent(c, now) }.getOrNull() ?: return
        if (apps.isEmpty()) return
        if (iconLink !== l) { iconLink = l; iconsSent.clear() } // a new link: the PC may have lost them
        sendIcons(c, l, apps)
        val counts = NotifCounts.current()
        val sig = apps.joinToString("|") { it.pkg + ":" + (counts[it.pkg] ?: 0) }
        lastAt = now
        if (!force && sig == lastSig) return
        lastSig = sig
        val dm = c.resources.displayMetrics
        l.send(Message.activity(AppsLogic.toJson(apps, minOf(dm.widthPixels, dm.heightPixels), maxOf(dm.widthPixels, dm.heightPixels), dm.densityDpi, now, counts)))
    }

    /** Each app icon (64 px PNG) is sent once per link; the PC keeps them on disk. */
    private fun sendIcons(c: Context, l: com.spacez.aura.link.AuraLink, apps: List<AppsLogic.App>) {
        for (a in apps) {
            if (a.pkg in iconsSent) continue
            runCatching {
                val d = c.packageManager.getApplicationIcon(a.pkg)
                val bmp = android.graphics.Bitmap.createBitmap(64, 64, android.graphics.Bitmap.Config.ARGB_8888)
                d.setBounds(0, 0, 64, 64)
                d.draw(android.graphics.Canvas(bmp))
                val out = java.io.ByteArrayOutputStream()
                bmp.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, out)
                val b64 = android.util.Base64.encodeToString(out.toByteArray(), android.util.Base64.NO_WRAP)
                if (b64.length < 60_000) l.send(Message.activity(AppsLogic.iconJson(a.pkg, b64)))
            }
            iconsSent.add(a.pkg)
        }
    }

    private fun recent(c: Context, now: Long): List<AppsLogic.App> {
        val pm = c.packageManager
        val launch = HashMap<String, String>()
        for (ri in pm.queryIntentActivities(Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_LAUNCHER), 0))
            launch[ri.activityInfo.packageName] = ri.loadLabel(pm).toString()
        val home = pm.resolveActivity(Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_HOME), 0)?.activityInfo?.packageName
        val usm = c.getSystemService(UsageStatsManager::class.java) ?: return emptyList()
        val ev = usm.queryEvents(now - 24 * 3_600_000L, now)
        val e = UsageEvents.Event()
        val seen = ArrayList<Pair<String, Long>>()
        while (ev.hasNextEvent()) {
            ev.getNextEvent(e)
            if (e.eventType == UsageEvents.Event.ACTIVITY_RESUMED) seen.add(e.packageName to e.timeStamp)
        }
        return AppsLogic.pick(seen, launch, setOfNotNull(c.packageName, home, "com.android.systemui"))
    }
}

/**
 * Unread-notification COUNT per app for the badges on the laptop's chat head. Only package names and a number per package
 * leave the phone: no title, text or icon of any notification is read. Event-driven by the listener callbacks (debounced 3 s),
 * and a changed count re-sends the apps list.
 */
object NotifCounts {
    private val main = android.os.Handler(android.os.Looper.getMainLooper())
    @Volatile private var counts: Map<String, Int> = emptyMap()
    @Volatile private var svc: android.service.notification.NotificationListenerService? = null
    @Volatile private var ctx: Context? = null

    fun current(): Map<String, Int> = counts

    /** Pure rule (JVM-tested): what counts as "unread" = clearable, not ongoing, not a group summary; per package. */
    fun tally(items: List<Triple<String, Boolean, Boolean>>): Map<String, Int> =
        items.filter { it.second && it.third }.groupingBy { it.first }.eachCount()

    fun attach(s: android.service.notification.NotificationListenerService) { svc = s; ctx = s.applicationContext; changed() }
    fun detach() { svc = null; counts = emptyMap() }

    private val fire = Runnable {
        val s = svc ?: return@Runnable
        val c = ctx ?: return@Runnable
        val next = runCatching {
            tally(s.activeNotifications.orEmpty().map {
                Triple(it.packageName, it.isClearable && !it.isOngoing, it.notification.flags and android.app.Notification.FLAG_GROUP_SUMMARY == 0)
            })
        }.getOrNull() ?: return@Runnable
        if (next == counts) return@Runnable
        counts = next
        Thread { runCatching { AppsPush.send(c, true) } }.start()
    }

    /** A notification came or went: look again in 3 s (a burst is one look). */
    fun changed() { main.removeCallbacks(fire); main.postDelayed(fire, 3_000) }
}
