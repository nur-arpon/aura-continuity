package com.spacez.unlock_stage1.handoff

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.content.pm.ShortcutInfo
import android.content.pm.ShortcutManager
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.RectF
import android.graphics.drawable.Icon
import android.util.Base64
import com.spacez.aura.link.Message
import com.spacez.unlock_stage1.UnlockState
import org.json.JSONArray
import org.json.JSONObject
import java.io.File

/** Pure part (JVM-tested): merging the PC's catalog records and choosing the dynamic shortcuts. */
object CatalogLogic {
    const val DYNAMIC = 4
    data class App(val id: String, val name: String, val recent: Boolean)

    /** Records of one answer arrive as part 0..n-1: part 0 starts a fresh list, later parts add (a repeated id is replaced). */
    fun merge(current: List<App>, part: Int, incoming: List<App>): List<App> {
        val base = if (part == 0) emptyList() else current
        val ids = incoming.map { it.id }.toSet()
        return base.filter { it.id !in ids } + incoming
    }

    /** Most recently opened-from-the-phone apps first, then the PC's own recent windows, then the list order; at most [DYNAMIC]. */
    fun top(apps: List<App>, opened: List<String>): List<App> {
        val byId = apps.associateBy { it.id }
        val head = opened.mapNotNull { byId[it] }
        val rest = apps.filter { a -> head.none { it.id == a.id } }.sortedByDescending { it.recent } // stable: keeps PC order inside each group
        return (head + rest).take(DYNAMIC)
    }

    fun shortcutId(id: String) = "lap-$id"
}

/**
 * "Laptop apps" on the phone: every app the PC offered (name + real icon), kept on disk so the picker and the long-press shortcuts
 * work without the laptop being up. Tapping a shortcut runs the chat head's path (ask the PC to open that exe, Apollo per-app entry,
 * Artemis straight into it) from a small translucent screen, [LaptopAppActivity].
 */
object LaptopCatalog {
    private const val PREFS = "aura_laptop_apps"
    @Volatile private var apps: List<CatalogLogic.App>? = null
    private val lock = Any()
    @Volatile var onChange: (() -> Unit)? = null

    private fun dir(c: Context) = File(c.filesDir, "laptop_icons").apply { mkdirs() }
    private fun p(c: Context) = c.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
    private fun log(s: String) = UnlockState.log("laptop apps: $s")

    fun list(c: Context): List<CatalogLogic.App> = apps ?: synchronized(lock) {
        apps ?: runCatching {
            val a = JSONArray(p(c).getString("list", "[]"))
            (0 until a.length()).map { a.getJSONObject(it).let { o -> CatalogLogic.App(o.getString("id"), o.getString("name"), o.optBoolean("recent")) } }
        }.getOrDefault(emptyList()).also { apps = it }
    }

    private fun safe(id: String) = id.replace(Regex("[^A-Za-z0-9._-]"), "_")
    fun iconFile(c: Context, id: String) = File(dir(c), safe(id) + ".png")
    fun icon(c: Context, id: String): Bitmap? = iconFile(c, id).takeIf { it.exists() }?.let { BitmapFactory.decodeFile(it.path) }
    fun iconBytes(c: Context, id: String): ByteArray? = iconFile(c, id).takeIf { it.exists() }?.readBytes()

    private fun opened(c: Context): List<String> = p(c).getString("opened", "")!!.split(',').filter { it.isNotBlank() }

    /** LAPTOP_APPS record with "all": a part of the PC's full list. */
    fun onMessage(c: Context, json: String) {
        val o = runCatching { JSONObject(json) }.getOrNull() ?: return
        val arr = o.optJSONArray("apps") ?: return
        val incoming = ArrayList<CatalogLogic.App>()
        for (i in 0 until arr.length()) {
            val a = arr.getJSONObject(i)
            val id = a.optString("id").ifBlank { continue }
            a.optString("icon").takeIf { it.isNotEmpty() }?.let { b64 ->
                runCatching { iconFile(c, id).writeBytes(Base64.decode(b64, Base64.DEFAULT)) }
            }
            incoming.add(CatalogLogic.App(id, a.optString("name", id), a.optBoolean("recent")))
        }
        val part = o.optInt("part", 0)
        val merged = synchronized(lock) { CatalogLogic.merge(list(c), part, incoming).also { apps = it } }
        // saved once the last part is in (an interrupted answer keeps the older saved list)
        if (part >= o.optInt("of", 1) - 1) {
            val ja = JSONArray()
            merged.forEach { ja.put(JSONObject().put("id", it.id).put("name", it.name).put("recent", it.recent)) }
            p(c).edit().putString("list", ja.toString()).apply()
            log("${merged.size} apps from the PC")
            runCatching { refreshDynamic(c) }.onFailure { log("long-press shortcuts: ${it.message}") }
        }
        onChange?.invoke()
    }

    /** Ask the PC for its full list (answered in a few LAPTOP_APPS records). */
    fun ask(c: Context) {
        val app = c.applicationContext
        Thread {
            com.spacez.unlock_stage1.away.AwayPhone.userAction()
            fun lk() = com.spacez.unlock_stage1.away.AwayPhone.link(app) ?: HandoffPhone.link(app)
            var l = lk()
            var waited = 0
            while ((l == null || !l.connected) && waited < 5000) { Thread.sleep(200); waited += 200; l = lk() }
            l?.send(Message.laptopApps("{\"ask\":\"all\"}"))
            log("asked the PC for all apps (link connected=${l?.connected})")
        }.start()
    }

    // ------------------------------------------------------------------ shortcuts

    /** The shortcut icon: the app icon on a soft tile, inside the adaptive-icon safe zone. */
    fun tile(c: Context, id: String): Icon {
        val size = 216
        val bmp = Bitmap.createBitmap(size, size, Bitmap.Config.ARGB_8888)
        val cv = Canvas(bmp)
        cv.drawColor(Color.rgb(244, 245, 250))
        val src = icon(c, id)
        if (src != null) {
            val s = 112f
            val l = (size - s) / 2f
            cv.drawBitmap(src, null, RectF(l, l, l + s, l + s), Paint(Paint.FILTER_BITMAP_FLAG or Paint.ANTI_ALIAS_FLAG))
        } else {
            val pt = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = Color.rgb(90, 98, 140); textSize = 90f; textAlign = Paint.Align.CENTER }
            cv.drawText((list(c).firstOrNull { it.id == id }?.name ?: id).take(1).uppercase(), size / 2f, size / 2f + 32f, pt)
        }
        return Icon.createWithAdaptiveBitmap(bmp)
    }

    fun launchIntent(c: Context, id: String, name: String): Intent =
        Intent(c, LaptopAppActivity::class.java).setAction(Intent.ACTION_MAIN).putExtra("laptop_app", id).putExtra("laptop_name", name)

    private fun info(c: Context, a: CatalogLogic.App): ShortcutInfo =
        ShortcutInfo.Builder(c, CatalogLogic.shortcutId(a.id)).setShortLabel(a.name.take(18)).setLongLabel("${a.name} on laptop")
            .setIcon(tile(c, a.id)).setIntent(launchIntent(c, a.id, a.name)).build()

    /** Home-screen pin of one app (the launcher shows its own confirmation). */
    fun pin(c: Context, id: String): Boolean {
        val a = list(c).firstOrNull { it.id == id } ?: return false
        val sm = c.getSystemService(ShortcutManager::class.java) ?: return false
        if (!sm.isRequestPinShortcutSupported) return false
        return sm.requestPinShortcut(info(c, a), null).also { log("pin ${a.id}: $it") }
    }

    /** Long-press menu of the Aura icon: the top four. Static shortcuts of the app (if any) keep their slots. */
    fun refreshDynamic(c: Context) {
        val sm = c.getSystemService(ShortcutManager::class.java) ?: return
        val room = (sm.maxShortcutCountPerActivity - sm.manifestShortcuts.size).coerceAtLeast(0)
        val top = CatalogLogic.top(list(c), opened(c)).take(minOf(CatalogLogic.DYNAMIC, room))
        if (top.isEmpty()) return
        sm.dynamicShortcuts = top.map { info(c, it) }
        // pinned copies pick up a fresh icon / name too
        val pinned = sm.pinnedShortcuts.filter { it.id.startsWith("lap-") }.mapNotNull { s -> list(c).firstOrNull { CatalogLogic.shortcutId(it.id) == s.id } }
        if (pinned.isNotEmpty()) runCatching { sm.updateShortcuts(pinned.map { info(c, it) }) }
        log("long-press shortcuts: ${top.joinToString { it.id }}")
    }

    /** A laptop app was opened from the phone: it moves to the front of the long-press menu. */
    fun noteOpened(c: Context, id: String) {
        val l = (listOf(id) + opened(c).filter { it != id }).take(8)
        p(c).edit().putString("opened", l.joinToString(",")).apply()
        runCatching { refreshDynamic(c) }
    }
}

/**
 * Target of every laptop-app shortcut and picker tap: a translucent screen that stays in front while the PC brings the app up
 * (Android only lets a visible app start Artemis), then closes itself.
 */
class LaptopAppActivity : Activity() {
    override fun onCreate(b: android.os.Bundle?) {
        super.onCreate(b)
        val id = intent.getStringExtra("laptop_app") ?: run { finish(); return }
        val name = intent.getStringExtra("laptop_name") ?: id
        val tv = android.widget.TextView(this).apply {
            text = "Opening $name on your laptop..."
            setTextColor(Color.WHITE); textSize = 16f; gravity = android.view.Gravity.CENTER
            setPadding(48, 36, 48, 36)
            background = android.graphics.drawable.GradientDrawable().apply { cornerRadius = 60f; setColor(Color.argb(220, 28, 30, 40)) }
        }
        setContentView(android.widget.FrameLayout(this).apply {
            addView(tv, android.widget.FrameLayout.LayoutParams(-2, -2, android.view.Gravity.CENTER))
        })
        val app = applicationContext
        LaptopCatalog.noteOpened(app, id)
        ChatHead.openLaptopAppById(app, id, name) { runOnUiThread { if (!isFinishing) finish() } }
        window.decorView.postDelayed({ if (!isFinishing) finish() }, 22_000) // never lingers
    }
}
