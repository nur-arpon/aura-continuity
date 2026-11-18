package com.spacez.unlock_stage1.handoff

/**
 * Item 26 pure parts (no Android types, JVM unit-tested in RecentLogicTest): the phone's "recent" list that is
 * pushed to the PC ("More from your phone"). Wire shape = ACTIVITY JSON
 * `{"state":true,"recent_list":true,"at_ms","sent_ms","recent":[{app,title,url?,channel?,pos_ms?,duration_ms?,playing,at_ms},...]}`
 * newest first; every entry has the shape of a single state report, so the PC parses them the same way.
 */
object RecentLogic {
    const val MAX = 8

    data class Entry(
        val app: String, val title: String, val url: String?, val channel: String?,
        val posMs: Long?, val durationMs: Long?, val playing: Boolean, val atMs: Long,
    ) {
        /** Same link, else same app + title. */
        fun key() = url?.substringBefore('#')?.substringBefore("&t=")?.substringBefore("?t=") ?: "$app:${title.lowercase()}"

        fun toJson() = HandoffLogic.json(linkedMapOf<String, Any?>(
            "app" to app, "title" to title, "url" to url, "channel" to channel, "pos_ms" to posMs,
            "duration_ms" to durationMs, "playing" to playing, "at_ms" to atMs,
        ).filterValues { it != null })

        companion object {
            fun parse(s: String): Entry? {
                val m = HandoffLogic.parseFlat(s)
                val app = m["app"] as? String ?: return null
                val title = ((m["title"] as? String) ?: "").take(200)
                val url = (m["url"] as? String)?.takeIf { it.startsWith("https://") || it.startsWith("http://") }
                return Entry(app, title, url, m["channel"] as? String, (m["pos_ms"] as? Number)?.toLong(),
                    (m["duration_ms"] as? Number)?.toLong(), m["playing"] == true, (m["at_ms"] as? Number)?.toLong() ?: 0)
            }
        }
    }

    /** What the PC can open: a link, or a video / music title it can resolve by title + channel + length. */
    fun usable(e: Entry): Boolean = e.title.isNotBlank() && (e.url != null || e.app == "youtube" || e.app == "music")

    /** A state report from the phone (the same map ZeroTap pushes) as a list entry, or null when it is not worth listing. */
    fun fromState(m: Map<String, Any?>, atMs: Long): Entry? {
        val app = m["app"] as? String ?: return null
        val e = Entry(app, ((m["title"] as? String) ?: "").take(200), (m["url"] as? String)?.takeIf { it.startsWith("http") },
            m["channel"] as? String, (m["pos_ms"] as? Number)?.toLong(), (m["duration_ms"] as? Number)?.toLong(), m["playing"] == true, atMs)
        return e.takeIf(::usable)
    }

    /** Newest-first list, de-duplicated, capped; reports whether the *visible* list changed. */
    class Ring(initial: List<Entry> = emptyList()) {
        private val items = ArrayList<Entry>(initial.filter(::usable).take(MAX))
        fun list(): List<Entry> = items.toList()

        /** Adds / refreshes [e] (moves it to the front). True when the list a person sees changed (order or titles), not when only a position moved. */
        fun note(e: Entry): Boolean {
            if (!usable(e)) return false
            val before = signature(items)
            items.removeAll { it.key() == e.key() || (it.app == e.app && it.title.equals(e.title, true)) }
            items.add(0, e)
            while (items.size > MAX) items.removeAt(items.size - 1)
            return signature(items) != before
        }
    }

    /** Changes only when an entry appears, goes or reorders (not for position / play state). */
    fun signature(l: List<Entry>): String = l.joinToString("\n") { "${it.app}|${it.title}|${it.url ?: ""}" }

    fun toJson(l: List<Entry>, nowMs: Long): String =
        "{\"state\":true,\"recent_list\":true,\"at_ms\":$nowMs,\"sent_ms\":$nowMs,\"recent\":[" + l.joinToString(",") { it.toJson() } + "]}"

    /** Persistence: one entry per line (the JSON never holds a raw newline). */
    fun serialize(l: List<Entry>): String = l.joinToString("\n") { it.toJson() }
    fun deserialize(s: String?): List<Entry> = s.orEmpty().lines().mapNotNull { if (it.isBlank()) null else Entry.parse(it) }
}
