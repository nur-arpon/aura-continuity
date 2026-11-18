package com.spacez.unlock_stage1

import android.content.Context
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.LinearGradient
import android.graphics.Paint
import android.graphics.Shader
import android.view.View
import android.widget.FrameLayout
import android.widget.TextView
import com.spacez.unlock_stage1.away.AwayWidget
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import java.io.File

/**
 * Item 13: renders the REAL home-screen widget (AwayWidget.views -> RemoteViews.apply, the same
 * code path a launcher uses) on a phone-wallpaper backdrop, light and dark, and saves the PNGs
 * to unlock-stage1/phone/screenshots/widget-*.png for review. Also checks the texts.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class WidgetRenderTest {
    private val out = File("../../screenshots").apply { mkdirs() }

    private fun seed(c: Context, paired: Boolean = true, locked: Boolean = false, near: Boolean = true) {
        c.getSharedPreferences("unlock", 0).edit().apply { if (paired) putString("pc_id", "0123456789abcdef") else remove("pc_id") }.commit()
        c.getSharedPreferences("aura_pc", 0).edit().apply { if (paired) putString("name", "MYLAPTOP") else remove("name") }.putBoolean("near", near)
            .putInt("locked", if (locked) 1 else 0).putLong("locked_at", System.currentTimeMillis()).commit()
        c.getSharedPreferences("away", 0).edit()
            .putString("last_json", """{"app":"youtube","title":"Slugterra S2E7 - The Return of the Shane Gang","url":"https://www.youtube.com/watch?v=abc","pos_ms":872000,"playing":false,"at_ms":${System.currentTimeMillis()},"pc_locked":$locked}""")
            .putLong("last_at", System.currentTimeMillis()).commit()
    }

    private fun render(name: String, wDp: Int = 360, hDp: Int = 184): View {
        val c: Context = RuntimeEnvironment.getApplication()
        val d = c.resources.displayMetrics.density
        val parent = FrameLayout(c)
        val v = AwayWidget.views(c).apply(c, parent)
        val w = (wDp * d).toInt(); val h = (hDp * d).toInt(); val pad = (16 * d).toInt()
        v.measure(View.MeasureSpec.makeMeasureSpec(w, View.MeasureSpec.EXACTLY), View.MeasureSpec.makeMeasureSpec(h, View.MeasureSpec.EXACTLY))
        v.layout(0, 0, w, h)
        val bmp = Bitmap.createBitmap(w + 2 * pad, h + 2 * pad, Bitmap.Config.ARGB_8888)
        val cv = Canvas(bmp)
        // a calm wallpaper so the card's edges and corners are visible
        val dark = name.contains("dark")
        cv.drawPaint(Paint().apply {
            shader = LinearGradient(0f, 0f, bmp.width.toFloat(), bmp.height.toFloat(),
                if (dark) 0xFF1B2440.toInt() else 0xFFB9C9E6.toInt(), if (dark) 0xFF3A2A4A.toInt() else 0xFFE8D9C9.toInt(), Shader.TileMode.CLAMP)
        })
        cv.translate(pad.toFloat(), pad.toFloat())
        v.draw(cv)
        File(out, "widget-$name.png").outputStream().use { bmp.compress(Bitmap.CompressFormat.PNG, 100, it) }
        return v
    }

    private fun text(v: View, id: Int) = (v.findViewById<TextView>(id)).text.toString()

    @Test @Config(qualifiers = "en-rUS-w411dp-h891dp-notnight-xxhdpi")
    fun nearby_light() {
        seed(RuntimeEnvironment.getApplication())
        val v = render("nearby-light")
        assertEquals("MYLAPTOP", text(v, R.id.aw_pc))
        assertEquals("Nearby", text(v, R.id.aw_state))
        assertEquals("Slugterra S2E7 - The Return of the Shane Gang", text(v, R.id.aw_title))
        assertEquals("YouTube · 14:32", text(v, R.id.aw_sub))
        assertEquals("Lock", text(v, R.id.aw_lock_label))
        assertEquals("Unlock", text(v, R.id.aw_unlock_label))
        assertEquals("On PC", text(v, R.id.aw_topc_label))
    }

    @Test @Config(qualifiers = "en-rUS-w411dp-h891dp-night-xxhdpi")
    fun nearby_dark() {
        seed(RuntimeEnvironment.getApplication())
        render("nearby-dark")
    }

    @Test @Config(qualifiers = "en-rUS-w411dp-h891dp-night-xxhdpi")
    fun locked_away_dark() {
        seed(RuntimeEnvironment.getApplication(), locked = true, near = false)
        val v = render("locked-dark")
        assertEquals("Locked", text(v, R.id.aw_state))
    }

    @Test @Config(qualifiers = "en-rUS-w411dp-h891dp-notnight-xxhdpi")
    fun not_paired_light() {
        seed(RuntimeEnvironment.getApplication(), paired = false)
        val v = render("unpaired-light", wDp = 300, hDp = 170)
        assertEquals("Set up Aura", text(v, R.id.aw_title))
        assertEquals("Not paired", text(v, R.id.aw_state))
    }

    /** The launcher icon as a squircle-ish circle mask, and the Android 13 themed (monochrome) version. */
    @Test @Config(qualifiers = "en-rUS-w411dp-h891dp-notnight-xxhdpi")
    fun launcher_icon() {
        val c: Context = RuntimeEnvironment.getApplication()
        val px = 324; val gap = 48
        val bmp = Bitmap.createBitmap(px * 2 + gap * 3, px + gap * 2, Bitmap.Config.ARGB_8888)
        val cv = Canvas(bmp)
        cv.drawColor(0xFFE9EEF5.toInt())
        fun draw(x: Int, vararg ids: Int, tint: Int? = null, bg: Int? = null) {
            val path = android.graphics.Path().apply { addCircle(x + px / 2f, gap + px / 2f, px / 2f, android.graphics.Path.Direction.CW) }
            cv.save(); cv.clipPath(path)
            bg?.let { cv.drawColor(it) }
            // adaptive icons: 108 dp canvas, the visible mask is the middle 72 dp -> scale 1.5 and centre
            val full = (px * 1.5f).toInt(); val off = (full - px) / 2
            ids.forEach { id ->
                val d = c.getDrawable(id)!!.mutate()
                tint?.let { t -> d.setTint(t) }
                d.setBounds(x - off, gap - off, x - off + full, gap - off + full); d.draw(cv)
            }
            cv.restore()
        }
        draw(gap, R.drawable.ic_aura_bg, R.drawable.ic_aura_fg)
        draw(gap * 2 + px, R.drawable.ic_aura_mono, tint = 0xFF1B3A6B.toInt(), bg = 0xFFD6E3FF.toInt())
        File(out, "icon-launcher-and-themed.png").outputStream().use { bmp.compress(Bitmap.CompressFormat.PNG, 100, it) }
    }
}
