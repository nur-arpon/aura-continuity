package com.spacez.unlock_stage1.handoff

/**
 * Aura Handoff, phone side: the pure parts (no Android types, JVM unit-tested in
 * app/src/test/.../HandoffLogicTest.kt). Mirrors handoff/src-tauri/src/core/handoff.rs.
 */
object HandoffLogic {
    val YOUTUBE_PACKAGES = listOf("app.morphe.android.youtube", "com.google.android.youtube")

    private val ID = Regex("^[A-Za-z0-9_-]{11}$")

    /** "812 KB", "4.2 MB" for the inbox. */
    fun size(bytes: Long): String = when {
        bytes < 1024 -> "$bytes B"
        bytes < 1024 * 1024 -> "${(bytes + 512) / 1024} KB"
        bytes < 1024L * 1024 * 1024 -> "%.1f MB".format(java.util.Locale.ROOT, bytes / 1048576.0)
        else -> "%.2f GB".format(java.util.Locale.ROOT, bytes / 1073741824.0)
    }

    fun host(url: String): String = url.substringAfter("://", url).substringBefore('/').substringBefore('?').substringBefore('#').lowercase()

    /** The YouTube video id of a watch / youtu.be / shorts / live link, else null. */
    fun videoId(url: String): String? {
        if (!url.startsWith("https://") && !url.startsWith("http://")) return null
        val h = host(url).removePrefix("www.").removePrefix("m.")
        val path = url.substringAfter("://").substringAfter('/', "").substringBefore('#')
        val p = path.substringBefore('?')
        val q = path.substringAfter('?', "")
        val id = when (h) {
            "youtu.be" -> p.trimEnd('/')
            "youtube.com", "music.youtube.com" -> when {
                p == "watch" -> q.split('&').firstOrNull { it.startsWith("v=") }?.removePrefix("v=")
                p.startsWith("shorts/") -> p.removePrefix("shorts/").trimEnd('/')
                p.startsWith("live/") -> p.removePrefix("live/").trimEnd('/')
                else -> null
            }
            else -> null
        }
        return id?.takeIf { ID.matches(it) }
    }

    fun isYouTube(url: String) = videoId(url) != null

    /** `t=` seconds from a link ("90", "90s", "1m30s"), else null. */
    fun tParam(url: String): Long? {
        val q = url.substringBefore('#').substringAfter('?', "")
        val v = q.split('&').firstOrNull { it.startsWith("t=") || it.startsWith("start=") }?.substringAfter('=') ?: return null
        v.toLongOrNull()?.let { return it }
        var total = 0L; var num = ""
        for (c in v) {
            if (c.isDigit()) num += c else {
                val n = num.toLongOrNull() ?: return null; num = ""
                total += when (c) { 'h' -> n * 3600; 'm' -> n * 60; 's' -> n; else -> return null }
            }
        }
        return if (num.isEmpty()) total else null
    }

    /** Link with `t=<s>s` for YouTube (replacing any old one); other links unchanged. Under 5 s = from the top. */
    fun withTime(url: String, posMs: Long?): String {
        if (posMs == null || !isYouTube(url)) return url
        val (base, frag) = url.split('#', limit = 2).let { it[0] to it.getOrNull(1) }
        val path = base.substringBefore('?')
        val keep = base.substringAfter('?', "").split('&').filter { it.isNotEmpty() && !it.startsWith("t=") && !it.startsWith("start=") }
        val s = posMs / 1000
        val parts = if (s >= 5) keep + "t=${s}s" else keep
        var out = if (parts.isEmpty()) path else path + "?" + parts.joinToString("&")
        if (frag != null) out += "#$frag"
        return out
    }

    /** A playing video kept going since [atMs]; ignore clock skew or absurd gaps. */
    fun positionNow(posMs: Long?, playing: Boolean, atMs: Long, nowMs: Long): Long? {
        if (posMs == null) return null
        if (!playing || atMs <= 0 || nowMs < atMs || nowMs - atMs > 6 * 3600_000L) return posMs
        return posMs + (nowMs - atMs)
    }

    /** First http(s) link in shared text ("Title https://youtu.be/x" from the YouTube share sheet). */
    fun firstUrl(text: String): String? = Regex("https?://[^\\s<>\"]+").find(text)?.value?.trimEnd('.', ',', ')', ']')

    // ------------------------------------------------------------------ tiny flat JSON (no org.json: JVM tests)

    fun esc(s: String): String {
        val b = StringBuilder("\"")
        for (c in s) when {
            c == '"' -> b.append("\\\""); c == '\\' -> b.append("\\\\")
            c == '\n' -> b.append("\\n"); c == '\r' -> b.append("\\r"); c == '\t' -> b.append("\\t")
            c < ' ' -> b.append("\\u%04x".format(c.code))
            else -> b.append(c)
        }
        return b.append('"').toString()
    }

    fun json(m: Map<String, Any?>): String = m.entries.joinToString(",", "{", "}") { (k, v) ->
        esc(k) + ":" + when (v) { null -> "null"; is String -> esc(v); is Boolean, is Number -> v.toString(); else -> esc(v.toString()) }
    }

    /** Parses one flat JSON object (strings, numbers, booleans, null). Nested values are skipped. */
    fun parseFlat(s: String): Map<String, Any?> {
        val out = LinkedHashMap<String, Any?>()
        var i = 0
        fun ws() { while (i < s.length && s[i].isWhitespace()) i++ }
        fun str(): String {
            val b = StringBuilder(); i++ // opening quote
            while (i < s.length && s[i] != '"') {
                if (s[i] == '\\' && i + 1 < s.length) {
                    i++
                    when (s[i]) { 'n' -> b.append('\n'); 'r' -> b.append('\r'); 't' -> b.append('\t'); 'b' -> b.append('\b'); 'f' -> b.append('\u000c')
                        'u' -> { b.append(s.substring(i + 1, i + 5).toInt(16).toChar()); i += 4 }
                        else -> b.append(s[i]) }
                } else b.append(s[i])
                i++
            }
            i++
            return b.toString()
        }
        fun skip() { var d = 0; do { val c = s[i]; if (c == '"') { str(); continue }; if (c == '{' || c == '[') d++; if (c == '}' || c == ']') d--; i++ } while (d > 0 && i < s.length) }
        ws(); if (i >= s.length || s[i] != '{') return out; i++
        while (true) {
            ws(); if (i >= s.length || s[i] == '}') break
            if (s[i] == ',') { i++; continue }
            if (s[i] != '"') break
            val k = str(); ws(); if (i < s.length && s[i] == ':') i++; ws()
            if (i >= s.length) break
            out[k] = when {
                s[i] == '"' -> str()
                s.startsWith("true", i) -> { i += 4; true }
                s.startsWith("false", i) -> { i += 5; false }
                s.startsWith("null", i) -> { i += 4; null }
                s[i] == '{' || s[i] == '[' -> { skip(); null }
                else -> { val st = i; while (i < s.length && s[i] !in ",}" && !s[i].isWhitespace()) i++; s.substring(st, i).toLongOrNull() ?: s.substring(st, i).toDoubleOrNull() }
            }
        }
        return out
    }

    // ------------------------------------------------------------------ suggestion

    /** The `"recent":[{...},...]` array of a LAST_ACTIVITY answer ("More from your PC"), in order. Bad items are skipped. */
    fun parseRecent(s: String): List<Suggestion> {
        var i = s.indexOf("\"recent\"")
        if (i < 0) return emptyList()
        i = s.indexOf('[', i)
        if (i < 0) return emptyList()
        val out = ArrayList<Suggestion>()
        i++
        var depth = 0
        var start = -1
        var inStr = false
        while (i < s.length) {
            val c = s[i]
            if (inStr) {
                if (c == '\\') i++ else if (c == '"') inStr = false
            } else when (c) {
                '"' -> inStr = true
                '{' -> { if (depth == 0) start = i; depth++ }
                '}' -> { depth--; if (depth == 0 && start >= 0) { Suggestion.parse(s.substring(start, i + 1))?.let { out.add(it) }; start = -1 } }
                ']' -> if (depth == 0) return out
            }
            i++
        }
        return out
    }

    data class Suggestion(val app: String, val title: String, val url: String, val posMs: Long?, val playing: Boolean, val atMs: Long) {
        fun toJson() = json(mapOf("app" to app, "title" to title, "url" to url, "pos_ms" to posMs, "playing" to playing, "at_ms" to atMs))
        fun openUrl(nowMs: Long) = withTime(url, positionNow(posMs, playing, atMs, nowMs))
        companion object {
            fun parse(s: String): Suggestion? {
                val m = parseFlat(s)
                val url = m["url"] as? String ?: return null
                if (!url.startsWith("https://") && !url.startsWith("http://")) return null
                return Suggestion((m["app"] as? String) ?: "web", ((m["title"] as? String) ?: "").take(200), url,
                    (m["pos_ms"] as? Number)?.toLong(), m["playing"] == true, (m["at_ms"] as? Number)?.toLong() ?: 0)
            }
        }
    }

    /** "On change only": a key must settle for [settleMs], differ from the last sent, and [minGapMs] must pass. */
    class Debouncer(private val settleMs: Long, private val minGapMs: Long) {
        private var pending: Pair<String, Long>? = null
        private var lastSent: String? = null
        private var lastSentAt = 0L
        fun observe(key: String?, now: Long) {
            if (key == null) { pending = null; return }
            if (pending?.first != key) pending = key to now
        }
        fun due(now: Long): String? {
            val (k, since) = pending ?: return null
            if (now < since + settleMs || lastSent == k) return null
            if (lastSent != null && now < lastSentAt + minGapMs) return null
            lastSent = k; lastSentAt = now; pending = null
            return k
        }
        fun nextWake(): Long? {
            val (k, since) = pending ?: return null
            if (lastSent == k) return null
            return maxOf(since + settleMs, if (lastSent != null) lastSentAt + minGapMs else 0)
        }
    }

    /** Clipboard ping-pong guard; keeps hashes only, never contents. */
    class ClipEcho {
        private var applied: Int? = null
        private var sent: Int? = null
        fun noteApplied(data: ByteArray) { applied = data.contentHashCode() }
        fun shouldSend(data: ByteArray): Boolean {
            val h = data.contentHashCode()
            if (h == applied || h == sent) return false
            sent = h; return true
        }
    }
}
