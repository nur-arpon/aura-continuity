package com.spacez.unlock_stage1.zerotap

import android.Manifest
import android.app.Activity
import android.content.ComponentName
import android.content.Intent
import android.graphics.Color
import android.graphics.drawable.GradientDrawable
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.provider.Settings
import android.view.Gravity
import android.view.ViewGroup
import android.widget.Button
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView

/**
 * The one-time explanation for zero-tap continue (item 19), shown before any system switch:
 * what the accessibility service reads (and what it never does), Usage access as the lighter
 * alternative, Contacts for WhatsApp chats. Each button opens the right system page; Aura never
 * flips a switch itself. Play Store note (for later, not the owner build): an app using the
 * AccessibilityService API for a non-accessibility purpose must declare it in the Play Console
 * (Permissions declaration form, "AccessibilityService API") and show this kind of prominent
 * disclosure + consent before enabling; see aura-anywhere/CONTINUE-RULES.md.
 */
class ZeroTapSetupActivity : Activity() {
    private lateinit var a11y: Button
    private lateinit var usage: Button
    private lateinit var contacts: Button

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val dp = resources.displayMetrics.density
        fun px(v: Int) = (v * dp).toInt()
        val navy = Color.parseColor("#0B1A33")
        window.statusBarColor = navy
        val col = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL; setPadding(px(24), px(32), px(24), px(32)); setBackgroundColor(navy) }
        fun text(s: String, size: Float, bold: Boolean = false, color: String = "#EAF1FF") = TextView(this).apply {
            text = s; textSize = size; setTextColor(Color.parseColor(color)); if (bold) setTypeface(typeface, android.graphics.Typeface.BOLD)
            setPadding(0, px(6), 0, px(6)); setLineSpacing(0f, 1.15f)
        }
        fun button(s: String, onClick: () -> Unit) = Button(this).apply {
            text = s; isAllCaps = false; textSize = 16f; setTextColor(Color.WHITE)
            background = GradientDrawable().apply { cornerRadius = px(24).toFloat(); setColor(Color.parseColor("#1D5FD0")) }
            layoutParams = LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, px(52)).apply { topMargin = px(12) }
            setOnClickListener { onClick() }
        }
        col.addView(text("Zero-tap continue", 26f, true))
        col.addView(text("Open YouTube, your browser, Facebook, Messenger, WhatsApp or Claude on this phone and it carries on what your PC was showing, at the same second. Focus your PC and it carries on what this phone was showing. No taps. A small Undo chip shows for 2 seconds.", 16f, color = "#C9D7F0"))
        col.addView(text("What Aura reads", 18f, true))
        col.addView(text("Only in those apps: which one is in front, its window title and labels (a chat name, a page address, a conversation title). Never what you type, never messages, never other apps. Nothing is stored or logged; that one title or link goes only to your own PC, end-to-end encrypted over Aura Link. The only thing Aura does in those apps is go Back when you press Undo.", 15f, color = "#C9D7F0"))
        a11y = button("Turn on Aura in Accessibility") { open(accessibilityIntent()) }
        col.addView(a11y)
        col.addView(text("Lighter option: Usage access only tells Aura which app opened (no titles). PC to phone then works, phone to PC only for YouTube and music.", 14f, color = "#9FB6DD"))
        usage = button("Allow Usage access") { open(Intent(Settings.ACTION_USAGE_ACCESS_SETTINGS, Uri.parse("package:$packageName"))) }
        col.addView(usage)
        col.addView(text("WhatsApp: the same chat on your PC needs the contact's number. Aura looks the name up in your contacts on this phone.", 14f, color = "#9FB6DD"))
        contacts = button("Allow contacts (WhatsApp chats)") { requestPermissions(arrayOf(Manifest.permission.READ_CONTACTS), 19) }
        col.addView(contacts)
        col.addView(button("Done") { finish() })
        setContentView(ScrollView(this).apply { setBackgroundColor(navy); addView(col) })
        ZeroTap.setExplained(this)
    }

    private fun accessibilityIntent(): Intent {
        if (Build.VERSION.SDK_INT >= 33) {
            val cn = ComponentName(this, AuraAccessibility::class.java).flattenToString()
            return Intent("android.settings.ACCESSIBILITY_DETAILS_SETTINGS").putExtra(Intent.EXTRA_COMPONENT_NAME, cn)
        }
        return Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS)
    }

    private fun open(i: Intent) {
        runCatching { startActivity(i) }.onFailure { runCatching { startActivity(Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS)) } }
    }

    override fun onResume() {
        super.onResume()
        fun mark(b: Button, on: Boolean, label: String) { b.text = if (on) "$label: on" else label; b.alpha = if (on) 0.6f else 1f }
        mark(a11y, AuraAccessibility.on(this), "Turn on Aura in Accessibility")
        mark(usage, Foreground.granted(this), "Allow Usage access")
        mark(contacts, Contacts.granted(this), "Allow contacts (WhatsApp chats)")
    }

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, grantResults: IntArray) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        onResume()
    }
}
