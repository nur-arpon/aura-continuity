package com.spacez.unlock_stage1.handoff

import android.content.Context
import android.graphics.Bitmap
import android.media.MediaMetadata
import android.media.session.MediaController
import android.media.session.PlaybackState
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import com.spacez.aura.link.Kind
import com.spacez.aura.link.Message
import com.spacez.unlock_stage1.AuraPhone
import com.spacez.unlock_stage1.UnlockState
import org.json.JSONArray
import org.json.JSONObject

/** Pure part (JVM-tested): has the phone's list changed enough to be worth sending again? */
object PhoneHubLogic {
    data class S(val id: String, val title: String, val artist: String, val playing: Boolean, val pos: Long)

    /** Same tracks and play states, and every position where it would be if it kept running = nothing to say. */
    fun changed(old: List<S>?, now: List<S>, elapsedMs: Long, driftMs: Long = 3_000): Boolean {
        if (old == null || old.size != now.size) return true
        for (i in now.indices) {
            val a = old[i]; val b = now[i]
            if (a.id != b.id || a.title != b.title || a.artist != b.artist || a.playing != b.playing) return true
            val expect = if (a.playing) a.pos + elapsedMs else a.pos
            if (Math.abs(b.pos - expect) > driftMs) return true
        }
        return false
    }
}

/**
 * "Now playing everywhere", phone end: every media session on the phone goes to the PC as PHONE_MEDIA (0x49) when
 * something changes (track, play / pause, a seek), settled for [SETTLE_MS]. Event-driven: session callbacks only. The PC
 * answers with PHONE_MEDIA_CMD (play / pause / next / prev / seek for one session). Artwork rides only when the track changed.
 */
object PhoneHub {
    private const val SETTLE_MS = 400L
    private val main = Handler(Looper.getMainLooper())
    @Volatile private var ctx: Context? = null
    private var lastSent: List<PhoneHubLogic.S>? = null
    private var lastSentAt = 0L
    private var lastLink: Any? = null
    private val artFor = HashMap<String, String>()
    private val tick = Runnable { push(false) }

    fun attach(c: Context) { ctx = c.applicationContext }

    /** A session appeared, changed state or metadata (called from MediaWatch's callbacks, main thread). */
    fun changed() { main.removeCallbacks(tick); main.postDelayed(tick, SETTLE_MS) }

    /** A new PC session: it has nothing yet. */
    fun pushNow() { main.post { push(true) } }

    private fun idsOf(list: List<MediaController>): List<Pair<String, MediaController>> {
        val seen = HashMap<String, Int>()
        return list.map { c -> val n = seen.merge(c.packageName, 1, Int::plus)!!; (if (n == 1) c.packageName else "${c.packageName}#$n") to c }
    }

    private fun pos(c: MediaController): Long {
        val st = c.playbackState ?: return 0
        if (st.position < 0) return 0
        return if (st.state == PlaybackState.STATE_PLAYING) st.position + ((SystemClock.elapsedRealtime() - st.lastPositionUpdateTime) * st.playbackSpeed).toLong() else st.position
    }

    private fun label(c: Context, pkg: String) = runCatching { c.packageManager.getApplicationLabel(c.packageManager.getApplicationInfo(pkg, 0)).toString() }.getOrDefault(pkg)

    private fun art(md: MediaMetadata?): String? {
        val bmp = md?.getBitmap(MediaMetadata.METADATA_KEY_ALBUM_ART) ?: md?.getBitmap(MediaMetadata.METADATA_KEY_ART) ?: md?.getBitmap(MediaMetadata.METADATA_KEY_DISPLAY_ICON) ?: return null
        return runCatching {
            val s = 96.0 / maxOf(bmp.width, bmp.height)
            val small = if (s < 1) Bitmap.createScaledBitmap(bmp, (bmp.width * s).toInt().coerceAtLeast(1), (bmp.height * s).toInt().coerceAtLeast(1), true) else bmp
            val out = java.io.ByteArrayOutputStream(); small.compress(Bitmap.CompressFormat.JPEG, 60, out)
            android.util.Base64.encodeToString(out.toByteArray(), android.util.Base64.NO_WRAP).takeIf { it.length < 30_000 }
        }.getOrNull()
    }

    private fun push(force: Boolean) {
        val c = ctx ?: return
        val l = AuraPhone.link ?: return
        if (!l.connected) return // never dial for this: the state goes out when the next session starts
        if (lastLink !== l) { lastLink = l; artFor.clear(); lastSent = null }
        val items = idsOf(MediaWatch.sessions()).filter { (_, s) -> s.metadata?.getString(MediaMetadata.METADATA_KEY_TITLE).orEmpty().isNotBlank() }
        val now = items.map { (id, s) ->
            val md = s.metadata
            PhoneHubLogic.S(id, md?.getString(MediaMetadata.METADATA_KEY_TITLE).orEmpty(), md?.getString(MediaMetadata.METADATA_KEY_ARTIST).orEmpty(), s.playbackState?.state == PlaybackState.STATE_PLAYING, pos(s))
        }
        val t = System.currentTimeMillis()
        if (!force && !PhoneHubLogic.changed(lastSent, now, t - lastSentAt)) return
        if (!force && lastSent == null && now.isEmpty()) return
        val arr = JSONArray()
        items.forEachIndexed { i, (id, s) ->
            val md = s.metadata; val st = now[i]
            val o = JSONObject().put("id", id).put("pkg", s.packageName).put("app", label(c, s.packageName))
                .put("title", st.title.take(200)).put("artist", st.artist.take(200)).put("playing", st.playing)
                .put("pos", st.pos).put("dur", md?.getLong(MediaMetadata.METADATA_KEY_DURATION)?.coerceAtLeast(0) ?: 0L)
            val key = st.title + "\u0001" + st.artist
            if (artFor[id] != key) { artFor[id] = key; art(md)?.let { o.put("art", it) } }
            arr.put(o)
        }
        lastSent = now; lastSentAt = t
        val body = JSONObject().put("v", 1).put("at", t).put("sessions", arr).toString()
        Thread { runCatching { l.send(Message.json(Kind.PHONE_MEDIA, body)) } }.start()
        UnlockState.log("now playing -> PC: ${now.size} session(s)${now.joinToString("") { " [${it.id} ${if (it.playing) "playing" else "paused"}]" }}")
    }

    /** From the PC: `{"id","cmd":"toggle"|"play"|"pause"|"next"|"prev"|"seek","pos"?}`. */
    fun onCmd(json: String) {
        val j = runCatching { JSONObject(json) }.getOrNull() ?: return
        main.post {
            val s = idsOf(MediaWatch.sessions()).firstOrNull { it.first == j.optString("id") }?.second ?: run { UnlockState.log("now playing: command for a session that is gone"); return@post }
            val tc = s.transportControls
            runCatching {
                when (j.optString("cmd")) {
                    "toggle" -> if (s.playbackState?.state == PlaybackState.STATE_PLAYING) tc.pause() else tc.play()
                    "play" -> tc.play()
                    "pause" -> tc.pause()
                    "next" -> tc.skipToNext()
                    "prev" -> tc.skipToPrevious()
                    "seek" -> if (j.has("pos")) tc.seekTo(j.optLong("pos"))
                }
            }
            UnlockState.log("now playing: PC said ${j.optString("cmd")} for ${s.packageName}")
        }
    }
}
