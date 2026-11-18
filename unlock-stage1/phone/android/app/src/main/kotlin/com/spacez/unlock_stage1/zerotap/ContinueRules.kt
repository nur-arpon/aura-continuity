package com.spacez.unlock_stage1.zerotap

import com.spacez.unlock_stage1.handoff.HandoffLogic

/**
 * Zero-tap continue (item 19): every decision rule, pure (no Android types) and unit-tested in
 * app/src/test/.../ContinueRulesTest.kt (one test per rule, recorded scenarios). The rules are
 * documented for the owner in aura-anywhere/CONTINUE-RULES.md; the PC half (phone -> PC) is the same
 * rule set in Rust, handoff/src-tauri/src/core/zerotap.rs.
 */
object ContinueRules {
    /** After continuing, the source device: pause (default) / mute / leave playing. */
    val AFTER = setOf("pause", "mute", "leave")
    /** The source must have played (or been in front) within this window. */
    const val WINDOW_MS = 120_000L
    /** The owner started something else himself on the target within this time: hands off. */
    const val OWNER_GUARD_MS = 3_000L
    /** Clips shorter than this are not worth carrying over. */
    const val MIN_CLIP_MS = 30_000L
    /** How long the "Undo / Pick another" chip stays. */
    const val CHIP_MS = 2_000L

    // ------------------------------------------------------------------ packages

    val YT_VIDEO_APPS = listOf("app.morphe.android.youtube", "com.google.android.youtube")
    val YT_MUSIC_APPS = listOf("app.morphe.android.apps.youtube.music", "app.rvx.android.apps.youtube.music", "com.google.android.apps.youtube.music")
    val BROWSERS = listOf("com.sec.android.app.sbrowser", "com.sec.android.app.sbrowser.beta", "com.brave.browser", "com.android.chrome")
    const val FACEBOOK = "com.facebook.katana"
    const val FACEBOOK_LITE = "com.facebook.lite"
    const val MESSENGER = "com.facebook.orca"
    const val WHATSAPP = "com.whatsapp"
    const val CLAUDE = "com.anthropic.claude"
    const val SPOTIFY = "com.spotify.music"

    /** The only packages the accessibility service listens to (res/xml/aura_accessibility.xml mirrors it). */
    val WATCHED = YT_VIDEO_APPS + YT_MUSIC_APPS + BROWSERS + listOf(FACEBOOK, FACEBOOK_LITE, MESSENGER, WHATSAPP, CLAUDE, SPOTIFY)

    /** Which kind of PC item an app on the phone continues ("youtube", "music", "web", "facebook", ...). */
    fun appKind(pkg: String): String? = when (pkg) {
        in YT_VIDEO_APPS -> "youtube"
        in YT_MUSIC_APPS -> "music"
        in BROWSERS -> "web"
        FACEBOOK, FACEBOOK_LITE -> "facebook"
        MESSENGER -> "messenger"
        WHATSAPP -> "whatsapp"
        CLAUDE -> "claude"
        SPOTIFY -> "spotify"
        else -> null
    }

    /** The kind of a page / item by its URL (the PC side knows only URLs). */
    fun urlKind(url: String): String {
        val h = HandoffLogic.host(url).removePrefix("www.").removePrefix("m.")
        return when {
            h == "music.youtube.com" -> "music"
            HandoffLogic.isYouTube(url) -> "youtube"
            h == "facebook.com" || h == "web.facebook.com" || h == "fb.watch" -> "facebook"
            h == "messenger.com" || h == "m.me" -> "messenger"
            h == "web.whatsapp.com" || h == "wa.me" -> "whatsapp"
            h == "claude.ai" -> "claude"
            h == "open.spotify.com" -> "spotify"
            else -> "web"
        }
    }

    /** A YouTube home / feed / search page: never something to continue. */
    fun isFeedPage(url: String): Boolean {
        val h = HandoffLogic.host(url).removePrefix("www.").removePrefix("m.")
        return (h == "youtube.com" || h == "music.youtube.com") && HandoffLogic.videoId(url) == null && !url.contains("list=")
    }

    // ------------------------------------------------------------------ items

    /** Something playing or open on one device. Times are on THIS phone's clock. */
    data class Item(
        val app: String,
        val title: String,
        val url: String?,
        val posMs: Long? = null,
        val playing: Boolean = false,
        /** producing sound (not muted, volume > 0) */
        val audible: Boolean = playing,
        /** when it last started playing */
        val startedMs: Long = 0,
        /** last focus / seek / pause / play */
        val touchedMs: Long = 0,
        val durationMs: Long? = null,
        val ad: Boolean = false,
        val live: Boolean = false,
        val private: Boolean = false,
        /** where it is: phone package, or "browser.exe|profile" on the PC */
        val where: String = "",
    ) {
        fun key(): String = url?.let { HandoffLogic.videoId(it)?.let { id -> "yt:$id" } ?: it.substringBefore('#').trimEnd('/').lowercase() } ?: "title:${title.lowercase()}"
        /** When it was last in use: now while playing, else the last touch. */
        fun lastActive(now: Long) = if (playing) now else touchedMs
    }

    /**
     * Rules "several things playing" and "nothing playing, several paused": what playing and audible
     * and most recently started wins; else the one last interacted with. Private items never count.
     */
    fun pickSource(items: List<Item>): Item? {
        val ok = items.filter { !it.private && it.url != null && !isFeedPage(it.url) }
        val playing = ok.filter { it.playing }
        if (playing.isNotEmpty()) return playing.sortedWith(compareByDescending<Item> { it.audible }.thenByDescending { it.startedMs }).first()
        return ok.maxByOrNull { it.touchedMs }
    }

    sealed class Decision {
        data class Go(val item: Item, val url: String, val seekOnly: Boolean) : Decision()
        data class Wait(val why: String, val retryMs: Long) : Decision()
        data class Skip(val why: String) : Decision()
    }

    /**
     * Should the [source] (other device) continue on this device now?
     * [targetNow] = what plays on the target device (null = nothing), [ownerStartedMs] = when the owner
     * started something on the target himself (0 = no), [targetOpen] = the item already open on the target.
     */
    fun decide(source: Item?, targetNow: Item?, ownerStartedMs: Long, targetOpen: Item?, now: Long): Decision {
        val s = source ?: return Decision.Skip("nothing to continue")
        val url = s.url ?: return Decision.Skip("no link for it")
        if (s.private) return Decision.Skip("private window")
        if (isFeedPage(url)) return Decision.Skip("a home / feed page")
        if (now - s.lastActive(now) > WINDOW_MS) return Decision.Skip("not used in the last 2 minutes")
        if (ownerStartedMs > 0 && now - ownerStartedMs in 0..OWNER_GUARD_MS && (targetNow == null || targetNow.key() != s.key()))
            return Decision.Skip("the owner started something else")
        if (targetNow != null && targetNow.playing && targetNow.key() != s.key() && s.playing)
            return Decision.Skip("both devices are playing different things")
        val d = s.durationMs
        if (d != null && d in 1 until MIN_CLIP_MS) return Decision.Skip("clip shorter than 30 s")
        if (s.ad) return Decision.Wait("an ad is playing", 1_000)
        val same = targetOpen != null && targetOpen.key() == s.key()
        return Decision.Go(s, resumeUrl(s, now), same)
    }

    /**
     * The link to open: YouTube at the exact second now (playing = extrapolated); Shorts as Shorts;
     * live streams at the live edge (no t); playlists keep list= and index=; other links unchanged.
     */
    fun resumeUrl(s: Item, now: Long): String {
        val url = s.url ?: return ""
        val id = HandoffLogic.videoId(url) ?: return url
        val path = url.substringAfter("://").substringAfter('/', "").substringBefore('?')
        if (path.startsWith("shorts/")) return "https://www.youtube.com/shorts/$id"
        val q = url.substringBefore('#').substringAfter('?', "").split('&').filter { it.startsWith("list=") || it.startsWith("index=") }
        val host = if (HandoffLogic.host(url).contains("music.youtube.com")) "music.youtube.com" else "www.youtube.com"
        val base = "https://$host/watch?" + (listOf("v=$id") + q).joinToString("&")
        if (s.live || path.startsWith("live/")) return base
        return HandoffLogic.withTime(base, positionNow(s, now))
    }

    fun positionNow(s: Item, now: Long): Long? = HandoffLogic.positionNow(s.posMs, s.playing, s.touchedMs.takeIf { it > 0 } ?: s.startedMs, now)

    /**
     * Rule "several YouTube apps on the phone": music -> the YT Music build used last; videos -> the
     * YouTube app used last; default the Morphe build. Pages -> null (the default browser).
     */
    fun phoneApp(url: String, installed: Collection<String>, lastUsed: Map<String, Long>): String? {
        fun best(cands: List<String>) = cands.filter { it in installed }.let { l -> l.maxByOrNull { lastUsed[it] ?: -1L }?.takeIf { (lastUsed[it] ?: 0L) > 0 } ?: l.firstOrNull() }
        return when (urlKind(url)) {
            "music" -> best(YT_MUSIC_APPS) ?: best(YT_VIDEO_APPS)
            "youtube" -> best(YT_VIDEO_APPS)
            "facebook" -> best(listOf(FACEBOOK, FACEBOOK_LITE))
            "messenger" -> MESSENGER.takeIf { it in installed }
            "claude" -> CLAUDE.takeIf { it in installed }
            "whatsapp" -> WHATSAPP.takeIf { it in installed }
            "spotify" -> SPOTIFY.takeIf { it in installed }
            else -> null
        }
    }

    /** The link as the phone app wants it (Messenger threads open from m.me links). */
    fun phoneLink(url: String): String {
        if (urlKind(url) == "messenger") {
            val t = url.substringAfter("/t/", "").substringBefore('/').substringBefore('?')
            if (t.isNotEmpty()) return "https://m.me/$t"
        }
        return url
    }

    /** Which PC items may continue in the app that just came to the front ([pkg]). */
    fun fits(pkg: String, item: Item): Boolean {
        val k = appKind(pkg) ?: return false
        val ik = item.url?.let(::urlKind) ?: item.app
        return when (k) {
            "web" -> ik == "web" // a browser continues pages; videos go to the YouTube app
            "youtube" -> ik == "youtube"
            "music" -> ik == "music" || ik == "youtube"
            else -> ik == k
        }
    }

    /**
     * A PC item (times already on this phone's clock) as a rules item. "Last used" on the PC: the
     * read time for what plays (its position is extrapolated from it), else the PC's last keyboard /
     * mouse input, so a page the owner left an hour ago is not "just used".
     */
    fun fromPc(s: HandoffLogic.Suggestion, idleMs: Long?, nowMs: Long): Item {
        val touched = if (s.playing || idleMs == null || idleMs < 0) s.atMs else nowMs - idleMs
        return Item(app = urlKind(s.url), title = s.title, url = s.url, posMs = s.posMs, playing = s.playing, startedMs = s.atMs, touchedMs = touched)
    }

    /** "The Emperor | Slugterra - YouTube" vs "The Emperor | Slugterra": same; a bare "YouTube" never matches. */
    fun sameTitle(a: String?, b: String?): Boolean {
        fun n(x: String) = x.trim().removeSuffix(" - YouTube").removeSuffix(" - YouTube Music").trim().lowercase()
        if (a.isNullOrBlank() || b.isNullOrBlank()) return false
        val x = n(a); val y = n(b)
        if (x.isEmpty() || y.isEmpty() || x == "youtube" || y == "youtube") return false
        return x == y || (minOf(x.length, y.length) >= 8 && (x.contains(y) || y.contains(x)))
    }
}
