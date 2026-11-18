package com.spacez.unlock_stage1.handoff

import android.app.KeyguardManager
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.graphics.drawable.Icon
import android.os.SystemClock
import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Color
import android.graphics.PixelFormat
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.media.AudioManager
import android.net.Uri
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import android.util.Base64
import android.util.TypedValue
import android.view.Gravity
import android.view.MotionEvent
import android.view.View
import android.view.ViewGroup
import android.view.WindowInsets
import android.view.WindowManager
import android.view.animation.Interpolator
import android.widget.FrameLayout
import android.widget.ImageView
import android.widget.TextView
import com.spacez.aura.link.Message
import com.spacez.unlock_stage1.AuraPhone
import com.spacez.unlock_stage1.R
import com.spacez.unlock_stage1.UnlockState
import org.json.JSONObject
import kotlin.concurrent.thread
import kotlin.math.hypot

/**
 * Chat-head tree (phone): an Aura head over other apps (Display over other apps). Tap -> bubbles for the laptop's 3-4
 * most recent apps (their real Windows icons, sent by the PC as LAPTOP_APPS); tap a browser -> bubbles for its open
 * tabs (favicon + short title); tap a tab -> the page opens here (YouTube at the same second). A non-browser app
 * opens the laptop screen in Moonlight. Tap outside folds one level back; drag the head to the X to dismiss it for
 * 30 minutes (an ongoing notification "Aura chat head" has a "Show bubble" action that brings it back at once). It stays through
 * opening an app, a dropped link and the like: only the X (or the switch) removes it. Hidden during calls, full-screen apps,
 * the lock screen. Pure rules: [ChatHeadLogic].
 */
object ChatHead {
    private const val PREFS = "chat_head"
    private val main = Handler(Looper.getMainLooper())

    @Volatile private var ctx: Context? = null
    @Volatile var apps: List<ChatHeadLogic.LApp> = emptyList()
    private val icons = HashMap<String, Bitmap>()
    /** site icons sent by the laptop (LAPTOP_APPS.favicons, once per host per session), keyed by the tab link's host */
    private val favicons = HashMap<String, Bitmap>()

    private var wm: WindowManager? = null
    private var head: FrameLayout? = null
    private var headLp: WindowManager.LayoutParams? = null
    private var tree: FrameLayout? = null
    private var xTarget: FrameLayout? = null
    private var level = 0
    private var started = false
    private var lastAsk = 0L
    private var fullscreen = false
    private var l1: List<FrameLayout> = emptyList()
    private var l2: List<FrameLayout> = emptyList()
    private var opened: FrameLayout? = null
    /** One tap = one action: set when a bubble did its thing, until the tree is opened again. */
    private var acting = false
    /** A LAPTOP_APPS that arrived while the tree was open: applied when it folds (bubbles never change under a finger). */
    private var pendingApps: List<ChatHeadLogic.LApp>? = null
    private var lastLinkUp = 0L
    private var lastWhy = ""
    private var askedFresh = false
    private const val NCH = "aura_chat_head"
    private const val NID = 4301
    private const val ACTION_SHOW = "com.spacez.unlock_stage1.CHAT_HEAD_SHOW"

    private fun p(c: Context) = c.getSharedPreferences(PREFS, 0)
    fun enabled(c: Context) = p(c).getBoolean("on", true)
    fun setEnabled(c: Context, on: Boolean) { p(c).edit().putBoolean("on", on).apply(); main.post { tick() }; postNotification(c) }
    fun permitted(c: Context) = Build.VERSION.SDK_INT < 23 || Settings.canDrawOverlays(c)
    fun status(c: Context): Map<String, Any?> = mapOf("chatHead" to enabled(c), "chatHeadAllowed" to permitted(c), "chatHeadApps" to apps.size)
    fun permissionIntent(c: Context): Intent =
        Intent(Settings.ACTION_MANAGE_OVERLAY_PERMISSION, Uri.parse("package:${c.packageName}")).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
    private fun log(s: String) = UnlockState.log("chat head: $s")

    private fun dp(v: Float): Int = TypedValue.applyDimension(TypedValue.COMPLEX_UNIT_DIP, v, ctx!!.resources.displayMetrics).toInt()
    private val springI = Interpolator { ChatHeadLogic.spring(it) }
    private val softI = Interpolator { ChatHeadLogic.spring(it, 9f, 8f) }

    // ------------------------------------------------------------------ data from the PC

    /** LAPTOP_APPS from the PC. */
    fun onLaptopApps(c: Context, json: String) {
        ctx = c.applicationContext
        runCatching { JSONObject(json) }.getOrNull()?.takeIf { it.has("all") }?.let { LaptopCatalog.onMessage(c.applicationContext, json); return }
        runCatching { JSONObject(json) }.getOrNull()?.takeIf { it.has("opened") }?.let { onOpened(it); return }
        runCatching {
            val f = JSONObject(json).optJSONObject("favicons") ?: return@runCatching
            for (host in f.keys()) {
                val bytes = runCatching { Base64.decode(f.optString(host), Base64.DEFAULT) }.getOrNull() ?: continue
                BitmapFactory.decodeByteArray(bytes, 0, bytes.size)?.let { synchronized(favicons) { favicons[host] = it } }
            }
        }
        val list = runCatching {
            val arr = JSONObject(json).optJSONArray("apps") ?: return@runCatching emptyList()
            (0 until arr.length()).mapNotNull { i ->
                val o = arr.getJSONObject(i)
                val id = o.optString("id").ifBlank { return@mapNotNull null }
                o.optString("icon").takeIf { it.isNotEmpty() }?.let { b64 ->
                    runCatching { Base64.decode(b64, Base64.DEFAULT) }.getOrNull()?.let { BitmapFactory.decodeByteArray(it, 0, it.size) }?.let { synchronized(icons) { icons[id] = it } }
                }
                val tabs = o.optJSONArray("tabs")?.let { t ->
                    (0 until t.length()).map { j -> t.getJSONObject(j).let { x -> ChatHeadLogic.Tab(x.optString("title"), x.optString("url").takeIf { u -> u.startsWith("http") }, x.optBoolean("active")) } }
                } ?: emptyList()
                ChatHeadLogic.LApp(id, o.optString("name", id), o.optBoolean("browser"), o.optString("title"), o.optString("url").takeIf { it.startsWith("http") },
                    if (o.has("pos_ms")) o.optLong("pos_ms") else null, tabs)
            }
        }.getOrElse { log("bad LAPTOP_APPS: ${it.message}"); return }
        val shown = list.take(ChatHeadLogic.MAX_APPS)
        log("laptop apps: ${shown.joinToString { "${it.id}(${it.tabs.size})" }}")
        main.post { if (level == 0) { apps = shown; tick() } else pendingApps = shown }
    }

    /** Ask the PC for its recent apps (empty-ish request, answered with LAPTOP_APPS). */
    fun ask() {
        val l = AuraPhone.link ?: return
        if (!l.connected) return
        lastAsk = System.currentTimeMillis()
        l.send(Message.laptopApps("{\"ask\":1}"))
    }

    // ------------------------------------------------------------------ lifecycle

    /** Start the 2-second watcher (idempotent). Called from the foreground service and the app. */
    fun attach(c: Context) {
        ctx = c.applicationContext
        if (started) { main.post { tick() }; return }
        started = true
        postNotification(ctx!!)
        wm = ctx!!.getSystemService(Context.WINDOW_SERVICE) as WindowManager
        main.post(object : Runnable { override fun run() { runCatching { tick() }.onFailure { log("tick: ${it.message}") }; main.postDelayed(this, 2000) } })
        log("watching")
    }

    private fun inCall(c: Context): Boolean = (c.getSystemService(Context.AUDIO_SERVICE) as AudioManager).mode.let { it == AudioManager.MODE_IN_CALL || it == AudioManager.MODE_IN_COMMUNICATION || it == AudioManager.MODE_RINGTONE }

    private fun tick() {
        val c = ctx ?: return
        val link = AuraPhone.link?.connected == true
        val now = System.currentTimeMillis()
        if (link) lastLinkUp = now
        if (level > 0) AuraPhone.link?.keepWarm(60_000) // tree open: keep the Aura Link up
        val locked = (c.getSystemService(Context.KEYGUARD_SERVICE) as KeyguardManager).isKeyguardLocked
        // the link to the PC comes and goes (Bluetooth, idle sessions): the head stays through a drop
        val show = ChatHeadLogic.shouldShow(enabled(c), permitted(c), ChatHeadLogic.linkOk(link, lastLinkUp, now), apps.isNotEmpty(), now, p(c).getLong("until", 0), false, false, locked) // calls do not hide it either (Messenger-style)
        // the insets callback may not run when an app goes back from full screen: read them too
        if (Build.VERSION.SDK_INT >= 30) head?.rootWindowInsets?.let { fullscreen = !it.isVisible(WindowInsets.Type.statusBars()) }
        val why = "on=${enabled(c)} perm=${permitted(c)} link=${ChatHeadLogic.linkOk(link, lastLinkUp, now)} apps=${apps.size} dismissed=${now < p(c).getLong("until", 0)} locked=$locked head=${head != null}"
        if (why != lastWhy) { lastWhy = why; log("show=$show $why") }
        if (link && apps.isEmpty() && now - lastAsk > 20_000) ask()
        // a fresh process (after an update) has no list and maybe no link: dial once for it, so the head comes back by itself
        if (!link && apps.isEmpty() && !askedFresh && enabled(c) && permitted(c)) {
            askedFresh = true; lastAsk = now
            thread(isDaemon = true) { runCatching { HandoffPhone.link(c)?.send(Message.laptopApps("{\"ask\":1}")) } }
        }
        if (show && head == null) addHead()
        else if (!show && head != null && drag == null) { if (level > 0) fold(true); removeHead() }
        // full-screen app in front (video, game): the head stays attached (it keeps hearing about the system bars) but unseen
        // like Messenger: the head only goes away when thrown on the X (full screen included); an open tree folds for a full-screen app
        // an open tree only folds on a tap outside the bubbles (or after a bubble did its job), never by itself
        if (head != null && level == 0) { setUnseen(false); keepOnScreen() }
    }

    /** After a rotation (Artemis is landscape) the old x/y can be off the screen: re-snap to the remembered edge and height. */
    private fun keepOnScreen() {
        val c = ctx ?: return
        val h = head ?: return
        val l = headLp ?: return
        if (drag != null || snapping) return
        val m = c.resources.displayMetrics
        val size = dp(56f)
        val x = if (p(c).getBoolean("right", true)) m.widthPixels - size - dp(4f) else dp(4f)
        val y = (p(c).getFloat("y", 0.42f) * m.heightPixels).toInt().coerceIn(dp(40f), (m.heightPixels - size - dp(80f)).coerceAtLeast(dp(40f)))
        if (l.x != x || l.y != y) { l.x = x; l.y = y; runCatching { wm?.updateViewLayout(h, l) } }
    }

    /**
     * Unseen = transparent and not touchable, but still a VISIBLE window: Android stops sending insets to an INVISIBLE
     * window, so a head hidden that way never heard the status bar come back and stayed hidden for good.
     */
    private fun setUnseen(unseen: Boolean) {
        val h = head ?: return
        val l = headLp ?: return
        if (h.visibility != View.VISIBLE) h.visibility = View.VISIBLE
        val want = if (unseen) 0f else 1f
        if (h.alpha != want) { if (unseen) h.animate().cancel(); h.alpha = want }
        val flags = if (unseen) l.flags or WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE else l.flags and WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE.inv()
        if (flags != l.flags) { l.flags = flags; runCatching { wm?.updateViewLayout(h, l) } }
    }

    // ------------------------------------------------------------------ the head

    private fun lp(w: Int, h: Int, x: Int, y: Int, touchable: Boolean = true) = WindowManager.LayoutParams(
        w, h, WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY,
        WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or WindowManager.LayoutParams.FLAG_LAYOUT_IN_SCREEN or WindowManager.LayoutParams.FLAG_LAYOUT_NO_LIMITS or
            WindowManager.LayoutParams.FLAG_NOT_TOUCH_MODAL or (if (touchable) 0 else WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE),
        PixelFormat.TRANSLUCENT,
    ).apply { gravity = Gravity.TOP or Gravity.START; this.x = x; this.y = y }

    private fun round(color: Int, stroke: Int = 0) = GradientDrawable().apply {
        shape = GradientDrawable.OVAL; setColor(color); if (stroke != 0) setStroke(dp(2f), stroke)
    }

    private fun headView(): FrameLayout {
        val c = ctx!!
        val size = dp(56f)
        val f = FrameLayout(c)
        f.layoutParams = ViewGroup.LayoutParams(size, size)
        f.background = GradientDrawable(GradientDrawable.Orientation.TL_BR, intArrayOf(Color.rgb(34, 84, 170), Color.rgb(12, 26, 51))).apply { shape = GradientDrawable.OVAL; setStroke(dp(2f), Color.argb(150, 120, 170, 255)) }
        f.elevation = dp(8f).toFloat()
        f.outlineProvider = android.view.ViewOutlineProvider.BACKGROUND
        f.addView(ImageView(c).apply { setImageResource(R.drawable.ic_aura_glyph); setPadding(dp(13f), dp(13f), dp(13f), dp(13f)) }, FrameLayout.LayoutParams(size, size))
        return f
    }

    private var drag: DragState? = null
    @Volatile private var snapping = false
    private class DragState(val sx: Float, val sy: Float, val x0: Int, val y0: Int, var moved: Boolean = false)

    private fun addHead() {
        val c = ctx!!
        val w = wm ?: return
        val m = c.resources.displayMetrics
        val size = dp(56f)
        val right = p(c).getBoolean("right", true)
        val y = (p(c).getFloat("y", 0.42f) * m.heightPixels).toInt().coerceIn(dp(40f), m.heightPixels - size - dp(80f))
        val v = headView()
        val params = lp(size, size, if (right) m.widthPixels - size - dp(4f) else dp(4f), y)
        v.setOnTouchListener { _, e -> onHeadTouch(e) }
        v.setOnApplyWindowInsetsListener { _, ins -> fullscreen = Build.VERSION.SDK_INT >= 30 && !ins.isVisible(WindowInsets.Type.statusBars()); ins }
        runCatching { w.addView(v, params) }.onFailure { log("cannot show the head: ${it.message}"); return }
        head = v; headLp = params
        v.alpha = 0f; v.scaleX = 0.3f; v.scaleY = 0.3f
        v.animate().alpha(1f).scaleX(1f).scaleY(1f).setDuration(420).setInterpolator(springI).start()
        log("head shown")
    }

    private fun removeHead() {
        val v = head ?: return
        runCatching { wm?.removeView(v) }
        head = null; headLp = null
        log("head hidden")
    }

    private fun onHeadTouch(e: MotionEvent): Boolean {
        val lp = headLp ?: return false
        val c = ctx ?: return false
        when (e.actionMasked) {
            MotionEvent.ACTION_DOWN -> { drag = DragState(e.rawX, e.rawY, lp.x, lp.y); return true }
            MotionEvent.ACTION_MOVE -> {
                val d = drag ?: return true
                val dx = e.rawX - d.sx; val dy = e.rawY - d.sy
                if (!d.moved && hypot(dx, dy) < dp(8f)) return true
                if (!d.moved) { d.moved = true; if (level > 0) fold(true); showX() }
                val m = c.resources.displayMetrics
                lp.x = (d.x0 + dx).toInt().coerceIn(0, m.widthPixels - dp(56f)); lp.y = (d.y0 + dy).toInt().coerceIn(0, m.heightPixels - dp(56f))
                runCatching { wm?.updateViewLayout(head, lp) }
                val hot = overX(lp)
                xTarget?.let { x -> x.scaleX = if (hot) 1.3f else 1f; x.scaleY = x.scaleX; (x.background as? GradientDrawable)?.setColor(if (hot) Color.rgb(196, 43, 28) else Color.argb(235, 13, 27, 51)) }
                return true
            }
            MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL -> {
                val d = drag ?: return true
                drag = null
                if (!d.moved) { if (e.actionMasked == MotionEvent.ACTION_UP) toggle(); return true }
                val over = overX(lp)
                hideX()
                if (over) { p(c).edit().putLong("until", System.currentTimeMillis() + ChatHeadLogic.SNOOZE_MS).apply(); log("dismissed (notification: Show bubble)"); removeHead(); return true }
                snap(lp)
                return true
            }
        }
        return true
    }

    private fun overX(lp: WindowManager.LayoutParams): Boolean {
        val m = ctx!!.resources.displayMetrics
        return hypot((lp.x + dp(28f) - m.widthPixels / 2f).toDouble(), (lp.y + dp(28f) - (m.heightPixels - dp(110f))).toDouble()) < dp(52f)
    }

    private fun snap(lp: WindowManager.LayoutParams) {
        val c = ctx ?: return
        val m = c.resources.displayMetrics
        val right = lp.x + dp(28f) > m.widthPixels / 2
        val tx = if (right) m.widthPixels - dp(56f) - dp(4f) else dp(4f)
        val x0 = lp.x
        p(c).edit().putBoolean("right", right).putFloat("y", lp.y.toFloat() / m.heightPixels).apply()
        android.animation.ValueAnimator.ofFloat(0f, 1f).apply {
            duration = 460; interpolator = softI
            snapping = true
            addListener(object : android.animation.AnimatorListenerAdapter() { override fun onAnimationEnd(a: android.animation.Animator) { snapping = false } })
            addUpdateListener { lp.x = (x0 + (tx - x0) * it.animatedValue as Float).toInt(); runCatching { wm?.updateViewLayout(head, lp) } }
            start()
        }
    }

    private fun showX() {
        val c = ctx ?: return
        val m = c.resources.displayMetrics
        val s = dp(64f)
        val f = FrameLayout(c).apply {
            background = round(Color.argb(235, 13, 27, 51), Color.argb(90, 255, 255, 255))
            addView(TextView(c).apply { text = "✕"; setTextColor(Color.WHITE); textSize = 24f; gravity = Gravity.CENTER }, FrameLayout.LayoutParams(s, s))
            alpha = 0f; scaleX = 0.6f; scaleY = 0.6f
        }
        runCatching { wm?.addView(f, lp(s, s, m.widthPixels / 2 - s / 2, m.heightPixels - dp(110f) - s / 2, touchable = false)) }
        xTarget = f
        f.animate().alpha(1f).scaleX(1f).scaleY(1f).setDuration(300).setInterpolator(springI).start()
    }

    private fun hideX() {
        val f = xTarget ?: return
        xTarget = null
        f.animate().alpha(0f).scaleX(0.6f).scaleY(0.6f).setDuration(160).withEndAction { runCatching { wm?.removeView(f) } }.start()
    }

    // ------------------------------------------------------------------ the tree

    private fun toggle() { if (level == 0) expand() else fold(false) }

    private fun headCentre(): Pair<Float, Float> { val lp = headLp!!; return (lp.x + dp(28f)).toFloat() to (lp.y + dp(28f)).toFloat() }

    /** The head's centre in the TREE window's coordinates: both windows are read in screen pixels, so a status-bar / cutout offset between them cancels out. */
    private fun headCentreInTree(): Pair<Float, Float> {
        val (hx, hy) = headCentre()
        val t = tree; val h = head
        if (t == null || h == null) return hx to hy
        val a = IntArray(2); val b = IntArray(2)
        t.getLocationOnScreen(a); h.getLocationOnScreen(b)
        return if (h.width > 0) (b[0] + h.width / 2f - a[0]) to (b[1] + h.height / 2f - a[1]) else (hx - a[0]) to (hy - a[1])
    }

    private fun bubble(label: String, icon: Bitmap?, letterOf: String, page: Boolean): FrameLayout {
        val c = ctx!!
        val s = dp(52f)
        val f = FrameLayout(c)
        f.layoutParams = FrameLayout.LayoutParams(s, s)
        f.background = round(if (page) Color.rgb(18, 37, 74) else Color.WHITE)
        f.elevation = dp(6f).toFloat()
        f.outlineProvider = android.view.ViewOutlineProvider.BACKGROUND
        val img = ImageView(c).apply { tag = "img"; scaleType = ImageView.ScaleType.FIT_CENTER; setPadding(dp(if (page) 12f else 7f), dp(if (page) 12f else 7f), dp(if (page) 12f else 7f), dp(if (page) 12f else 7f)) }
        if (icon != null) { img.setImageBitmap(icon) } else {
            val chip = TextView(c).apply {
                tag = "chip"; text = ChatHeadLogic.letter(letterOf); setTextColor(Color.WHITE); textSize = 20f; typeface = Typeface.DEFAULT_BOLD; gravity = Gravity.CENTER
                background = round(Color.HSVToColor(floatArrayOf(ChatHeadLogic.hue(letterOf), 0.55f, 0.62f)))
            }
            f.addView(chip, FrameLayout.LayoutParams(s, s))
        }
        f.addView(img, FrameLayout.LayoutParams(s, s))
        val lab = TextView(c).apply {
            text = label; setTextColor(Color.WHITE); textSize = 11f; maxLines = 1; gravity = Gravity.CENTER; setPadding(dp(8f), dp(2f), dp(8f), dp(2f))
            background = GradientDrawable().apply { cornerRadius = dp(10f).toFloat(); setColor(Color.argb(235, 13, 27, 51)) }
        }
        f.clipChildren = false
        f.addView(lab, FrameLayout.LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT, Gravity.TOP or Gravity.CENTER_HORIZONTAL).apply { topMargin = s + dp(3f) })
        return f
    }

    private fun place(b: FrameLayout, cx: Float, cy: Float) { b.x = cx - dp(26f); b.y = cy - dp(26f) }

    private fun fanOut(list: List<FrameLayout>, cx: Float, cy: Float, r: Float, left: Boolean) {
        val m = ctx!!.resources.displayMetrics
        val pts = ChatHeadLogic.rings(list.size, r, dp(96f).toFloat(), dp(78f).toFloat(), left, ChatHeadLogic.tilt(cy, m.heightPixels.toFloat()))
        // near the top or bottom the arc would be squashed against the edge: fan out from a point far enough in (bubbles still fly from the head)
        val reach = (pts.maxOfOrNull { kotlin.math.abs(it.second) } ?: 0f) + dp(34f)
        val fy = if (m.heightPixels - dp(60f) > 2 * reach) cy.coerceIn(reach, m.heightPixels - dp(60f) - reach) else cy
        list.forEachIndexed { i, b ->
            val (tx, ty) = ChatHeadLogic.clampCentre(cx + pts[i].first, fy + pts[i].second, m.widthPixels, m.heightPixels, dp(26f).toFloat(), dp(26f).toFloat())
            place(b, cx, cy)
            b.alpha = 0f; b.scaleX = 0.2f; b.scaleY = 0.2f
            b.animate().x(tx - dp(26f)).y(ty - dp(26f)).alpha(1f).scaleX(1f).scaleY(1f).setStartDelay(i * 45L).setDuration(520).setInterpolator(springI).start()
        }
    }

    private fun foldAway(list: List<FrameLayout>, cx: Float, cy: Float, then: () -> Unit = {}) {
        val t = tree
        list.forEachIndexed { i, b ->
            b.animate().x(cx - dp(26f)).y(cy - dp(26f)).alpha(0f).scaleX(0.2f).scaleY(0.2f).setStartDelay(i * 20L).setDuration(240)
                .withEndAction { t?.removeView(b); if (i == list.lastIndex) then() }.start()
        }
        if (list.isEmpty()) then()
    }

    private fun expand() {
        val c = ctx ?: return
        val w = wm ?: return
        if (apps.isEmpty()) { ask(); return }
        if (level != 0) return
        val m = c.resources.displayMetrics
        acting = false
        // the tree window takes keys only while open (Back folds one level); folded, it is gone and no window holds focus
        val t = object : FrameLayout(c) {
            override fun dispatchKeyEvent(e: android.view.KeyEvent): Boolean {
                if (e.keyCode == android.view.KeyEvent.KEYCODE_BACK) {
                    if (e.action == android.view.KeyEvent.ACTION_UP) { if (level >= 2) back() else fold(false) }
                    return true
                }
                return super.dispatchKeyEvent(e)
            }
        }
        t.isFocusable = true; t.isFocusableInTouchMode = true
        t.setBackgroundColor(Color.argb(0, 7, 15, 28))
        t.setOnTouchListener { _, e -> if (e.actionMasked == MotionEvent.ACTION_DOWN) { if (level >= 2) back() else fold(false) }; true }
        val tlp = lp(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT, 0, 0)
        tlp.flags = tlp.flags and WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE.inv()
        runCatching { w.addView(t, tlp) }.onFailure { log("cannot open the tree: ${it.message}"); return }
        tree = t
        t.requestFocus()
        android.animation.ValueAnimator.ofInt(0, 90).apply { duration = 260; addUpdateListener { t.setBackgroundColor(Color.argb(it.animatedValue as Int, 7, 15, 28)) }; start() }
        // read the head's place before it goes unseen (an unseen head keeps its window, so this is fine either way)
        level = 1
        t.post {
            if (tree !== t) return@post
            val (cx, cy) = headCentreInTree()
            // a copy of the head inside the tree, so taps on it fold the tree and it never looks dimmed
            val rep = headView().apply { x = cx - dp(28f); y = cy - dp(28f); setOnClickListener { fold(false) } }
            t.addView(rep)
            setUnseen(true) // the tree has its own copy of the head
            // every bubble carries its own app (tag): what is tapped is what opens, however the list moves meanwhile
            l1 = apps.map { a -> bubble(ChatHeadLogic.shortTitle(a.name), synchronized(icons) { icons[a.id] }, a.name, false).also { b -> b.tag = a; b.setOnClickListener { onApp(b) } } }
            l1.forEach { t.addView(it) }
            fanOut(l1, cx, cy, dp(116f).toFloat(), cx > m.widthPixels / 2)
        }
        ask() // the PC answers with fresher tabs
    }

    private fun onApp(b: FrameLayout) {
        if (level != 1 || acting) return
        val c = ctx ?: return
        val tapped = b.tag as? ChatHeadLogic.LApp ?: return
        // fresher tabs / title of the very same app (by id), if the PC sent news while the tree was open
        val a = (pendingApps ?: apps).firstOrNull { it.id == tapped.id } ?: tapped
        when (ChatHeadLogic.appAction(a)) {
            ChatHeadLogic.AppAction.TABS -> {
                level = 2; opened = b
                l1.forEach { if (it !== b) { it.isClickable = false; it.setOnClickListener(null); it.animate().alpha(0.3f).scaleX(0.8f).scaleY(0.8f).setDuration(220).start() } }
                b.isClickable = false; b.setOnClickListener(null) // the opened bubble is not a button while its tabs are out
                val cx = b.x + dp(26f); val cy = b.y + dp(26f)
                val tabs = ChatHeadLogic.visibleTabs(a)
                l2 = tabs.map { tab ->
                    bubble(ChatHeadLogic.shortTitle(tab.title, 14), null, ChatHeadLogic.hostOf(tab.url).ifBlank { tab.title }, true).also { x ->
                        x.setOnClickListener { openTab(a, tab) }
                        loadFavicon(tab.url, x)
                    }
                }
                l2.forEach { tree?.addView(it) }
                fanOut(l2, cx, cy, dp(112f).toFloat(), headCentre().first > c.resources.displayMetrics.widthPixels / 2)
            }
            ChatHeadLogic.AppAction.OPEN_URL -> { acting = true; fold(false); HandoffPhone.open(c, a.url!!, ChatHeadLogic.posFor(a, ChatHeadLogic.Tab(a.title, a.url, true)), "chat head") }
            ChatHeadLogic.AppAction.OPEN_LAPTOP -> {
                acting = true
                HandoffPhone.quietFor(20_000) // Artemis coming to the front must not make Zero-tap open anything else
                fold(false)
                openLaptopApp(c, a)
            }
        }
    }

    /**
     * A laptop app was tapped: first ask the PC to bring exactly that window up (restore + foreground, start the exe if it is closed;
     * the PC also starts its stream host if that is off), then open Artemis straight into the host's Desktop stream.
     */
    /** Same path for a home-screen shortcut / the picker: [done] runs when Artemis was started (or the attempt gave up). */
    fun openLaptopAppById(c: Context, id: String, name: String, done: (() -> Unit)? = null) {
        ctx = c.applicationContext
        HandoffPhone.quietFor(20_000)
        openLaptopApp(c.applicationContext, ChatHeadLogic.LApp(id, name, false, "", null, null, emptyList()), done)
    }

    private fun openLaptopApp(c: Context, a: ChatHeadLogic.LApp, done: (() -> Unit)? = null) {
        // the link idles out between uses: wake it, wait (up to 5 s) until it is really connected, send, resend once if unanswered
        val sent = AuraPhone.link != null || AuraPhone.ensureLink(c) != null
        val token = Any()
        awaiting = a.id to token
        val tapAt = SystemClock.elapsedRealtime()
        fun go(app: String?, uuid: String?) {
            if (awaiting?.second !== token) return
            awaiting = null
            val msg = runCatching {
                if (app != null && uuid != null) com.spacez.unlock_stage1.extras.Moonlight.openApp(c, app, uuid) else com.spacez.unlock_stage1.extras.Moonlight.openDesktop(c)
            }.getOrElse { "Could not open the laptop (${it.message})" }
            log("open laptop app ${a.id}: ${if (uuid != null) "own host entry $app" else "Desktop (no host entry answered)"} after ${SystemClock.elapsedRealtime() - tapAt} ms")
            val what = if (uuid != null && msg.startsWith("Opening ${a.name}")) "${a.name}: opening it on the laptop" else if (uuid != null) "${a.name}: could not open that app directly, showing the laptop Desktop" else if (sent) "${a.name}: the PC did not answer in time, showing the laptop Desktop" else "The PC is not linked, so ${a.name} was not brought up; showing the laptop Desktop"
            android.widget.Toast.makeText(c, "$what ($msg)", android.widget.Toast.LENGTH_LONG).show()
            done?.invoke()
        }
        pendingGo = ::go
        thread(isDaemon = true) {
            com.spacez.unlock_stage1.away.AwayPhone.userAction()
            fun lk() = com.spacez.unlock_stage1.away.AwayPhone.link(c) ?: HandoffPhone.link(c)
            var l = lk()
            var waited = 0
            while ((l == null || !l.connected) && waited < 5000 && awaiting?.second === token) { Thread.sleep(200); waited += 200; l = lk() }
            val req = JSONObject().put("open", a.id).put("rid", java.util.UUID.randomUUID().toString().take(8)).toString() // same rid on the resend: the PC dedupes
            l?.send(Message.laptopApps(req))
            log("open laptop app ${a.id}: PC asked=${l != null} connected=${l?.connected} after waiting $waited ms")
            // no reply in 5 s: ask once more (a first message can be lost while the link wakes up)
            Thread.sleep(5000)
            if (awaiting?.second === token) { (lk() ?: l)?.send(Message.laptopApps(req)); log("open laptop app ${a.id}: no reply yet, asked again") }
            // a cold host start + app registration takes several seconds: the Desktop fallback comes 15 s after the first send
            Thread.sleep(10000)
            main.post { go(null, null) }
        }
    }
    private var awaiting: Pair<String, Any>? = null
    private var pendingGo: ((String?, String?) -> Unit)? = null

    /** The PC's answer to "open this laptop app": its host entry (name + uuid), or none. */
    private fun onOpened(o: JSONObject) {
        if (awaiting?.first != o.optString("opened")) return
        val app = o.optString("app").ifBlank { null }
        val uuid = o.optString("app_uuid").ifBlank { null }
        main.post { pendingGo?.invoke(app, uuid) }
    }

    private fun openTab(a: ChatHeadLogic.LApp, t: ChatHeadLogic.Tab) {
        val c = ctx ?: return
        val url = t.url ?: return
        if (acting) return
        acting = true
        fold(false)
        log("open tab ${HandoffLogic.host(url)} pos=${ChatHeadLogic.posFor(a, t)}")
        HandoffPhone.open(c, url, ChatHeadLogic.posFor(a, t), "chat head")
    }

    /** The laptop's own favicon of the tab's site (the PC reads it from the browser); a letter chip stays when it has none. Nothing is fetched from the internet. */
    private fun loadFavicon(url: String?, b: FrameLayout) {
        val host = ChatHeadLogic.hostOf(url).ifBlank { return }
        val bm = synchronized(favicons) { favicons[host] } ?: return
        (b.findViewWithTag<ImageView>("img"))?.apply { setImageBitmap(bm); b.findViewWithTag<View>("chip")?.visibility = View.GONE; b.background = round(Color.WHITE) }
    }

    private fun back() {
        if (level != 2) return
        val (cx, cy) = opened?.let { it.x + dp(26f) to it.y + dp(26f) } ?: headCentreInTree()
        foldAway(l2, cx, cy); l2 = emptyList()
        l1.forEach { it.animate().alpha(1f).scaleX(1f).scaleY(1f).setDuration(240).start() }
        l1.forEach { b -> b.isClickable = true; b.setOnClickListener { onApp(b) } }
        opened = null; level = 1
    }

    /** Fold the whole tree back into the head. [now] = without animation (hiding). */
    private fun fold(now: Boolean) {
        val t = tree ?: return
        val (cx, cy) = headCentreInTree()
        val all = l2 + l1
        level = 0; l1 = emptyList(); l2 = emptyList(); opened = null; tree = null
        fun finish() { runCatching { wm?.removeView(t) }; setUnseen(false); pendingApps?.let { apps = it; pendingApps = null } }
        if (now) { finish(); return }
        all.forEachIndexed { i, b -> b.animate().x(cx - dp(26f)).y(cy - dp(26f)).alpha(0f).scaleX(0.2f).scaleY(0.2f).setStartDelay(i * 20L).setDuration(240).start() }
        android.animation.ValueAnimator.ofInt(90, 0).apply { duration = 260; addUpdateListener { t.setBackgroundColor(Color.argb(it.animatedValue as Int, 7, 15, 28)) }; start() }
        main.postDelayed({ finish() }, 300)
    }

    // ------------------------------------------------------------------ the notification

    /** Bring the head back now (the "Show bubble" action; also turns the head on if it was switched off). */
    fun summon(c: Context) {
        ctx = c.applicationContext
        p(c).edit().putLong("until", 0).putBoolean("on", true).apply()
        attach(c)
        main.post { tick() }
        log("show bubble (notification)")
    }

    /** The ongoing notification "Aura chat head": always there while the head is switched on, with a "Show bubble" action. */
    fun postNotification(c: Context) {
        runCatching {
            val nm = c.getSystemService(NotificationManager::class.java) ?: return
            if (!enabled(c) || !permitted(c)) { nm.cancel(NID); return }
            nm.createNotificationChannel(NotificationChannel(NCH, "Aura chat head", NotificationManager.IMPORTANCE_LOW).apply { setShowBadge(false) })
            val show = PendingIntent.getBroadcast(c, NID, Intent(ACTION_SHOW).setPackage(c.packageName), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
            val n = Notification.Builder(c, NCH).setSmallIcon(R.drawable.ic_aura_laptop).setContentTitle("Aura chat head")
                .setContentText("Your laptop's apps, one tap away. Tap Show bubble if it is hidden")
                .setOngoing(true).setShowWhen(false).setOnlyAlertOnce(true).setCategory(Notification.CATEGORY_SERVICE)
                .addAction(Notification.Action.Builder(Icon.createWithResource(c, R.drawable.ic_aura_laptop), "Show bubble", show).build())
                .build()
            nm.notify(NID, n)
        }.onFailure { log("notification: ${it.message}") }
    }
}
