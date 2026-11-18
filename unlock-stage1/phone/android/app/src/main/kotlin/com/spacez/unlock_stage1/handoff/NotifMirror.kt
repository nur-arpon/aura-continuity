package com.spacez.unlock_stage1.handoff

import android.app.Notification
import android.app.PendingIntent
import android.app.RemoteInput
import android.content.Context
import android.content.Intent
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.service.notification.NotificationListenerService
import android.service.notification.StatusBarNotification
import com.spacez.aura.link.Kind
import com.spacez.aura.link.Message
import com.spacez.unlock_stage1.AuraPhone
import com.spacez.unlock_stage1.UnlockState
import org.json.JSONArray
import org.json.JSONObject

/** Pure rules (JVM-tested): which phone notifications are worth a toast on the laptop. */
object NotifLogic {
    /** Android's IMPORTANCE_DEFAULT: anything below it is silent (no sound / no heads-up) and stays on the phone. */
    const val MIN_IMPORTANCE = 3
    private val SKIP_CATEGORIES = setOf("transport", "progress", "service", "sys", "status", "call", "alarm", "navigation")

    fun mirror(own: String, pkg: String, ongoing: Boolean, groupSummary: Boolean, importance: Int, category: String?, hasContent: Boolean): Boolean {
        if (pkg == own || ongoing || groupSummary || !hasContent) return false
        if (importance in 0 until MIN_IMPORTANCE) return false
        if (category != null && category in SKIP_CATEGORIES) return false
        if (pkg == "android" || pkg == "com.android.systemui") return false
        return true
    }

    fun key(k: String) = k
}

/**
 * Notification mirror, phone end. The notification listener (MediaListener, already allowed for media) reports each
 * new notification worth showing (not Aura's own, not ongoing, not silent, not a group summary) to the PC as NOTIF
 * (0x4B). Event-driven and batched: events settle for [SETTLE_MS] and leave as one message. The PC answers with
 * NOTIF_ACT (dismiss here / reply through the notification's RemoteInput). Nothing polls.
 */
object NotifMirror {
    private const val SETTLE_MS = 400L
    private const val KEEP = 60
    private val main = Handler(Looper.getMainLooper())
    @Volatile private var svc: NotificationListenerService? = null
    @Volatile private var ctx: Context? = null
    private val live = object : LinkedHashMap<String, StatusBarNotification>() {
        override fun removeEldestEntry(e: MutableMap.MutableEntry<String, StatusBarNotification>?) = size > KEEP
    }
    private val pending = ArrayList<JSONObject>()
    private val iconsSent = HashSet<String>()
    private var iconLink: Any? = null
    private val tick = Runnable { flush() }

    fun attach(s: NotificationListenerService) { svc = s; ctx = s.applicationContext }
    fun detach() { svc = null }

    fun onPosted(sbn: StatusBarNotification, rm: NotificationListenerService.RankingMap?) {
        val c = ctx ?: return
        runCatching {
            val n = sbn.notification
            val ex = n.extras
            val title = ex.getCharSequence(Notification.EXTRA_TITLE)?.toString().orEmpty()
            val text = (ex.getCharSequence(Notification.EXTRA_BIG_TEXT) ?: ex.getCharSequence(Notification.EXTRA_TEXT))?.toString().orEmpty()
            var importance = -1
            if (rm != null) { val r = NotificationListenerService.Ranking(); if (rm.getRanking(sbn.key, r)) importance = r.importance }
            val ongoing = n.flags and (Notification.FLAG_ONGOING_EVENT or Notification.FLAG_FOREGROUND_SERVICE) != 0
            val summary = n.flags and Notification.FLAG_GROUP_SUMMARY != 0
            if (!NotifLogic.mirror(c.packageName, sbn.packageName, ongoing, summary, importance, n.category, title.isNotBlank() || text.isNotBlank())) return
            val reply = replyAction(n) != null
            synchronized(live) { live[sbn.key] = sbn }
            val o = JSONObject().put("op", "post").put("key", sbn.key).put("pkg", sbn.packageName)
                .put("app", label(c, sbn.packageName)).put("title", title.take(200)).put("text", text.take(500))
                .put("when", n.`when`).put("reply", reply)
            queue(c, o)
        }.onFailure { UnlockState.log("notif mirror: ${it.javaClass.simpleName} ${it.message}") }
    }

    fun onRemoved(sbn: StatusBarNotification) {
        val c = ctx ?: return
        val had = synchronized(live) { live.remove(sbn.key) != null }
        if (!had) return
        queue(c, JSONObject().put("op", "remove").put("key", sbn.key))
    }

    private fun label(c: Context, pkg: String): String =
        runCatching { c.packageManager.getApplicationLabel(c.packageManager.getApplicationInfo(pkg, 0)).toString() }.getOrDefault(pkg)

    private fun queue(c: Context, o: JSONObject) {
        synchronized(pending) {
            // a newer event for the same notification replaces the older one
            pending.removeAll { it.optString("key") == o.optString("key") }
            pending.add(o)
        }
        main.removeCallbacks(tick); main.postDelayed(tick, SETTLE_MS)
    }

    private fun flush() {
        val c = ctx ?: return
        val l = AuraPhone.link ?: return
        val items = synchronized(pending) { val x = ArrayList(pending); pending.clear(); x }
        if (items.isEmpty()) return
        if (iconLink !== l) { iconLink = l; iconsSent.clear() }
        // each app's icon rides with its first notification on this link (the PC keeps it)
        for (o in items) {
            val pkg = o.optString("pkg")
            if (o.optString("op") == "post" && pkg.isNotEmpty() && iconsSent.add(pkg)) icon(c, pkg)?.let { o.put("icon", it) }
        }
        val body = JSONObject().put("items", JSONArray(items)).toString()
        Thread { runCatching { l.send(Message.json(Kind.NOTIF, body)) }; UnlockState.log("notif mirror -> PC: ${items.size} event(s)") }.start()
    }

    private fun icon(c: Context, pkg: String): String? = runCatching {
        val d = c.packageManager.getApplicationIcon(pkg)
        val bmp = android.graphics.Bitmap.createBitmap(64, 64, android.graphics.Bitmap.Config.ARGB_8888)
        d.setBounds(0, 0, 64, 64); d.draw(android.graphics.Canvas(bmp))
        val out = java.io.ByteArrayOutputStream(); bmp.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, out)
        android.util.Base64.encodeToString(out.toByteArray(), android.util.Base64.NO_WRAP).takeIf { it.length < 60_000 }
    }.getOrNull()

    private fun replyAction(n: Notification): Notification.Action? {
        val all = ArrayList<Notification.Action>()
        n.actions?.let { all.addAll(it) }
        runCatching { all.addAll(Notification.WearableExtender(n).actions) }
        return all.firstOrNull { a -> a.remoteInputs?.any { it.allowFreeFormInput } == true && a.actionIntent != null }
    }

    /** From the PC: `{"key","act":"dismiss"|"reply","text"?}`. */
    fun onAct(json: String) {
        val j = runCatching { JSONObject(json) }.getOrNull() ?: return
        val key = j.optString("key")
        when (j.optString("act")) {
            "dismiss" -> { runCatching { svc?.cancelNotification(key) }; UnlockState.log("notif mirror: dismissed from the PC") }
            "reply" -> reply(key, j.optString("text"))
        }
    }

    private fun reply(key: String, text: String) {
        val c = ctx ?: return
        val sbn = synchronized(live) { live[key] } ?: run { UnlockState.log("notif mirror: reply for a notification that is gone"); return }
        val a = replyAction(sbn.notification) ?: return
        val inputs = a.remoteInputs?.filter { it.allowFreeFormInput }?.toTypedArray() ?: return
        if (text.isBlank()) return
        runCatching {
            val b = Bundle()
            for (ri in inputs) b.putCharSequence(ri.resultKey, text)
            val intent = Intent()
            RemoteInput.addResultsToIntent(a.remoteInputs, intent, b)
            a.actionIntent.send(c, 0, intent)
            UnlockState.log("notif mirror: reply sent through ${sbn.packageName}")
            // a replied notification normally clears itself; make sure it also leaves the PC
        }.onFailure { UnlockState.log("notif mirror: reply failed ${it.message}") }
    }
}
