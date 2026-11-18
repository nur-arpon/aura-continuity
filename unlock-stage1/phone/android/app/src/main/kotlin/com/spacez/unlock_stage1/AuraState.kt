package com.spacez.unlock_stage1

import android.content.Context
import org.json.JSONArray
import org.json.JSONObject

/**
 * What the Aura home screen, widget and tiles show about the PC (item 13). Only remembers events
 * that already happen (no polling): the PC's Bluetooth link coming and going (CDM presence), the
 * Aura Link session (and which pipe carries it), PC_STATE / lock / unlock results, the PC's name
 * from its Aura Link hello.
 */
object PcState {
    private fun p(c: Context) = c.getSharedPreferences("aura_pc", 0)
    private fun now() = System.currentTimeMillis()

    fun name(c: Context) = p(c).getString("name", null)
    fun setName(c: Context, n: String) { if (n.isNotBlank() && n != name(c)) p(c).edit().putString("name", n.trim()).apply() }

    /** CDM presence: the PC's Bluetooth link to this phone. */
    fun setNear(c: Context, near: Boolean) = p(c).edit().putBoolean("near", near).putLong("near_at", now()).apply()
    fun near(c: Context) = p(c).getBoolean("near", false)
    fun nearAt(c: Context) = p(c).getLong("near_at", 0)

    /** true = locked, false = unlocked, null = not known. */
    fun setLocked(c: Context, locked: Boolean) {
        p(c).edit().putInt("locked", if (locked) 1 else 0).putLong("locked_at", now()).apply()
        runCatching { com.spacez.unlock_stage1.away.AwayWidget.updateAll(c) }
    }
    fun locked(c: Context): Boolean? = when (p(c).getInt("locked", -1)) { 1 -> true; 0 -> false; else -> null }
    fun lockedAt(c: Context) = p(c).getLong("locked_at", 0)

    fun sessionUp(c: Context, label: String?) = p(c).edit().putString("via", label ?: "").putLong("via_at", now()).apply()
    fun lastSeen(c: Context) = maxOf(p(c).getLong("via_at", 0), if (near(c)) now() else nearAt(c))

    fun forget(c: Context) = p(c).edit().clear().apply()
}

/**
 * Handoff inbox (item 13): the last 60 things that went between the phone and the PC, newest
 * first. Stores what is needed to show and reopen an item (title, host, link, file uri); never
 * clipboard contents (only "Clipboard from PC" and a length).
 */
object HandoffInbox {
    private const val MAX = 60
    private fun p(c: Context) = c.getSharedPreferences("aura_inbox", 0)
    private val lock = Any()

    data class Item(val id: Long, val kind: String, val dir: String, val title: String, val sub: String,
                    val url: String?, val uri: String?, val mime: String?, val at: Long) {
        fun json() = JSONObject().put("id", id).put("kind", kind).put("dir", dir).put("title", title).put("sub", sub)
            .put("url", url ?: "").put("uri", uri ?: "").put("mime", mime ?: "").put("at", at)
        fun map(): Map<String, Any> = mapOf("id" to id, "kind" to kind, "dir" to dir, "title" to title, "sub" to sub,
            "url" to (url ?: ""), "uri" to (uri ?: ""), "mime" to (mime ?: ""), "at" to at)
        companion object {
            fun parse(o: JSONObject) = Item(o.optLong("id"), o.optString("kind"), o.optString("dir"), o.optString("title"),
                o.optString("sub"), o.optString("url").ifEmpty { null }, o.optString("uri").ifEmpty { null },
                o.optString("mime").ifEmpty { null }, o.optLong("at"))
        }
    }

    fun all(c: Context): List<Item> = synchronized(lock) {
        val a = runCatching { JSONArray(p(c).getString("items", "[]")) }.getOrElse { JSONArray() }
        (0 until a.length()).mapNotNull { runCatching { Item.parse(a.getJSONObject(it)) }.getOrNull() }
    }

    /** kind: video | link | file | clip | text. dir: in (from the PC) | out (to the PC). */
    fun add(c: Context, kind: String, dir: String, title: String, sub: String, url: String? = null, uri: String? = null, mime: String? = null) {
        synchronized(lock) {
            val now = System.currentTimeMillis()
            val items = all(c).toMutableList()
            // the same link again within 2 minutes (a retry, a re-open) replaces the old row
            items.removeAll { url != null && it.url == url && it.dir == dir && now - it.at < 120_000 }
            items.add(0, Item(now, kind, dir, title.take(200), sub.take(120), url, uri, mime, now))
            val a = JSONArray(); items.take(MAX).forEach { a.put(it.json()) }
            p(c).edit().putString("items", a.toString()).putInt("unseen", p(c).getInt("unseen", 0) + if (dir == "in") 1 else 0).apply()
        }
    }

    fun get(c: Context, id: Long) = all(c).firstOrNull { it.id == id }
    fun unseen(c: Context) = p(c).getInt("unseen", 0)
    fun markSeen(c: Context) = p(c).edit().putInt("unseen", 0).apply()
    fun remove(c: Context, id: Long) = synchronized(lock) {
        val a = JSONArray(); all(c).filter { it.id != id }.forEach { a.put(it.json()) }
        p(c).edit().putString("items", a.toString()).apply()
    }
    fun clear(c: Context) = p(c).edit().putString("items", "[]").putInt("unseen", 0).apply()
}
