package com.spacez.unlock_stage1.handoff

/**
 * Item 24 pure parts (no Android types, JVM unit-tested in LinkLogicTest): which links have an app,
 * app-vs-browser choice, the "More from your PC" rows of the notification / bubble.
 */
object LinkLogic {
    enum class Mode { APP, BROWSER;
        companion object { fun parse(s: String?): Mode? = when (s?.lowercase()) { "app" -> APP; "browser" -> BROWSER; else -> null } }
        val wire get() = name.lowercase()
    }

    /** A site that may have an installed app. No package names: Android picks the handler the user chose. */
    data class App(val key: String, val label: String, val hosts: List<String>)

    val APPS = listOf(
        App("youtube", "YouTube", listOf("youtube.com", "youtu.be")),
        App("linkedin", "LinkedIn", listOf("linkedin.com", "lnkd.in")),
        App("facebook", "Facebook", listOf("facebook.com", "fb.com", "fb.watch")),
        App("instagram", "Instagram", listOf("instagram.com")),
        App("x", "X", listOf("x.com", "twitter.com")),
        App("reddit", "Reddit", listOf("reddit.com", "redd.it")),
        App("spotify", "Spotify", listOf("spotify.com", "spotify.link")),
        App("github", "GitHub", listOf("github.com")),
        App("whatsapp", "WhatsApp", listOf("wa.me", "whatsapp.com")),
    )

    /** The app a link belongs to, matching the host and its subdomains (m.youtube.com, music.youtube.com...). */
    fun appFor(url: String): App? {
        val h = HandoffLogic.host(url).substringBefore(':')
        return APPS.firstOrNull { a -> a.hosts.any { h == it || h.endsWith(".$it") } }
    }

    /** Default for every app: open in the installed app (falls to the browser by itself when there is none). */
    val DEFAULT_MODE = Mode.APP

    /** [override] (a long-press choice) wins; else the per-app setting; else the default. Sites without an app: browser. */
    fun decide(url: String, perApp: Map<String, Mode>, override: Mode? = null): Mode {
        if (override != null) return override
        val app = appFor(url) ?: return Mode.BROWSER
        return perApp[app.key] ?: DEFAULT_MODE
    }

    /** An installed app that can open a youtube.com/watch link. */
    data class Handler(val pkg: String, val label: String, val browser: Boolean)

    /**
     * Which app opens YouTube links: the owner's pick ([chosen], if it is still a candidate), else the system default when
     * it is a real app (a browser as "default" is the very friction we fix: Android has none for youtube.com and falls to
     * the browser), else the first non-browser app (Morphe / YouTube first), else null (a plain VIEW, the browser).
     */
    fun pickYouTube(cands: List<Handler>, systemDefault: String?, chosen: String?): String? {
        if (!chosen.isNullOrEmpty() && cands.any { it.pkg == chosen }) return chosen
        val apps = cands.filter { !it.browser }
        if (systemDefault != null && apps.any { it.pkg == systemDefault }) return systemDefault
        val rank = HandoffLogic.YOUTUBE_PACKAGES
        return apps.sortedBy { rank.indexOf(it.pkg).let { i -> if (i < 0) rank.size else i } }.firstOrNull()?.pkg
    }

    /** Dedupe key: the YouTube video, else the link without its fragment and trailing slash. */
    fun sameKey(url: String): String = HandoffLogic.videoId(url)?.let { "yt:$it" } ?: url.substringBefore('#').trimEnd('/')

    /** "More from your PC": [recent] without the main item and duplicates, newest first as given, at most [max]. */
    fun moreRows(main: HandoffLogic.Suggestion?, recent: List<HandoffLogic.Suggestion>, max: Int = 4): List<HandoffLogic.Suggestion> {
        val seen = HashSet<String>()
        main?.let { seen.add(sameKey(it.url)) }
        val out = ArrayList<HandoffLogic.Suggestion>()
        for (r in recent) {
            if (out.size >= max) break
            if (seen.add(sameKey(r.url))) out.add(r)
        }
        return out
    }

    /** How long a bubble stays up, and whether one may be shown for a suggestion pushed at [pushedMs]. */
    const val BUBBLE_TTL_MS = 2 * 60_000L

    fun bubbleWanted(enabled: Boolean, suggestionsOn: Boolean, allowedBySystem: Boolean) = enabled && suggestionsOn && allowedBySystem

    /** True while a bubble pushed at [shownAt] should still exist (the notification timeout enforces it; this is for the activity). */
    fun bubbleAlive(shownAt: Long, nowMs: Long) = nowMs >= shownAt && nowMs - shownAt < BUBBLE_TTL_MS
}
