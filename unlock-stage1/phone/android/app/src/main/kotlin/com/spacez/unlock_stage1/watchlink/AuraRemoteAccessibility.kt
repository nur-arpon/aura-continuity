package com.spacez.unlock_stage1.watchlink

import android.accessibilityservice.AccessibilityService
import android.accessibilityservice.GestureDescription
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.graphics.Color
import android.graphics.Path
import android.graphics.PixelFormat
import android.graphics.drawable.GradientDrawable
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import android.util.Log
import android.view.Gravity
import android.view.View
import android.view.WindowManager
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityNodeInfo
import com.google.android.gms.wearable.Wearable
import org.json.JSONObject

private const val TAG = "AuraWatchCtl"

/**
 * Watch controls the phone (FUTURE-PLANS item 7). A second accessibility service, separate from [com.spacez.unlock_stage1.zerotap.AuraAccessibility]
 * (whose package filter and narrow event list stay untouched): this one asks for NO events at all, so it hears nothing from any app;
 * it only acts when the paired watch sends INPUT-shaped JSON on PATH_PHONE_INPUT:
 *   move {dx,dy} -> the on-screen cursor; click {b:left} -> tap at the cursor; click {b:right} -> Back (Android's mouse convention);
 *   scroll {dy} (PC wheel units, 120 = a notch, negative = down) -> a swipe at the cursor; key {k: back|home|recents|enter|backspace};
 *   text {s} -> typed into the focused field (ACTION_SET_TEXT, clipboard paste as the fallback).
 */
class AuraRemoteAccessibility : AccessibilityService() {
    companion object {
        @Volatile var instance: AuraRemoteAccessibility? = null
        fun on(c: Context): Boolean {
            val flat = Settings.Secure.getString(c.contentResolver, Settings.Secure.ENABLED_ACCESSIBILITY_SERVICES) ?: return false
            return flat.split(':').any { it.startsWith(c.packageName + "/") && it.endsWith("AuraRemoteAccessibility") }
        }
    }

    private val main = Handler(Looper.getMainLooper())
    private var pointer: PhonePointer? = null
    private var cursor: View? = null
    private var lp: WindowManager.LayoutParams? = null
    private val hide = Runnable { setCursorVisible(false) }
    private val r = 22

    override fun onServiceConnected() { instance = this; Log.i(TAG, "watch control service on") }
    override fun onUnbind(intent: Intent?): Boolean { instance = null; main.post { removeCursor() }; return super.onUnbind(intent) }
    override fun onAccessibilityEvent(e: AccessibilityEvent) {}
    override fun onInterrupt() {}

    private fun ptr(): PhonePointer {
        val wm = getSystemService(WindowManager::class.java)
        val b = wm.maximumWindowMetrics.bounds
        return pointer?.takeIf { it.w == b.width() && it.h == b.height() } ?: PhonePointer(b.width(), b.height()).also { pointer = it }
    }

    /** Runs on the main thread. Returns a one-word result for the log / tests. */
    fun handle(o: JSONObject): String = when (o.optString("t")) {
        "move" -> { val p = ptr(); p.move(o.optInt("dx"), o.optInt("dy")); showCursor(p); "move" }
        "click" -> {
            val p = ptr(); showCursor(p)
            if (o.optString("b") == "right") { performGlobalAction(GLOBAL_ACTION_BACK); "back" }
            else { tapAt(p.x, p.y); pulse(); "tap" }
        }
        "scroll" -> { val p = ptr(); p.scrollSwipe(o.optInt("dy"))?.let { swipe(it[0], it[1], it[2], it[3]); showCursor(p); "scroll" } ?: "none" }
        "key" -> key(o.optString("k"))
        "text" -> typeText(o.optString("s").take(500))
        else -> "ignored"
    }

    private fun key(k: String): String = when (k) {
        "back" -> { performGlobalAction(GLOBAL_ACTION_BACK); k }
        "home" -> { performGlobalAction(GLOBAL_ACTION_HOME); k }
        "recents" -> { performGlobalAction(GLOBAL_ACTION_RECENTS); k }
        "backspace" -> editFocused { text, a, b -> if (a != b) text.removeRange(minOf(a, b), maxOf(a, b)) to minOf(a, b) else if (a > 0) text.removeRange(a - 1, a) to a - 1 else text to a }
        "enter" -> {
            val f = rootInActiveWindow?.findFocus(AccessibilityNodeInfo.FOCUS_INPUT)
            if (f != null && android.os.Build.VERSION.SDK_INT >= 30 && f.performAction(AccessibilityNodeInfo.AccessibilityAction.ACTION_IME_ENTER.id)) "enter" else "enter-nofield"
        }
        else -> "ignored"
    }

    private fun tapAt(x: Float, y: Float) = dispatch(Path().apply { moveTo(x, y) }, 60)
    private fun swipe(x0: Float, y0: Float, x1: Float, y1: Float) = dispatch(Path().apply { moveTo(x0, y0); lineTo(x1, y1) }, 220)
    private fun dispatch(path: Path, ms: Long) {
        val g = GestureDescription.Builder().addStroke(GestureDescription.StrokeDescription(path, 0, ms)).build()
        if (!dispatchGesture(g, null, null)) Log.w(TAG, "dispatchGesture refused")
    }

    /** Insert / edit text in the focused editable: new text = f(old, selStart, selEnd) -> (text, caret). */
    private fun editFocused(f: (String, Int, Int) -> Pair<String, Int>): String {
        val n = rootInActiveWindow?.findFocus(AccessibilityNodeInfo.FOCUS_INPUT) ?: return "no-field"
        if (!n.isEditable) return "no-field"
        val cur = if (n.isShowingHintText) "" else n.text?.toString() ?: ""
        val a = if (n.textSelectionStart in 0..cur.length) n.textSelectionStart else cur.length
        val b = if (n.textSelectionEnd in 0..cur.length) n.textSelectionEnd else cur.length
        val (t, caret) = f(cur, a, b)
        val ok = n.performAction(AccessibilityNodeInfo.ACTION_SET_TEXT, Bundle().apply { putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE, t) })
        if (ok) n.performAction(AccessibilityNodeInfo.ACTION_SET_SELECTION, Bundle().apply {
            putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_START_INT, caret); putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_END_INT, caret) })
        return if (ok) "text" else "set-text-refused"
    }

    private fun typeText(s: String): String {
        if (s.isEmpty()) return "empty"
        val res = editFocused { t, a, b -> val lo = minOf(a, b); val hi = maxOf(a, b); (t.substring(0, lo) + s + t.substring(hi)) to (lo + s.length) }
        if (res == "text") return res
        // fallback: clipboard paste into whatever has input focus
        val n = rootInActiveWindow?.findFocus(AccessibilityNodeInfo.FOCUS_INPUT) ?: return res
        val cm = getSystemService(ClipboardManager::class.java)
        cm.setPrimaryClip(ClipData.newPlainText("aura", s))
        return if (n.performAction(AccessibilityNodeInfo.ACTION_PASTE)) "paste" else res
    }

    // ------------------------------------------------------------------ cursor overlay (accessibility overlay: no overlay permission)

    private fun showCursor(p: PhonePointer) {
        if (cursor == null) {
            val v = View(this).apply {
                background = GradientDrawable().apply { shape = GradientDrawable.OVAL; setColor(Color.argb(200, 80, 160, 255)); setStroke(4, Color.WHITE) }
            }
            val params = WindowManager.LayoutParams(r * 2, r * 2, WindowManager.LayoutParams.TYPE_ACCESSIBILITY_OVERLAY,
                WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE or
                    WindowManager.LayoutParams.FLAG_LAYOUT_IN_SCREEN or WindowManager.LayoutParams.FLAG_LAYOUT_NO_LIMITS, PixelFormat.TRANSLUCENT)
                .apply { gravity = Gravity.TOP or Gravity.START }
            runCatching { getSystemService(WindowManager::class.java).addView(v, params); cursor = v; lp = params }
        }
        val v = cursor ?: return
        val params = lp ?: return
        params.x = (p.x - r).toInt(); params.y = (p.y - r).toInt()
        runCatching { getSystemService(WindowManager::class.java).updateViewLayout(v, params) }
        v.visibility = View.VISIBLE; v.alpha = 1f
        main.removeCallbacks(hide); main.postDelayed(hide, 4_000)
    }

    private fun pulse() { cursor?.animate()?.scaleX(0.6f)?.scaleY(0.6f)?.setDuration(70)?.withEndAction { cursor?.animate()?.scaleX(1f)?.scaleY(1f)?.setDuration(90)?.start() }?.start() }
    private fun setCursorVisible(on: Boolean) { cursor?.visibility = if (on) View.VISIBLE else View.INVISIBLE }
    private fun removeCursor() { cursor?.let { v -> runCatching { getSystemService(WindowManager::class.java).removeView(v) } }; cursor = null }
}

/** Receives the watch's phone-input messages (called from [WatchLinkService]) and answers with the service state. */
object PhoneControl {
    private const val CH = "aura_watch_control"
    private const val NOTE_ID = 4417
    @Volatile private var lastNoteMs = 0L
    @Volatile private var lastStateMs = 0L
    @Volatile var lastResult = ""; private set
    @Volatile var received = 0L; private set

    fun onInput(ctx: Context, node: String, data: ByteArray) {
        val o = runCatching { JSONObject(String(data, Charsets.UTF_8)) }.getOrNull() ?: return
        val svc = AuraRemoteAccessibility.instance
        if (svc == null || !AuraRemoteAccessibility.on(ctx)) {
            reply(ctx, node, "a11y_off", force = true)
            offerEnable(ctx)
            return
        }
        if (o.optString("t") == "hello") { reply(ctx, node, "ok", force = true); return }
        received++
        Handler(Looper.getMainLooper()).post {
            lastResult = runCatching { svc.handle(o) }.getOrElse { "error ${it.message}" }
            if (o.optString("t") != "move") Log.i(TAG, "watch ${o.optString("t")} -> $lastResult")
        }
        if (System.currentTimeMillis() - lastStateMs > 30_000) reply(ctx, node, "ok")
    }

    private fun reply(ctx: Context, node: String, state: String, force: Boolean = false) {
        val now = System.currentTimeMillis()
        if (!force && now - lastStateMs < 3_000) return
        lastStateMs = now
        Wearable.getMessageClient(ctx).sendMessage(node, WatchProtocol.PATH_PHONE_INPUT_STATE, state.toByteArray())
    }

    /** One-tap card: opens the accessibility settings where "Aura watch control" is switched on. At most once a minute. */
    private fun offerEnable(ctx: Context) {
        val now = System.currentTimeMillis()
        if (now - lastNoteMs < 60_000) return
        lastNoteMs = now
        val nm = ctx.getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(NotificationChannel(CH, "Watch control", NotificationManager.IMPORTANCE_DEFAULT))
        val pi = PendingIntent.getActivity(ctx, 0, Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        val n = Notification.Builder(ctx, CH).setSmallIcon(com.spacez.unlock_stage1.R.drawable.ic_aura_glyph)
            .setContentTitle("Turn on Aura watch control").setContentText("Your watch wants to tap and type on this phone. Tap here, then switch on \"Aura watch control\".")
            .setContentIntent(pi).setAutoCancel(true).setTimeoutAfter(10 * 60_000L).build()
        runCatching { nm.notify(NOTE_ID, n) }
    }
}
