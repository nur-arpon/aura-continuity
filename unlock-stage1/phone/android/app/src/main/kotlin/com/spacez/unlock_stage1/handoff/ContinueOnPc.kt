package com.spacez.unlock_stage1.handoff

import android.app.Activity
import android.app.PendingIntent
import android.content.ClipboardManager
import android.content.ContentProvider
import android.content.ContentValues
import android.content.Context
import android.content.Intent
import android.database.Cursor
import android.graphics.Color
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.os.ParcelFileDescriptor
import android.provider.OpenableColumns
import android.provider.Settings
import android.service.quicksettings.Tile
import android.service.quicksettings.TileService
import android.view.Gravity
import android.widget.Button
import android.widget.LinearLayout
import android.widget.TextView
import android.widget.Toast
import java.io.File
import kotlin.concurrent.thread

/** The "Continue on PC" decision shared by the tile, the in-app button and the share sheet. */
object ContinueOnPc {
    private val main = Handler(Looper.getMainLooper())

    fun toast(c: Context, s: String) = main.post { Toast.makeText(c.applicationContext, s, Toast.LENGTH_SHORT).show() }

    /** Blocking (call off the main thread). [clip] = clipboard text read while we had focus. */
    fun now(c: Context, clip: CharSequence?): String {
        val media = if (MediaWatch.granted(c)) MediaWatch.now() else null
        if (media?.url != null) {
            val r = HandoffPhone.sendUrl(c, media.url, media.posMs)
            if (r.startsWith("Opened")) MediaWatch.pause()
            return r
        }
        val url = clip?.toString()?.let(HandoffLogic::firstUrl)
        if (url != null) return HandoffPhone.sendUrl(c, url, if (HandoffLogic.isYouTube(url)) HandoffLogic.tParam(url)?.times(1000) else null)
        return when {
            media != null && media.title.isNotBlank() && media.pkg in HandoffLogic.YOUTUBE_PACKAGES -> {
                // no link: let the PC find it from title + channel + length (the `aura-pc --continue` path)
                val (channel, dur) = MediaWatch.identity() ?: (null to null)
                val err = com.spacez.unlock_stage1.zerotap.ZeroTap.continueNow(c, media, channel, dur)
                if (err == null) { MediaWatch.pause(); "Finding it on your PC…" } else err
            }
            media != null -> "Nothing to continue from this app. Share the page to Continue on PC"
            !MediaWatch.granted(c) -> "Nothing to continue. Share a page or video to Continue on PC"
            else -> "Nothing to continue. Share a page or video to Continue on PC"
        }
    }

    /** A shared link: YouTube links get the position of the YouTube video playing here. */
    fun sharedUrl(c: Context, url: String): String {
        var pos: Long? = null
        var fromMedia = false
        if (HandoffLogic.isYouTube(url)) {
            val m = if (MediaWatch.granted(c)) MediaWatch.now() else null
            if (m != null && m.posMs != null) { pos = m.posMs; fromMedia = true; MediaWatch.remember(m.title, url) }
            else pos = HandoffLogic.tParam(url)?.times(1000)
        }
        val r = HandoffPhone.sendUrl(c, url, pos)
        if (fromMedia && r.startsWith("Opened")) MediaWatch.pause()
        if (HandoffLogic.isYouTube(url) && !MediaWatch.granted(c)) return "$r (allow media access in Aura for the exact second)"
        return r
    }
}

/** Share sheet target "Continue on PC" (and, via an alias, "Copy to PC clipboard"). No UI. */
class ShareToPcActivity : Activity() {
    override fun onCreate(b: Bundle?) {
        super.onCreate(b)
        val i = intent
        val copy = i.component?.className?.endsWith("CopyToPc") == true
        val app = applicationContext
        val text = i.getCharSequenceExtra(Intent.EXTRA_TEXT)?.toString()
        val streams: List<Uri> = when (i.action) {
            Intent.ACTION_SEND_MULTIPLE -> if (Build.VERSION.SDK_INT >= 33) i.getParcelableArrayListExtra(Intent.EXTRA_STREAM, Uri::class.java) ?: arrayListOf()
                else @Suppress("DEPRECATION") (i.getParcelableArrayListExtra<Uri>(Intent.EXTRA_STREAM) ?: arrayListOf())
            else -> listOfNotNull(if (Build.VERSION.SDK_INT >= 33) i.getParcelableExtra(Intent.EXTRA_STREAM, Uri::class.java) else @Suppress("DEPRECATION") i.getParcelableExtra(Intent.EXTRA_STREAM))
        }
        // copy content:// streams now, while we still hold the read grant
        val files = streams.mapNotNull { u -> runCatching { copyIn(u) }.onFailure { HandoffPhone.log("share: cannot read a file: ${it.message}") }.getOrNull() }
        com.spacez.unlock_stage1.Feedback.play(app, com.spacez.unlock_stage1.FeedbackPlan.Cue.SEND)
        ContinueOnPc.toast(app, if (files.isNotEmpty()) "Sending to your PC…" else if (copy) "Copying to your PC…" else "Opening on your PC…")
        thread(isDaemon = true) {
            val r = when {
                files.isNotEmpty() -> {
                    val l = HandoffPhone.link(app)
                    if (l == null) "Not paired with a PC" else files.map { f ->
                        val size = f.length()
                        HandoffPhone.sendFileBlocking(l, f, true).also { r ->
                            if (r.startsWith("Sent")) com.spacez.unlock_stage1.HandoffInbox.add(app, "file", "out", f.name, "Sent to your PC · ${HandoffLogic.size(size)} · Downloads/Aura")
                        }
                    }.last()
                }
                text == null -> "Nothing to send"
                copy -> HandoffPhone.sendClipboardText(app, text)
                else -> HandoffLogic.firstUrl(text)?.let { ContinueOnPc.sharedUrl(app, it) } ?: HandoffPhone.sendText(app, text)
            }
            if (!(r.startsWith("Opened") || r.startsWith("Sent") || r.startsWith("Copied")))
                com.spacez.unlock_stage1.Feedback.play(app, com.spacez.unlock_stage1.FeedbackPlan.Cue.ERROR)
            ContinueOnPc.toast(app, r)
        }
        finish()
    }

    private fun copyIn(u: Uri): File {
        var name = "shared-${System.currentTimeMillis()}"
        contentResolver.query(u, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { if (it.moveToFirst()) it.getString(0)?.let { n -> name = n } }
        val dir = File(cacheDir, "out/${System.nanoTime()}").apply { mkdirs() }
        val f = File(dir, name.replace(Regex("[\\\\/:*?\"<>|]"), "_"))
        contentResolver.openInputStream(u)!!.use { i -> f.outputStream().use { i.copyTo(it, 256 * 1024) } }
        return f
    }
}

/** Tile / in-app "Continue on PC": takes focus for a moment so it may read the clipboard (Android 10+ rule). */
class ContinueActivity : Activity() {
    private var done = false
    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (!hasFocus || done) return
        done = true
        val clip = runCatching { getSystemService(ClipboardManager::class.java).primaryClip?.takeIf { it.itemCount > 0 }?.getItemAt(0)?.coerceToText(this) }.getOrNull()
        val app = applicationContext
        com.spacez.unlock_stage1.Feedback.play(app, com.spacez.unlock_stage1.FeedbackPlan.Cue.SEND)
        ContinueOnPc.toast(app, "Continuing on your PC…")
        thread(isDaemon = true) {
            val r = ContinueOnPc.now(app, clip)
            com.spacez.unlock_stage1.Feedback.play(app, if (r.startsWith("Opened") || r.startsWith("Finding")) com.spacez.unlock_stage1.FeedbackPlan.Cue.APPROVED else com.spacez.unlock_stage1.FeedbackPlan.Cue.ERROR)
            ContinueOnPc.toast(app, r)
        }
        finish()
    }
}

/** Tap on the quiet "Continue on phone" notification: open what the PC shows, at the right second now. */
class ContinueHereActivity : Activity() {
    override fun onCreate(b: Bundle?) {
        super.onCreate(b)
        val s = intent.getStringExtra("json")?.let(HandoffLogic.Suggestion::parse)
        if (s != null) HandoffPhone.open(applicationContext, s.url, HandoffLogic.positionNow(s.posMs, s.playing, s.atMs, System.currentTimeMillis()), "PC suggestion")
        finish()
    }
}

/** Explains why Aura asks for notification access before sending the user to the system switch. */
class MediaAccessActivity : Activity() {
    override fun onCreate(b: Bundle?) {
        super.onCreate(b)
        val dp = resources.displayMetrics.density
        val dark = (resources.configuration.uiMode and android.content.res.Configuration.UI_MODE_NIGHT_MASK) == android.content.res.Configuration.UI_MODE_NIGHT_YES
        val fg = if (dark) Color.WHITE else Color.rgb(16, 24, 40)
        val box = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL; gravity = Gravity.CENTER_VERTICAL
            setPadding((24 * dp).toInt(), (48 * dp).toInt(), (24 * dp).toInt(), (24 * dp).toInt())
            setBackgroundColor(if (dark) Color.rgb(10, 18, 36) else Color.rgb(244, 247, 252))
        }
        fun tv(t: String, size: Float, bold: Boolean = false) = TextView(this).apply {
            text = t; textSize = size; setTextColor(fg); if (bold) setTypeface(typeface, android.graphics.Typeface.BOLD)
            setPadding(0, 0, 0, (14 * dp).toInt())
        }
        val on = MediaWatch.granted(this)
        box.addView(tv("Notification access", 24f, true))
        box.addView(tv(
            "Aura asks for Notification access so it can do three things for you and your own paired PC:\n\n" +
            "\u2022 Read what is playing (title and position) so a video continues on your PC at the same second.\n" +
            "\u2022 Show new notifications from this phone on your PC (the app name, title and text), if you leave that on.\n" +
            "\u2022 Count unread notifications per app for the badges on your PC.\n\n" +
            "This can include the text of messages you receive. It goes only to your own paired PC, over a direct encrypted link. " +
            "It is not sent to us or to anyone else, not stored on a server, and not used for ads. " +
            "You can turn it off at any time in Android Settings, or by turning mirroring off in Aura.", 16f))
        box.addView(tv(if (on) "Notification access is on." else "Nothing is read until you allow it on the next screen.", 14f))
        box.addView(Button(this).apply {
            text = if (on) "Done" else "Agree and continue"
            setOnClickListener {
                if (!on) {
                    val i = if (Build.VERSION.SDK_INT >= 30) Intent(Settings.ACTION_NOTIFICATION_LISTENER_DETAIL_SETTINGS)
                        .putExtra(Settings.EXTRA_NOTIFICATION_LISTENER_COMPONENT_NAME, MediaWatch.component(this@MediaAccessActivity).flattenToString())
                    else Intent(Settings.ACTION_NOTIFICATION_LISTENER_SETTINGS)
                    runCatching { startActivity(i) }.onFailure { startActivity(Intent(Settings.ACTION_NOTIFICATION_LISTENER_SETTINGS)) }
                }
                finish()
            }
        })
        if (!on) box.addView(Button(this).apply { text = "Not now"; setOnClickListener { finish() } })
        setContentView(box)
    }
}

/** Quick-settings tile "Continue on PC". */
class ContinueTileService : TileService() {
    override fun onStartListening() {
        qsTile?.apply {
            val paired = com.spacez.unlock_stage1.UnlockCrypto.pcId(this@ContinueTileService) != null
            state = if (paired) Tile.STATE_INACTIVE else Tile.STATE_UNAVAILABLE
            label = "Continue on PC"
            if (Build.VERSION.SDK_INT >= 29) subtitle = if (paired) "What's playing or copied" else "Not paired"
            updateTile()
        }
    }

    override fun onClick() {
        val go = {
            val i = Intent(this, ContinueActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            if (Build.VERSION.SDK_INT >= 34) startActivityAndCollapse(PendingIntent.getActivity(this, 1, i, PendingIntent.FLAG_IMMUTABLE))
            else @Suppress("DEPRECATION") startActivityAndCollapse(i)
        }
        if (isLocked) unlockAndRun(go) else go()
    }
}

/** Read-only provider for clipboard images from the PC (cache/clip, png files), so other apps can paste them. */
class ClipProvider : ContentProvider() {
    companion object {
        fun uri(c: Context, f: File): Uri = Uri.parse("content://${c.packageName}.aura.clip/${f.parentFile?.name}/${f.name}")
    }
    private fun file(u: Uri): File? {
        val c = context ?: return null
        val seg = u.pathSegments; if (seg.size != 2) return null
        val dir = when (seg[0]) { "clip" -> File(c.cacheDir, "clip"); "Aura" -> File(c.getExternalFilesDir(null), "Aura"); "update" -> File(c.filesDir, "update"); else -> return null }
        val f = File(dir, seg[1])
        return f.takeIf { it.canonicalPath.startsWith(dir.canonicalPath + File.separator) && it.isFile }
    }
    override fun onCreate() = true
    override fun getType(uri: Uri): String = if (uri.lastPathSegment?.endsWith(".png") == true) "image/png" else if (uri.lastPathSegment?.endsWith(".apk") == true) "application/vnd.android.package-archive" else "application/octet-stream"
    override fun openFile(uri: Uri, mode: String): ParcelFileDescriptor? = file(uri)?.let { ParcelFileDescriptor.open(it, ParcelFileDescriptor.MODE_READ_ONLY) }
    override fun query(uri: Uri, p: Array<out String>?, s: String?, a: Array<out String>?, o: String?): Cursor? {
        val f = file(uri) ?: return null
        return android.database.MatrixCursor(arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE)).apply { addRow(arrayOf<Any>(f.name, f.length())) }
    }
    override fun insert(uri: Uri, values: ContentValues?): Uri? = null
    override fun delete(uri: Uri, s: String?, a: Array<out String>?) = 0
    override fun update(uri: Uri, v: ContentValues?, s: String?, a: Array<out String>?) = 0
}
