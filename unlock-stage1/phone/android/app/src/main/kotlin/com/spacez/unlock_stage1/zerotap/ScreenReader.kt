package com.spacez.unlock_stage1.zerotap

/**
 * Zero-tap continue (item 19): what a supported app shows, from the few labels the accessibility
 * service hands over (view id, text, content description of the top of the window). Pure and
 * unit-tested. Nothing here is stored or logged: the result is one small "state" sent to the PC
 * over Aura Link when it changes, and kept in memory only.
 */
object ScreenReader {
    /** One label of the window: resource id (may be empty), text, content description. */
    data class Node(val id: String, val text: String?, val desc: String?)

    /** What the app shows: app kind, a title (chat / conversation / page), a link if known. */
    data class Screen(val app: String, val title: String, val url: String?)

    private val GENERIC = setOf("claude", "new chat", "chats", "messenger", "facebook", "whatsapp", "search", "home", "menu", "back",
        "navigate up", "more options", "settings", "calls", "updates", "communities", "stories", "reels", "notifications", "video", "marketplace")

    private fun clean(s: String?) = s?.trim()?.replace(Regex("\\s+"), " ")?.takeIf { it.isNotEmpty() }

    /** "example.com/a" or a full link from a browser's address bar -> https link. */
    fun addressToUrl(raw: String?): String? {
        val t = clean(raw) ?: return null
        if (t.contains(' ')) return null // search text being typed
        if (t.startsWith("https://") || t.startsWith("http://")) return t
        val host = t.substringBefore('/')
        if (!host.contains('.') || host.startsWith('.') || host.endsWith('.')) return null
        return "https://$t"
    }

    fun read(pkg: String, nodes: List<Node>): Screen? {
        val kind = ContinueRules.appKind(pkg) ?: return null
        fun byId(vararg suffixes: String) = nodes.firstOrNull { n -> suffixes.any { n.id.endsWith(":id/$it") } }
        return when (kind) {
            "web" -> {
                val bar = byId("location_bar_edit_text", "url_bar", "url_field", "search_box_text")
                val url = addressToUrl(bar?.text) ?: return null
                Screen("web", "", url)
            }
            "whatsapp" -> {
                // the open chat's name in the conversation toolbar (absent on the chat list)
                val name = clean(byId("conversation_contact_name")?.text) ?: return null
                Screen("whatsapp", name, null)
            }
            "messenger" -> {
                val name = clean(byId("thread_title", "thread_title_name", "title")?.text) ?: firstTitle(nodes) ?: return null
                Screen("messenger", name, null)
            }
            "claude" -> {
                val t = firstTitle(nodes) ?: return null
                Screen("claude", t, null)
            }
            "facebook" -> Screen("facebook", "", "https://www.facebook.com/")
            else -> null // YouTube / YT Music / Spotify come from their media sessions
        }
    }

    /** The first meaningful label near the top (a toolbar title): not a generic button name. */
    fun firstTitle(nodes: List<Node>): String? = nodes.asSequence()
        .mapNotNull { clean(it.text) ?: clean(it.desc) }
        .firstOrNull { it.length in 3..120 && it.lowercase() !in GENERIC && !it.all { c -> c.isDigit() || c == ':' } }
}
