package com.spacez.unlock_stage1.zerotap

import android.accessibilityservice.AccessibilityService
import android.content.Context
import android.graphics.PixelFormat
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import android.view.Gravity
import android.view.LayoutInflater
import android.view.View
import android.view.WindowManager
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityNodeInfo
import android.widget.TextView

/**
 * Zero-tap continue (item 19). Limited to the owner's apps (res/xml/aura_accessibility.xml lists
 * the packages; the system delivers nothing else). Two jobs:
 *  1. "app came to the front" events (TYPE_WINDOW_STATE_CHANGED), the trigger for PC -> phone;
 *  2. the top labels of that window (view ids, titles, content descriptions only, never typed text
 *     fields other than a browser's address bar) -> [ScreenReader] -> one state for the PC.
 * Nothing is stored or logged; the only action is Back when the owner presses Undo. It also draws the
 * 2-second "Undo / Pick another" chip as an accessibility overlay (no overlay permission needed).
 */
class AuraAccessibility : AccessibilityService() {
    companion object {
        @Volatile var instance: AuraAccessibility? = null
        fun on(c: Context): Boolean {
            val flat = Settings.Secure.getString(c.contentResolver, Settings.Secure.ENABLED_ACCESSIBILITY_SERVICES) ?: return false
            return flat.split(':').any { it.startsWith(c.packageName + "/") && it.endsWith("AuraAccessibility") }
        }
    }

    private val main = Handler(Looper.getMainLooper())
    private var lastPkg: String? = null
    private var readPending: String? = null
    private val readTask = Runnable { readPending?.let(::readWindow); readPending = null }
    private var chip: View? = null

    override fun onServiceConnected() { instance = this; ZeroTap.log("accessibility on (owner's apps only)") }
    override fun onUnbind(intent: android.content.Intent?): Boolean { instance = null; hideChip(); return super.onUnbind(intent) }
    override fun onInterrupt() {}

    override fun onAccessibilityEvent(e: AccessibilityEvent) {
        val pkg = e.packageName?.toString() ?: return
        if (pkg !in ContinueRules.WATCHED) return
        if (e.eventType == AccessibilityEvent.TYPE_WINDOW_STATE_CHANGED && pkg != lastPkg) {
            lastPkg = pkg
            ZeroTap.onForeground(applicationContext, pkg, "accessibility")
        }
        // what the window shows, read at most once per 1.5 s of changes (settled), only if anyone wants it
        if (ZeroTap.phoneToPc(applicationContext) && ContinueRules.appKind(pkg) !in setOf("youtube", "music", "spotify")) {
            readPending = pkg
            main.removeCallbacks(readTask)
            main.postDelayed(readTask, 1_500)
        }
    }

    private fun readWindow(pkg: String) {
        val root = rootInActiveWindow ?: return
        if (root.packageName?.toString() != pkg) return
        val nodes = ArrayList<ScreenReader.Node>()
        fun walk(n: AccessibilityNodeInfo?, depth: Int) {
            if (n == null || nodes.size >= 60 || depth > 12) return
            // never read what the owner types (except a browser's address bar, which ScreenReader needs)
            val editable = n.isEditable && !(n.viewIdResourceName ?: "").let { it.endsWith("location_bar_edit_text") || it.endsWith("url_bar") }
            if (!editable && (n.text != null || n.contentDescription != null || n.viewIdResourceName != null)) {
                nodes.add(ScreenReader.Node(n.viewIdResourceName ?: "", n.text?.toString()?.take(200), n.contentDescription?.toString()?.take(200)))
            }
            for (i in 0 until n.childCount) walk(n.getChild(i), depth + 1)
        }
        walk(root, 0)
        ScreenReader.read(pkg, nodes)?.let { ZeroTap.onScreen(applicationContext, pkg, it) }
    }

    // ------------------------------------------------------------------ chip

    fun showChip(title: String, onUndo: () -> Unit, onPick: (() -> Unit)?, ms: Long) {
        main.post {
            hideChip()
            val wm = getSystemService(WindowManager::class.java)
            val v = LayoutInflater.from(this).inflate(com.spacez.unlock_stage1.R.layout.aura_zt_chip, null)
            v.findViewById<TextView>(com.spacez.unlock_stage1.R.id.zt_title).text = title
            v.findViewById<View>(com.spacez.unlock_stage1.R.id.zt_undo).setOnClickListener { hideChip(); onUndo() }
            val pick = v.findViewById<View>(com.spacez.unlock_stage1.R.id.zt_pick)
            if (onPick == null) pick.visibility = View.GONE else pick.setOnClickListener { hideChip(); onPick() }
            val lp = WindowManager.LayoutParams(
                WindowManager.LayoutParams.MATCH_PARENT, WindowManager.LayoutParams.WRAP_CONTENT,
                WindowManager.LayoutParams.TYPE_ACCESSIBILITY_OVERLAY,
                WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or WindowManager.LayoutParams.FLAG_LAYOUT_IN_SCREEN,
                PixelFormat.TRANSLUCENT).apply { gravity = Gravity.BOTTOM; y = 180 }
            runCatching { wm.addView(v, lp); chip = v }
            v.alpha = 0f; v.translationY = 40f
            v.animate().alpha(1f).translationY(0f).setDuration(220).start()
            main.postDelayed({ if (chip === v) hideChip() }, ms)
        }
    }

    fun hideChip() {
        val v = chip ?: return
        chip = null
        runCatching { getSystemService(WindowManager::class.java).removeView(v) }
    }

    /** Undo on the phone: leave the app that was just opened (Back), the PC resumes. */
    fun back() = performGlobalAction(GLOBAL_ACTION_BACK)
}
