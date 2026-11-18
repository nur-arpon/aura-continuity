package com.spacez.unlock_stage1.away

import android.app.PendingIntent
import android.appwidget.AppWidgetManager
import android.appwidget.AppWidgetProvider
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.widget.RemoteViews
import com.spacez.unlock_stage1.MainActivity
import com.spacez.unlock_stage1.PcState
import com.spacez.unlock_stage1.R
import com.spacez.unlock_stage1.UnlockCrypto

/**
 * Aura home-screen widget (items 17 + 13): a status line (PC name, nearby / away / locked), the
 * "Continue from laptop" row (what the PC was last doing, at the exact second), and three
 * buttons: Lock, Unlock (fingerprint every time), Continue on PC.
 * No periodic updates (updatePeriodMillis = 0): redrawn when the PC answers, a lock / unlock
 * result arrives, or the app is opened.
 */
class AwayWidget : AppWidgetProvider() {
    override fun onUpdate(c: Context, m: AppWidgetManager, ids: IntArray) {
        ids.forEach { m.updateAppWidget(it, views(c)) }
    }

    companion object {
        fun views(c: Context): RemoteViews {
            val v = RemoteViews(c.packageName, R.layout.aura_away_widget)
            val paired = UnlockCrypto.pcId(c) != null
            val l = AwayPhone.last(c)
            val s = WidgetLogic.state(paired, PcState.locked(c), PcState.near(c), AwayPhone.pcAddr(c) != null && Tailnet.ownIp() != null)
            v.setTextViewText(R.id.aw_pc, PcState.name(c) ?: "Aura")
            v.setTextViewText(R.id.aw_state, s.text)
            v.setInt(R.id.aw_dot, "setColorFilter", c.getColor(when (s.tone) {
                WidgetLogic.Tone.GOOD -> R.color.aura_w_success
                WidgetLogic.Tone.WARN -> R.color.aura_w_warning
                WidgetLogic.Tone.INFO -> R.color.aura_w_primary
            }))
            // the PC is locked: Unlock becomes the primary button (same rule as the app)
            val unlockFirst = PcState.locked(c) == true
            fun style(bg: Int, icon: Int, label: Int, primary: Boolean) {
                v.setInt(bg, "setBackgroundResource", if (primary) R.drawable.aura_widget_btn_primary else R.drawable.aura_widget_btn)
                val fg = c.getColor(if (primary) R.color.aura_w_on_primary else R.color.aura_w_btn_text)
                v.setInt(icon, "setColorFilter", fg)
                v.setTextColor(label, fg)
            }
            style(R.id.aw_lock, R.id.aw_lock_icon, R.id.aw_lock_label, !unlockFirst)
            style(R.id.aw_unlock, R.id.aw_unlock_icon, R.id.aw_unlock_label, unlockFirst)
            val yt = paired && l?.s?.app == "youtube"
            v.setInt(R.id.aw_icon_bg, "setBackgroundResource", if (yt) R.drawable.aura_widget_icon_bg_yt else R.drawable.aura_widget_icon_bg)
            v.setImageViewResource(R.id.aw_icon, if (yt) R.drawable.ic_aura_play else R.drawable.ic_aura_laptop)
            v.setTextViewText(R.id.aw_title, if (!paired) "Set up Aura" else l?.title() ?: "Continue from laptop")
            v.setTextViewText(R.id.aw_sub, when {
                !paired -> "Pair with your PC in the app"
                l != null -> l.subtitle(System.currentTimeMillis())
                else -> "Tap to pick up where your PC left off"
            })
            val open = PendingIntent.getActivity(c, 10, Intent(c, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
                PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
            v.setOnClickPendingIntent(R.id.aw_head, open)
            v.setOnClickPendingIntent(R.id.aw_top, if (paired) AwayActionActivity.pending(c, "continue", 11) else open)
            v.setOnClickPendingIntent(R.id.aw_lock, AwayActionActivity.pending(c, "lock", 13))
            v.setOnClickPendingIntent(R.id.aw_unlock, AwayActionActivity.pending(c, "unlock", 14))
            v.setOnClickPendingIntent(R.id.aw_topc, AwayActionActivity.pending(c, "topc", 15))
            return v
        }

        fun updateAll(c: Context) = runCatching {
            val m = AppWidgetManager.getInstance(c)
            val ids = m.getAppWidgetIds(ComponentName(c, AwayWidget::class.java))
            if (ids.isNotEmpty()) m.updateAppWidget(ids, views(c))
        }
    }
}

/** The widget's status line (pure, unit-tested). Same wording as the app's status card. */
object WidgetLogic {
    enum class Tone { GOOD, INFO, WARN }
    data class State(val text: String, val tone: Tone)

    fun state(paired: Boolean, locked: Boolean?, near: Boolean, awayReady: Boolean): State = when {
        !paired -> State("Not paired", Tone.WARN)
        locked == true && near -> State("Locked · nearby", Tone.INFO)
        locked == true -> State("Locked", Tone.INFO)
        near -> State("Nearby", Tone.GOOD)
        awayReady -> State("Away · Tailscale", Tone.INFO)
        else -> State("Out of reach", Tone.WARN)
    }
}
