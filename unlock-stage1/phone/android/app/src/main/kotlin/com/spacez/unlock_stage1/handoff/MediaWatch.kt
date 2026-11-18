package com.spacez.unlock_stage1.handoff

import android.content.ComponentName
import android.content.Context
import android.media.MediaMetadata
import android.media.session.MediaController
import android.media.session.MediaSessionManager
import android.media.session.PlaybackState
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.provider.Settings
import android.service.notification.NotificationListenerService

/**
 * YouTube position for "Continue on PC" (item 12). Android only lets a notification listener read
 * other apps' media sessions, so this service exists for that alone: it never reads, keeps or
 * forwards notification CONTENT (it only counts unread notifications per app, for the badges on the laptop's chat head). The system binds it once the user allows it (MediaAccessActivity
 * explains why first). Event-driven: session / playback / metadata callbacks; no polling.
 */
class MediaListener : NotificationListenerService() {
    override fun onListenerConnected() { NotifMirror.attach(this); PhoneHub.attach(this); MediaWatch.attach(this); NotifCounts.attach(this) }
    override fun onListenerDisconnected() { NotifMirror.detach(); MediaWatch.detach(); NotifCounts.detach() }
    // "Now playing everywhere": new notifications worth a toast go to the PC (NotifMirror: not Aura's own, not ongoing, not silent)
    override fun onNotificationPosted(sbn: android.service.notification.StatusBarNotification, rankingMap: RankingMap?) { super.onNotificationPosted(sbn, rankingMap); NotifMirror.onPosted(sbn, rankingMap) }
    override fun onNotificationRemoved(sbn: android.service.notification.StatusBarNotification, rankingMap: RankingMap?) { super.onNotificationRemoved(sbn, rankingMap); NotifMirror.onRemoved(sbn) }
    // only a per-app COUNT is kept (badges on the laptop's chat head); no notification content is read
    override fun onNotificationPosted(sbn: android.service.notification.StatusBarNotification?) { NotifCounts.changed() }
    override fun onNotificationRemoved(sbn: android.service.notification.StatusBarNotification?) { NotifCounts.changed() }
}

object MediaWatch {
    private val main = Handler(Looper.getMainLooper())
    private var msm: MediaSessionManager? = null
    private var ctx: Context? = null
    private val controllers = HashMap<MediaController, MediaController.Callback>()
    private val deb = HandoffLogic.Debouncer(5_000, 60_000)
    /** title -> link, learned when a link from the PC (or a share) is followed by that video playing */
    private val known = com.spacez.unlock_stage1.zerotap.LinkCache(50)

    fun granted(c: Context): Boolean {
        val flat = Settings.Secure.getString(c.contentResolver, "enabled_notification_listeners") ?: return false
        return flat.split(':').any { ComponentName.unflattenFromString(it)?.let { n -> n.packageName == c.packageName && n.className.endsWith("MediaListener") } == true }
    }

    fun component(c: Context) = ComponentName(c, MediaListener::class.java)

    private val sessionsChanged = MediaSessionManager.OnActiveSessionsChangedListener { list -> main.post { track(list ?: emptyList()) } }

    fun attach(c: Context) {
        ctx = c.applicationContext
        val m = c.getSystemService(MediaSessionManager::class.java) ?: return
        msm = m
        runCatching {
            m.addOnActiveSessionsChangedListener(sessionsChanged, component(c), main)
            track(m.getActiveSessions(component(c)))
        }.onFailure { HandoffPhone.log("media sessions unavailable: ${it.message}") }
        HandoffPhone.log("media access on (YouTube position for Continue on PC)")
        com.spacez.unlock_stage1.buds.BudsHandoff.start(c)
    }

    fun detach() {
        runCatching { msm?.removeOnActiveSessionsChangedListener(sessionsChanged) }
        controllers.forEach { (c, cb) -> c.unregisterCallback(cb) }; controllers.clear()
        budsCbs.forEach { (c, cb) -> c.unregisterCallback(cb) }; budsCbs.clear()
    }

    private fun isYt(c: MediaController) = c.packageName in HandoffLogic.YOUTUBE_PACKAGES
    /** item 19: also YouTube Music builds and Spotify (their state goes to the PC for zero-tap continue) */
    private fun isWatched(c: MediaController) = isYt(c) || c.packageName in com.spacez.unlock_stage1.zerotap.ContinueRules.YT_MUSIC_APPS || c.packageName == com.spacez.unlock_stage1.zerotap.ContinueRules.SPOTIFY

    /** Galaxy Buds hand-off: ANY app's session counts, the PC is told when the phone starts / stops playing. */
    private val budsCbs = HashMap<MediaController, MediaController.Callback>()

    private fun trackAll(list: List<MediaController>) {
        budsCbs.keys.filter { k -> list.none { it.sessionToken == k.sessionToken } }.forEach { budsCbs.remove(it)?.let(it::unregisterCallback) }
        for (c in list) if (budsCbs.keys.none { it.sessionToken == c.sessionToken }) {
            val cb = object : MediaController.Callback() {
                override fun onPlaybackStateChanged(state: PlaybackState?) { ctx?.let { com.spacez.unlock_stage1.buds.BudsHandoff.report(it, false) }; PhoneHub.changed() }
                override fun onMetadataChanged(metadata: MediaMetadata?) { PhoneHub.changed() }
            }
            c.registerCallback(cb, main); budsCbs[c] = cb
        }
        ctx?.let { com.spacez.unlock_stage1.buds.BudsHandoff.report(it, false) }
        PhoneHub.changed()
    }

    /** Every active media session of any app (Now playing everywhere). */
    fun sessions(): List<MediaController> = all()

    /** Any media session is playing right now (any app). */
    fun anyPlaying(): Boolean = all().any { it.playbackState?.state == PlaybackState.STATE_PLAYING }

    private fun track(list: List<MediaController>) {
        runCatching { trackAll(list) }
        val yt = list.filter(::isWatched)
        controllers.keys.filter { k -> yt.none { it.sessionToken == k.sessionToken } }.forEach { controllers.remove(it)?.let(it::unregisterCallback) }
        for (c in yt) if (controllers.keys.none { it.sessionToken == c.sessionToken }) {
            val cb = object : MediaController.Callback() {
                override fun onPlaybackStateChanged(state: PlaybackState?) = changed(c)
                override fun onMetadataChanged(metadata: MediaMetadata?) = changed(c)
            }
            c.registerCallback(cb, main); controllers[c] = cb
            changed(c)
        }
    }

    /** The YouTube session that is playing (or the most recent one). */
    private fun current(): MediaController? {
        val m = msm; val c = ctx
        val live = if (m != null && c != null) runCatching { m.getActiveSessions(component(c)) }.getOrDefault(emptyList()).filter(::isYt) else controllers.keys.toList()
        return live.firstOrNull { it.playbackState?.state == PlaybackState.STATE_PLAYING } ?: live.firstOrNull()
    }

    data class Now(val pkg: String, val title: String, val posMs: Long?, val playing: Boolean, val url: String?)

    fun now(): Now? {
        val c = current() ?: return null
        val st = c.playbackState
        val playing = st?.state == PlaybackState.STATE_PLAYING
        val pos = st?.let {
            if (it.position < 0) null
            else if (playing) it.position + ((SystemClock.elapsedRealtime() - it.lastPositionUpdateTime) * it.playbackSpeed).toLong()
            else it.position
        }
        val md = c.metadata
        val title = md?.getString(MediaMetadata.METADATA_KEY_TITLE) ?: ""
        return Now(c.packageName, title, pos, playing, linkFor(title, md))
    }

    /** Channel + length of the current session, the rest of the identity the PC needs to find a video that has no link. */
    fun identity(): Pair<String?, Long?>? {
        val md = current()?.metadata ?: return null
        return md.getString(MediaMetadata.METADATA_KEY_ARTIST) to md.getLong(MediaMetadata.METADATA_KEY_DURATION).takeIf { it > 0 }
    }

    /** Pause the phone video after it moved to the PC (Apple-style hand-over). */
    fun pause() { runCatching { current()?.transportControls?.pause() } }

    private fun all(): List<MediaController> {
        val m = msm; val c = ctx
        return if (m != null && c != null) runCatching { m.getActiveSessions(component(c)) }.getOrDefault(emptyList()) else controllers.keys.toList()
    }

    private fun titleOf(c: MediaController) = c.metadata?.getString(MediaMetadata.METADATA_KEY_TITLE) ?: ""

    /** Item 19: the session whose title matches, else the only one playing (two playing: none, never hijack). */
    private fun matching(title: String?, wantPlaying: Boolean): MediaController? {
        val l = all()
        if (!title.isNullOrBlank()) l.firstOrNull { com.spacez.unlock_stage1.zerotap.ContinueRules.sameTitle(titleOf(it), title) }?.let { return it }
        if (!wantPlaying) return null
        return l.filter { it.playbackState?.state == PlaybackState.STATE_PLAYING }.singleOrNull()
    }

    /** Pause (or with [mute], pause too: Android has no per-app mute for another app) what now continues on the PC. */
    fun pauseMatching(title: String?, @Suppress("UNUSED_PARAMETER") mute: Boolean): Boolean {
        val c = matching(title, true) ?: return false
        if (c.playbackState?.state != PlaybackState.STATE_PLAYING) return false
        return runCatching { c.transportControls.pause(); true }.getOrDefault(false)
    }

    /** Undo: resume it here. */
    fun playMatching(title: String?): Boolean {
        val c = matching(title, false) ?: current() ?: return false
        return runCatching { c.transportControls.play(); true }.getOrDefault(false)
    }

    private fun linkFor(title: String, md: MediaMetadata?): String? {
        // 1. a video id in the metadata (some YouTube builds expose it)
        for (k in listOf(MediaMetadata.METADATA_KEY_MEDIA_ID, MediaMetadata.METADATA_KEY_MEDIA_URI)) {
            val v = md?.getString(k) ?: continue
            if (Regex("^[A-Za-z0-9_-]{11}$").matches(v)) return "https://www.youtube.com/watch?v=$v"
            if (HandoffLogic.isYouTube(v)) return v
        }
        // 2. learned: the video that started right after a link from the PC / a share
        // only an entry whose title (and channel) match the CURRENT media session. The old "the video that
        // started within 60 s of a link we opened" guess is gone: it attached stale links to other videos.
        return known.get(title, md?.getString(MediaMetadata.METADATA_KEY_ARTIST))
    }

    fun remember(title: String, url: String, channel: String? = null) {
        if (title.isBlank()) return
        known.put(title, channel, url.substringBefore("&t=").substringBefore("?t="))
    }

    private val tick = Runnable { flush() }

    @Volatile private var startedAtMs = 0L
    @Volatile private var lastPlayingKey: String? = null
    /** When the media that plays now started (wall clock), for the "owner started something" guard. */
    fun startedAt() = startedAtMs

    /** Seek the playing YouTube session (the same video was already open here). */
    fun seekTo(posMs: Long?) { if (posMs != null) runCatching { current()?.transportControls?.apply { seekTo(posMs); play() } } }

    private fun nowOf(c: MediaController): Now {
        val st = c.playbackState
        val playing = st?.state == PlaybackState.STATE_PLAYING
        val pos = st?.let { if (it.position < 0) null else if (playing) it.position + ((SystemClock.elapsedRealtime() - it.lastPositionUpdateTime) * it.playbackSpeed).toLong() else it.position }
        val md = c.metadata
        val title = md?.getString(MediaMetadata.METADATA_KEY_TITLE) ?: ""
        return Now(c.packageName, title, pos, playing, if (isYt(c)) linkFor(title, md) else md?.getString(MediaMetadata.METADATA_KEY_MEDIA_ID)?.takeIf { it.startsWith("spotify:") }?.let { "https://open.spotify.com/" + it.removePrefix("spotify:").replace(':', '/') })
    }

    /** The PC asked (item 19 fix): push the current YouTube state again, even if unchanged. */
    fun reportNow() {
        main.post { runCatching { current()?.let { com.spacez.unlock_stage1.zerotap.ZeroTap.forceNext(); changed(it) } } }
        // item 26: the PC asks what the phone has, so it also gets the whole recent list
        ctx?.let { com.spacez.unlock_stage1.handoff.RecentPush.pushNow(it) }
    }

    private fun changed(c: MediaController) {
        // item 19: every play / pause / seek / new item -> the PC's zero-tap state (on change only)
        runCatching {
            val n = nowOf(c)
            val key = if (n.playing) "${n.pkg}|${n.title}" else null
            if (key != null && key != lastPlayingKey) startedAtMs = System.currentTimeMillis()
            lastPlayingKey = key
            val md = c.metadata
            ctx?.let { cx -> com.spacez.unlock_stage1.zerotap.ZeroTap.onMedia(cx, n, md?.getString(MediaMetadata.METADATA_KEY_ARTIST), md?.getLong(MediaMetadata.METADATA_KEY_DURATION)?.takeIf { it > 0 }) }
        }
        if (!isYt(c)) return
        val n = now()
        // a video that plays and that we can link to: candidate for the passive suggestion
        deb.observe(if (n != null && n.playing && n.url != null) n.url else null, System.currentTimeMillis())
        main.removeCallbacks(tick)
        deb.nextWake()?.let { main.postDelayed(tick, maxOf(50, it - System.currentTimeMillis())) }
    }

    private fun flush() {
        val c = ctx ?: return
        if (deb.due(System.currentTimeMillis()) == null) { deb.nextWake()?.let { main.postDelayed(tick, maxOf(50, it - System.currentTimeMillis())) }; return }
        val n = now() ?: return
        val url = n.url ?: return
        Thread { HandoffPhone.suggestToPc(c, HandoffLogic.Suggestion("youtube", n.title, url, n.posMs, n.playing, System.currentTimeMillis())) }.start()
    }
}
