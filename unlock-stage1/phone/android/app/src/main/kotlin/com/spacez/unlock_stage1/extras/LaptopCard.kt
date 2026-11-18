package com.spacez.unlock_stage1.extras

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.graphics.BitmapFactory
import android.os.Handler
import android.os.Looper
import com.spacez.aura.link.MediaCmd

/**
 * "Now playing everywhere": what plays on the laptop shows here as an ongoing media-style notification
 * ("Laptop - song") with previous / play-pause / next. The PC pushes its state when its sound starts or stops (and
 * while this phone asked for updates); while the laptop plays, the phone renews that request every 100 s, nothing else.
 */
object LaptopCard {
    private const val CH = "aura_laptop_media"
    private const val ID = 7411
    private const val ACTION = "com.spacez.unlock_stage1.LAPTOP_MEDIA"
    private val main = Handler(Looper.getMainLooper())
    @Volatile private var shownKey: String? = null
    @Volatile private var dismissedKey: String? = null
    private var renew: Runnable? = null

    fun refresh(c: Context) {
        val m = PcRemote.media
        val ctx = c.applicationContext
        val nm = ctx.getSystemService(NotificationManager::class.java)
        if (m == null || m.title.isBlank()) { hide(nm); return }
        val key = m.title + "\u0001" + m.artist
        if (!m.playing && dismissedKey == key) { hide(nm); return }
        if (m.playing) dismissedKey = null
        nm.createNotificationChannel(NotificationChannel(CH, "Laptop now playing", NotificationManager.IMPORTANCE_LOW).apply { setShowBadge(false) })
        fun act(label: String, cmd: Int, code: Int) = Notification.Action.Builder(null, label,
            PendingIntent.getBroadcast(ctx, code, Intent(ctx, LaptopMediaReceiver::class.java).setAction(ACTION).putExtra("cmd", cmd), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)).build()
        val del = PendingIntent.getBroadcast(ctx, 9, Intent(ctx, LaptopMediaReceiver::class.java).setAction(ACTION).putExtra("cmd", -1), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        val b = Notification.Builder(ctx, CH).setSmallIcon(com.spacez.unlock_stage1.R.drawable.ic_aura_glyph)
            .setContentTitle(m.title).setContentText("Laptop · " + m.artist.ifBlank { "Aura" }).setSubText("Aura")
            .setOngoing(m.playing).setOnlyAlertOnce(true).setShowWhen(false).setDeleteIntent(del)
            .setStyle(Notification.MediaStyle().setShowActionsInCompactView(0, 1, 2))
            .addAction(act("Previous", MediaCmd.PREV, 1)).addAction(act(if (m.playing) "Pause" else "Play", MediaCmd.PLAY_PAUSE, 2)).addAction(act("Next", MediaCmd.NEXT, 3))
        PcRemote.art?.let { runCatching { BitmapFactory.decodeByteArray(it, 0, it.size) }.getOrNull()?.let(b::setLargeIcon) }
        nm.notify(ID, b.build())
        shownKey = key
        scheduleRenew(ctx, m.playing)
    }

    private fun hide(nm: NotificationManager) {
        if (shownKey != null) runCatching { nm.cancel(ID) }
        shownKey = null; cancelRenew()
    }

    private fun cancelRenew() { renew?.let(main::removeCallbacks); renew = null }

    private fun scheduleRenew(c: Context, playing: Boolean) {
        cancelRenew()
        if (!playing) return
        val r = object : Runnable {
            override fun run() {
                if (PcRemote.media?.playing != true || shownKey == null) return
                Thread { PcRemote.mediaCmd(c, MediaCmd.REFRESH) }.start()
                main.postDelayed(this, 100_000)
            }
        }
        renew = r; main.postDelayed(r, 100_000)
    }

    /** Swiped away: do not bring this track back until it plays again. */
    fun onDismissed() { dismissedKey = shownKey; shownKey = null; cancelRenew() }
}

class LaptopMediaReceiver : BroadcastReceiver() {
    override fun onReceive(c: Context, i: Intent) {
        val cmd = i.getIntExtra("cmd", 0)
        if (cmd == -1) { LaptopCard.onDismissed(); return }
        val app = c.applicationContext
        val pr = goAsync()
        Thread {
            // the PC pushes the effect only to a screen that asked for updates: ask first, then act
            PcRemote.mediaCmd(app, MediaCmd.REFRESH)
            PcRemote.mediaCmd(app, cmd)
            pr.finish()
        }.start()
    }
}
