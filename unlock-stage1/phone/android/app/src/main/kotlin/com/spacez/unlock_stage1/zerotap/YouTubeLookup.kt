package com.spacez.unlock_stage1.zerotap

import java.net.HttpURLConnection
import java.net.URL
import java.net.URLEncoder
import kotlin.concurrent.thread

/**
 * The YouTube app's media session has the title and channel but not the video id (item 19).
 * When the link is not known from a share / a link the PC sent, look it up once: YouTube's public
 * search page (no API key), then a CONFIDENCE CHECK with YouTube's oEmbed answer for the candidate
 * (exact title, and the channel when known). Unsure = no link (the PC then offers nothing, never a
 * wrong video). One request pair per new video, results kept in memory (no storage, no logging).
 */
/** Title(+channel) -> link. A link is returned only for the SAME title, and the same channel when both are known. Pure, unit-tested. */
class LinkCache(private val max: Int) {
    private data class E(val title: String, val channel: String?, val url: String)
    private val list = ArrayList<E>()
    private fun t(s: String) = s.trim().lowercase()
    private fun c(s: String?) = s?.trim()?.lowercase()?.ifEmpty { null }
    @Synchronized fun get(title: String, channel: String?): String? {
        val ct = c(channel)
        return list.firstOrNull { t(it.title) == t(title) && (it.channel == null || ct == null || it.channel == ct) }?.url
    }
    @Synchronized fun put(title: String, channel: String?, url: String) {
        val ct = c(channel)
        list.removeAll { t(it.title) == t(title) && (it.channel == ct || it.channel == null || ct == null) }
        list.add(0, E(title, ct, url))
        while (list.size > max) list.removeAt(list.size - 1)
    }
}

object YouTubeLookup {
    /** the whole title lookup (search + oEmbed checks) must finish in this long, else "unsure" */
    const val BUDGET_MS = 2_000L
    private val cache = object : LinkedHashMap<String, String?>(64, 0.75f, true) {
        override fun removeEldestEntry(eldest: MutableMap.MutableEntry<String, String?>?) = size > 64
    }
    private val pending = HashSet<String>()
    /** the same title with another length is another video, so the length is part of the key */
    private fun key(t: String, ch: String?, durMs: Long?) = "${t.trim().lowercase()}|${ch?.trim()?.lowercase()}|${(durMs ?: 0) / 1000}"

    fun cached(title: String, channel: String?, durMs: Long?): String? = synchronized(cache) { cache[key(title, channel, durMs)] }

    fun resolveAsync(title: String, channel: String?, durMs: Long?, done: (String?) -> Unit) {
        val k = key(title, channel, durMs)
        synchronized(cache) { if (cache.containsKey(k) || !pending.add(k)) return }
        thread(isDaemon = true, name = "yt-lookup") {
            val r = runCatching { resolve(title, channel, durMs) }
            // a timeout / network error is not an answer: do not cache it, a later change retries
            synchronized(cache) { if (r.isSuccess) cache[k] = r.getOrNull(); pending.remove(k) }
            val u = r.getOrNull()
            if (u != null) done(u)
        }
    }

    private fun get(url: String, deadline: Long): String {
        val left = (deadline - System.currentTimeMillis()).toInt()
        if (left <= 100) throw java.net.SocketTimeoutException("lookup budget used")
        val c = URL(url).openConnection() as HttpURLConnection
        c.connectTimeout = left; c.readTimeout = left
        c.setRequestProperty("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36")
        c.setRequestProperty("Accept-Language", "en")
        return c.inputStream.bufferedReader().use { it.readText() }.also { c.disconnect() }
    }

    fun resolve(title: String, channel: String?, durMs: Long?): String? {
        val q = URLEncoder.encode(listOfNotNull(title, channel).joinToString(" "), "UTF-8")
        val deadline = System.currentTimeMillis() + BUDGET_MS
        val page = get("https://www.youtube.com/results?search_query=$q", deadline)
        for ((id, len) in candidatesWithLen(page).take(8)) {
            if (!durationMatches(durMs, len)) continue // same title, other length: another video
            val o = runCatching { get("https://www.youtube.com/oembed?format=json&url=" + URLEncoder.encode("https://www.youtube.com/watch?v=$id", "UTF-8"), deadline) }.getOrNull() ?: continue
            if (confident(o, title, channel)) return "https://www.youtube.com/watch?v=$id"
        }
        return null
    }

    /** Video ids in a search results page, in order, no duplicates. Pure, unit-tested. */
    fun candidates(page: String): List<String> =
        Regex("\"videoId\":\"([A-Za-z0-9_-]{11})\"").findAll(page).map { it.groupValues[1] }.distinct().toList()
            // the mobile page (m.youtube.com) escapes its JSON as backslash-x22
            .ifEmpty { Regex("""\\x22videoId\\x22:\\x22([A-Za-z0-9_-]{11})""").findAll(page).map { it.groupValues[1] }.distinct().toList() }

    const val DURATION_TOL_S = 3L

    /** "12:34" / "1:02:03" -> seconds. */
    fun parseClock(s: String): Long? {
        val p = s.trim().split(':')
        if (p.size !in 2..3) return null
        return p.fold(0L) { acc, x -> acc * 60 + (x.trim().toLongOrNull()?.takeIf { x.isNotBlank() } ?: return null) }
    }

    /** Video ids with the length (s) the search page shows for each (null for live / unknown). Pure, unit-tested. */
    fun candidatesWithLen(page: String): List<Pair<String, Long?>> {
        val ids = Regex("\"videoId\":\"([A-Za-z0-9_-]{11})\"").findAll(page).toList()
        val out = LinkedHashMap<String, Long?>()
        for ((n, m) in ids.withIndex()) {
            val end = minOf(ids.getOrNull(n + 1)?.range?.first ?: page.length, m.range.last + 6000)
            val seg = page.substring(m.range.last, end)
            val i = seg.indexOf("\"lengthText\":")
            val len = if (i < 0) null else Regex("\"simpleText\":\"([^\"]*)\"").find(seg, i)?.let { parseClock(it.groupValues[1]) }
            val id = m.groupValues[1]
            if (!out.containsKey(id) || out[id] == null) out[id] = len
        }
        return out.map { it.key to it.value }
    }

    /** Phone session duration vs candidate length: +-3 s. Phone unknown = nothing to compare; candidate unknown = no. */
    fun durationMatches(phoneMs: Long?, candS: Long?, tolS: Long = DURATION_TOL_S): Boolean {
        val p = phoneMs?.takeIf { it > 0 } ?: return true
        val c = candS ?: return false
        return Math.abs(p / 1000 - c) <= tolS
    }

    /** oEmbed JSON says the same title (and channel when known). Pure, unit-tested. */
    fun confident(oembed: String, title: String, channel: String?): Boolean {
        val m = com.spacez.unlock_stage1.handoff.HandoffLogic.parseFlat(oembed)
        val t = (m["title"] as? String)?.trim() ?: return false
        val a = (m["author_name"] as? String)?.trim()
        if (!t.equals(title.trim(), ignoreCase = true)) return false
        return channel.isNullOrBlank() || a == null || a.equals(channel.trim(), ignoreCase = true)
    }
}
