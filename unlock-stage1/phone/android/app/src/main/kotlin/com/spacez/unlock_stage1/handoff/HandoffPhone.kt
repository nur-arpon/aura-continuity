package com.spacez.unlock_stage1.handoff

import android.app.KeyguardManager
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.ClipData
import android.content.ClipboardManager
import android.content.ContentValues
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.provider.MediaStore
import com.spacez.aura.link.AuraLink
import com.spacez.aura.link.FileOffer
import com.spacez.aura.link.Message
import com.spacez.unlock_stage1.AuraPhone
import com.spacez.unlock_stage1.UnlockService
import com.spacez.unlock_stage1.UnlockState
import java.io.File
import kotlin.concurrent.thread

/**
 * Aura Handoff, phone side (item 12). Everything travels over Aura Link (no KDE Connect):
 *  - PC -> phone links open at once in the background (the CDM association exempts the app from
 *    background-activity-start limits): YouTube in the YouTube app at ?t=, pages in the default
 *    browser. Phone locked = the link waits and opens on unlock (ACTION_USER_PRESENT).
 *  - phone -> PC "Continue on PC": [send] with the YouTube position from [MediaWatch].
 *  - passive suggestions both ways (quiet notification here, ACTIVITY to the PC).
 *  - clipboard (opt-in) and files (Download/Aura).
 * Contents of the clipboard / shared text are never logged.
 */
object HandoffPhone {
    private const val CH_SUGGEST = "aura_suggest"
    private const val CH_FILES = "aura_files"
    private const val N_SUGGEST = 4201
    private val main = Handler(Looper.getMainLooper())
    val echo = HandoffLogic.ClipEcho()
    @Volatile private var pendingUrl: Pair<String, Long?>? = null
    @Volatile private var presentReceiver: BroadcastReceiver? = null
    /** Last link opened from the PC (url, elapsedRealtime) - lets MediaWatch learn which video plays. */
    @Volatile var lastOpened: Pair<String, Long>? = null
    /** Until this elapsedRealtime Zero-tap must not "continue" anything: the owner just opened something from Aura himself (chat head), and
     *  the app he opened coming to the front must not make Zero-tap open the PC's video on top of it. */
    @Volatile var quietUntil = 0L
    fun quietFor(ms: Long) { quietUntil = SystemClock.elapsedRealtime() + ms }

    private fun p(c: Context) = c.getSharedPreferences("handoff", 0)
    fun clipSync(c: Context) = p(c).getBoolean("clip_sync", false)
    fun setClipSync(c: Context, on: Boolean) = p(c).edit().putBoolean("clip_sync", on).apply()
    fun suggest(c: Context) = p(c).getBoolean("suggest", true)
    fun setSuggest(c: Context, on: Boolean) = p(c).edit().putBoolean("suggest", on).apply()
    fun log(s: String) = UnlockState.log("handoff: $s")

    /** The Aura Link to the PC, starting the CDM service if it is not up yet (e.g. right after an update). */
    fun link(c: Context): AuraLink? {
        // only explicit user actions call this (Continue on PC, clipboard, share): the tailnet may carry them when away
        com.spacez.unlock_stage1.away.AwayPhone.userAction()
        AuraPhone.link?.let { return it }
        UnlockService.start(c.applicationContext, "handoff")
        repeat(30) { AuraPhone.link?.let { return it }; SystemClock.sleep(100) }
        return AuraPhone.link ?: AuraPhone.ensureLink(c)
    }

    // ------------------------------------------------------------------ PC -> phone

    fun onUrl(c: Context, url: String, posMs: Long?) {
        if (!url.startsWith("https://") && !url.startsWith("http://")) return
        val km = c.getSystemService(KeyguardManager::class.java)
        if (km.isKeyguardLocked) {
            pendingUrl = url to posMs
            log("link from PC waits for unlock (${HandoffLogic.host(url)})")
            watchUnlock(c)
            return
        }
        open(c, url, posMs, "PC")
    }

    private fun watchUnlock(c: Context) {
        if (presentReceiver != null) return
        val r = object : BroadcastReceiver() {
            override fun onReceive(ctx: Context, i: Intent) {
                val pu = pendingUrl ?: return
                pendingUrl = null
                runCatching { ctx.applicationContext.unregisterReceiver(this) }; presentReceiver = null
                open(ctx.applicationContext, pu.first, pu.second, "PC (after unlock)")
            }
        }
        presentReceiver = r
        c.applicationContext.registerReceiver(r, IntentFilter(Intent.ACTION_USER_PRESENT))
    }

    fun toastOnPc(c: Context, msg: String) {
        ContinueOnPc.toast(c, msg)
        com.spacez.unlock_stage1.Feedback.play(c, com.spacez.unlock_stage1.FeedbackPlan.Cue.ERROR)
    }

    fun installed(c: Context, pkg: String) = runCatching { c.packageManager.getPackageInfo(pkg, 0); true }.getOrDefault(false)

    /**
     * Item 24: per app (LinkedIn, YouTube...) the owner chose "in the app" or "in the browser" ([LinkPrefs]; [mode] is a
     * one-off long-press choice). "In the app" is a plain ACTION_VIEW with no package, so Android sends it to whatever
     * handler the user set (YouTube included) and to the browser when no app takes it. "Browser" and sites without an app
     * go to the default browser, never a chooser.
     */
    fun viewIntent(c: Context, url: String, posMs: Long?, mode: LinkLogic.Mode? = null): Intent {
        val target = HandoffLogic.withTime(url, posMs)
        val i = Intent(Intent.ACTION_VIEW, Uri.parse(target)).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        if (LinkLogic.decide(url, LinkPrefs.all(c), mode) == LinkLogic.Mode.APP) {
            // YouTube: Android has no default handler for youtube.com, so a plain VIEW lands in the browser.
            // Use the owner's pick / the system default / the first YouTube-capable app (item 27). Others: Android decides.
            if (LinkLogic.appFor(url)?.key == "youtube") YtHandlers.pick(c)?.let { i.setPackage(it) }
        } else {
            // the user's default browser, never a chooser
            val probe = Intent(Intent.ACTION_VIEW, Uri.parse("https://example.com/")).addCategory(Intent.CATEGORY_BROWSABLE)
            val ri = c.packageManager.resolveActivity(probe, PackageManager.MATCH_DEFAULT_ONLY)
            val pkg = ri?.activityInfo?.packageName
            if (pkg != null && pkg != "android") i.setPackage(pkg)
        }
        return i
    }

    fun open(c: Context, url: String, posMs: Long?, from: String, mode: LinkLogic.Mode? = null) {
        val t0 = SystemClock.elapsedRealtime()
        quietFor(15_000)
        val i = viewIntent(c, url, posMs, mode)
        try {
            c.startActivity(i)
            com.spacez.unlock_stage1.Feedback.play(c, com.spacez.unlock_stage1.FeedbackPlan.Cue.ARRIVE)
            lastOpened = url to SystemClock.elapsedRealtime()
            val yt = HandoffLogic.isYouTube(url)
            com.spacez.unlock_stage1.HandoffInbox.add(c, if (yt) "video" else "link", "in",
                knownTitle(c, url) ?: if (yt) "YouTube video" else HandoffLogic.host(url),
                listOfNotNull(if (from.contains("continue")) "Continued from your PC" else "From your PC", posMs?.let { clockOf(it) }).joinToString(" · "), url = url)
            log("opened link from $from in ${i.`package` ?: "default app"} (${HandoffLogic.host(url)}, pos ${posMs?.div(1000)} s) in ${SystemClock.elapsedRealtime() - t0} ms")
        } catch (t: Throwable) {
            log("could not open link from $from: ${t.message}")
        }
        cancelSuggestion(c)
    }

    /** The title the PC told us for this link (its pushed / recent items), for inbox rows. */
    private fun knownTitle(c: Context, url: String): String? {
        val l = com.spacez.unlock_stage1.away.AwayPhone.last(c) ?: return null
        val k = LinkLogic.sameKey(url)
        return (listOf(l.s) + l.recent).firstOrNull { LinkLogic.sameKey(it.url) == k && it.title.isNotBlank() }?.title
    }

    private fun clockOf(ms: Long): String { val s = ms / 1000; return if (s >= 3600) "%d:%02d:%02d".format(s / 3600, s / 60 % 60, s % 60) else "%d:%02d".format(s / 60, s % 60) }

    fun onClipboard(c: Context, mime: String, data: ByteArray) {
        if (!clipSync(c)) { log("clipboard from PC ignored (sync is off)"); return }
        if (!mime.startsWith("text/")) return
        val text = String(data, Charsets.UTF_8)
        echo.noteApplied(data)
        main.post {
            runCatching {
                c.getSystemService(ClipboardManager::class.java).setPrimaryClip(ClipData.newPlainText("Aura", text))
                log("clipboard from PC applied (text, ${text.length} chars)")
                com.spacez.unlock_stage1.HandoffInbox.add(c, "clip", "in", "Clipboard from your PC", "Text · ${text.length} characters · ready to paste")
            }.onFailure { log("clipboard set failed: ${it.message}") }
        }
    }

    private fun clipImage(c: Context, file: File) {
        if (!clipSync(c)) { file.delete(); log("clipboard image from PC ignored (sync is off)"); return }
        val dir = File(c.cacheDir, "clip").apply { mkdirs() }
        val dst = File(dir, "clip-${System.currentTimeMillis()}.png")
        if (!file.renameTo(dst)) { file.copyTo(dst, true); file.delete() }
        dir.listFiles()?.filter { it != dst && System.currentTimeMillis() - it.lastModified() > 3600_000 }?.forEach { it.delete() }
        echo.noteApplied(dst.readBytes())
        main.post {
            runCatching {
                val uri = ClipProvider.uri(c, dst)
                c.getSystemService(ClipboardManager::class.java).setPrimaryClip(ClipData.newUri(c.contentResolver, "Aura", uri))
                log("clipboard from PC applied (image)")
                com.spacez.unlock_stage1.HandoffInbox.add(c, "clip", "in", "Image on the clipboard", "From your PC · ready to paste")
            }.onFailure { log("clipboard image failed: ${it.message}") }
        }
    }

    /** Received files go to the public Download/Aura (MediaStore, no storage permission). */
    fun onFile(c: Context, file: File, offer: FileOffer) {
        if (offer.name.startsWith(Message.CLIPBOARD_FILE_PREFIX)) {
            if (offer.name.endsWith(".txt")) { onClipboard(c, "text/plain", file.readBytes()); file.delete() } else clipImage(c, file)
            return
        }
        thread(isDaemon = true) {
            val uri = runCatching { publish(c, file, offer.name, offer.mime) }.getOrElse { log("could not move ${offer.name} to Downloads: ${it.message}"); null }
            if (uri != null) file.delete()
            log("file from PC: ${offer.name} (${offer.size} bytes) -> ${if (uri != null) "Download/Aura" else file.parent}")
            notifyFile(c, offer.name, uri ?: ClipProvider.uri(c, file), offer.mime)
            com.spacez.unlock_stage1.HandoffInbox.add(c, "file", "in", offer.name, "From your PC · ${HandoffLogic.size(offer.size)} · Download/Aura",
                uri = (uri ?: ClipProvider.uri(c, file)).toString(), mime = offer.mime)
        }
    }

    private fun publish(c: Context, f: File, name: String, mime: String): Uri? {
        if (Build.VERSION.SDK_INT < 29) {
            val d = File(Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOWNLOADS), "Aura").apply { mkdirs() }
            val out = File(d, name); f.copyTo(out, true); return Uri.fromFile(out)
        }
        val cv = ContentValues().apply {
            put(MediaStore.MediaColumns.DISPLAY_NAME, name)
            put(MediaStore.MediaColumns.MIME_TYPE, mime.ifEmpty { "application/octet-stream" })
            put(MediaStore.MediaColumns.RELATIVE_PATH, Environment.DIRECTORY_DOWNLOADS + "/Aura")
            put(MediaStore.MediaColumns.IS_PENDING, 1)
        }
        val r = c.contentResolver
        val uri = r.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, cv) ?: return null
        r.openOutputStream(uri)!!.use { o -> f.inputStream().use { it.copyTo(o, 256 * 1024) } }
        r.update(uri, ContentValues().apply { put(MediaStore.MediaColumns.IS_PENDING, 0) }, null, null)
        return uri
    }

    private fun channels(c: Context) {
        val nm = c.getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(NotificationChannel(CH_SUGGEST, "Continue on phone", NotificationManager.IMPORTANCE_LOW).apply {
            description = "A quiet offer to carry on what your PC shows"; setShowBadge(false) })
        nm.createNotificationChannel(NotificationChannel(CH_FILES, "Files from PC", NotificationManager.IMPORTANCE_LOW).apply { setShowBadge(false) })
    }

    private fun notifyFile(c: Context, name: String, uri: Uri, mime: String) {
        channels(c)
        val view = Intent(Intent.ACTION_VIEW).setDataAndType(uri, mime.ifEmpty { "*/*" }).addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
        val pi = PendingIntent.getActivity(c, name.hashCode(), view, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        val n = Notification.Builder(c, CH_FILES).setSmallIcon(com.spacez.unlock_stage1.R.drawable.ic_aura_glyph)
            .setContentTitle("From PC: $name").setContentText("Saved in Download/Aura").setContentIntent(pi).setAutoCancel(true).build()
        runCatching { c.getSystemService(NotificationManager::class.java).notify(name.hashCode(), n) }
    }

    /** PC suggests "Continue on phone": a quiet notification, replaced on change, gone after 10 min. */
    fun onActivity(c: Context, json: String) {
        if (!suggest(c)) return
        val s = HandoffLogic.Suggestion.parse(json)
        if (s == null) { cancelSuggestion(c); PcBubble.cancel(c); return }
        activeJson = json to System.currentTimeMillis()
        com.spacez.unlock_stage1.away.AwayPhone.onPushed(c, json)
        postSuggestion(c, json, s)
        PcBubble.show(c, s, json)
        // the expanded "More from your PC" rows come from the PC's last answer; ask for a fresh one (rate limited)
        com.spacez.unlock_stage1.away.AwayPhone.autoFetch(c, "suggestion")
    }

    /** A fresh LAST_ACTIVITY answer arrived: redraw the live suggestion so its "More from your PC" rows are current. */
    fun refreshMore(c: Context) {
        val a = activeJson ?: return
        if (System.currentTimeMillis() - a.second > 10 * 60_000L || !suggest(c)) return
        HandoffLogic.Suggestion.parse(a.first)?.let { postSuggestion(c, a.first, it) }
    }

    @Volatile private var activeJson: Pair<String, Long>? = null

    private fun postSuggestion(c: Context, json: String, s: HandoffLogic.Suggestion) {
        channels(c)
        val tap = Intent(c, ContinueHereActivity::class.java).putExtra("json", json)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_NO_HISTORY)
        val pi = PendingIntent.getActivity(c, N_SUGGEST, tap, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        val what = if (s.app == "youtube") "Watch on phone" else "Open on phone"
        val n = Notification.Builder(c, CH_SUGGEST)
            .setSmallIcon(com.spacez.unlock_stage1.R.drawable.ic_aura_laptop)
            .setContentTitle(what)
            .setContentText(s.title.ifBlank { HandoffLogic.host(s.url) })
            .setSubText("PC")
            .setContentIntent(pi).setAutoCancel(true).setOnlyAlertOnce(true)
            .setTimeoutAfter(10 * 60_000L)
            .addAction(Notification.Action.Builder(null, what, pi).build())
        val more = LinkLogic.moreRows(s, com.spacez.unlock_stage1.away.AwayPhone.last(c)?.recent ?: emptyList(), Int.MAX_VALUE)
        if (more.isNotEmpty()) n.setStyle(Notification.DecoratedCustomViewStyle()).setCustomBigContentView(PcBubble.moreView(c, more, System.currentTimeMillis()))
        runCatching { c.getSystemService(NotificationManager::class.java).notify(N_SUGGEST, n.build()) }
        log("PC suggests ${s.app} (${HandoffLogic.host(s.url)})")
    }

    fun cancelSuggestion(c: Context) { activeJson = null; PcBubble.cancel(c); runCatching { c.getSystemService(NotificationManager::class.java).cancel(N_SUGGEST) } }

    // ------------------------------------------------------------------ phone -> PC

    /** Result line for the UI / a toast. Blocks up to ~10 s: call off the main thread. */
    fun sendUrl(c: Context, url: String, posMs: Long?): String {
        val l = link(c) ?: return "Not paired with a PC"
        val id = l.send(Message.url(url, posMs))
        val err = l.waitAcked(id, 10_000)
        log("Continue on PC (${HandoffLogic.host(url)}, pos ${posMs?.div(1000)} s): ${err ?: "opened on the PC"}")
        if (err == null) {
            val yt = HandoffLogic.isYouTube(url)
            com.spacez.unlock_stage1.HandoffInbox.add(c, if (yt) "video" else "link", "out",
                (if (yt) MediaWatch.now()?.title?.takeIf { it.isNotBlank() } else null) ?: if (yt) "YouTube video" else HandoffLogic.host(url),
                listOfNotNull("Opened on your PC", posMs?.let { clockOf(it) }).joinToString(" · "), url = url)
        }
        if (err != null) {
            val ts = com.spacez.unlock_stage1.away.Tailnet
            if (ts.installed(c) && ts.ownIp() == null) {
                tailscaleOffNudge(c)
                return "PC not reachable: Tailscale is off on this phone. Tap the Aura notice to turn it on"
            }
        }
        return if (err == null) "Opened on your PC" else "PC not reachable: it opens when the PC is back (5 min)"
    }

    /** Away from home Aura Link needs Tailscale: one tap opens it (and Always-on VPN keeps it on). */
    private fun tailscaleOffNudge(c: Context) {
        channels(c)
        val i = com.spacez.unlock_stage1.away.Tailnet.openIntent(c) ?: return
        val pi = PendingIntent.getActivity(c, 4230, i, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        val n = Notification.Builder(c, CH_SUGGEST).setSmallIcon(com.spacez.unlock_stage1.R.drawable.ic_aura_laptop)
            .setContentTitle("Turn on Tailscale").setContentText("Your PC is out of Bluetooth range. Tailscale connects Aura from anywhere.")
            .setContentIntent(pi).setAutoCancel(true).setTimeoutAfter(5 * 60_000L)
            .addAction(Notification.Action.Builder(null, "Turn on", pi).build()).build()
        runCatching { c.getSystemService(NotificationManager::class.java).notify(4230, n) }
    }

    fun sendClipboardText(c: Context, text: String): String {
        val l = link(c) ?: return "Not paired with a PC"
        val bytes = text.toByteArray(Charsets.UTF_8)
        if (!echo.shouldSend(bytes) && text.isNotEmpty()) return "Already on the PC clipboard"
        if (bytes.size > Message.CLIPBOARD_MAX) {
            val f = File(File(c.cacheDir, "out").apply { mkdirs() }, "${Message.CLIPBOARD_FILE_PREFIX}.txt"); f.writeBytes(bytes)
            return sendFileBlocking(l, f, true).also { f.delete() }
        }
        val err = l.waitAcked(l.send(Message.clipboard("text/plain", bytes)), 10_000)
        log("clipboard -> PC (text, ${text.length} chars): ${err ?: "ok"}")
        if (err == null) com.spacez.unlock_stage1.HandoffInbox.add(c, "clip", "out", "Clipboard to your PC", "Text · ${text.length} characters")
        return if (err == null) "Copied to the PC clipboard" else "PC not reachable"
    }

    fun sendText(c: Context, text: String): String {
        val l = link(c) ?: return "Not paired with a PC"
        val err = l.waitAcked(l.send(Message.text(text)), 10_000)
        log("text -> PC (${text.length} chars): ${err ?: "ok"}")
        if (err == null) com.spacez.unlock_stage1.HandoffInbox.add(c, "text", "out", "Text to your PC", "${text.length} characters")
        return if (err == null) "Sent to your PC" else "PC not reachable"
    }

    fun sendFileBlocking(l: AuraLink, f: File, deleteAfter: Boolean): String {
        return try {
            val r = l.waitFile(l.sendFile(f, null), 30 * 60_000L)
            log("file -> PC: ${f.name} $r")
            "Sent ${f.name} to the PC (Downloads\\Aura)"
        } catch (t: Throwable) {
            log("file -> PC failed: ${f.name}: ${t.message}"); "Could not send ${f.name}: ${t.message}"
        } finally { if (deleteAfter) f.delete() }
    }

    /** Passive "Continue on PC": what plays here, sent to the PC (MediaWatch debounces, on change only). */
    fun suggestToPc(c: Context, s: HandoffLogic.Suggestion) {
        if (!suggest(c)) return
        val l = AuraPhone.link ?: return
        val id = l.send(Message.activity(s.toJson()))
        // a stale suggestion is worthless: if the PC is not reachable within 6 s, withdraw it (no retries, no battery)
        thread(isDaemon = true) {
            var waited = 0
            while (l.queued(id) && waited < 6000) { SystemClock.sleep(100); waited += 100 }
            val dropped = l.cancel(id)
            log("suggested ${s.app} to the PC: ${if (dropped) "PC not reachable, dropped" else "sent"}")
        }
    }
}
