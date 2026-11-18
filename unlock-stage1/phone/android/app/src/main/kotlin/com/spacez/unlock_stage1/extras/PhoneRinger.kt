package com.spacez.unlock_stage1.extras

import android.app.Activity
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.RadialGradient
import android.graphics.Shader
import android.media.AudioAttributes
import android.media.AudioManager
import android.media.MediaPlayer
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.os.VibrationEffect
import android.os.VibratorManager
import android.view.KeyEvent
import android.view.MotionEvent
import android.view.View
import android.view.WindowManager
import com.spacez.unlock_stage1.R
import com.spacez.unlock_stage1.UnlockState

/**
 * "Ring my phone" (item 15, PC tray -> phone): the Aura chime at MAX volume through the ALARM stream
 * (silent mode does not mute alarms), a strong repeating vibration, and a full-screen "touch anywhere
 * to stop" screen. Stops on any touch, any key, the notification's Stop, a stop from the PC, or after
 * 5 minutes. The alarm volume is put back afterwards.
 */
object PhoneRinger {
    const val CHANNEL = "aura_ring"
    const val NOTIF_ID = 4115
    const val ACTION_STOP = "com.spacez.unlock_stage1.RING_STOP"
    const val MAX_MS = 5 * 60_000L

    @Volatile var ringing = false; private set
    private var player: MediaPlayer? = null
    private var savedVolume = -1
    private val main = Handler(Looper.getMainLooper())
    private val timeout = Runnable { ctxRef?.let { stop(it, "5 minute limit") } }
    private var ctxRef: Context? = null

    /** Owner switch: Settings > Let my PC ring this phone (default on). */
    fun allowed(c: Context) = c.getSharedPreferences("aura_extras", 0).getBoolean("allow_ring", true)
    fun setAllowed(c: Context, on: Boolean) = c.getSharedPreferences("aura_extras", 0).edit().putBoolean("allow_ring", on).apply()

    /** Pure: the volume to ring at (always the maximum) and what to put back. */
    fun plan(current: Int, max: Int) = Plan(ringAt = max, restore = current.coerceIn(0, max))
    data class Plan(val ringAt: Int, val restore: Int)

    @Synchronized
    fun start(c: Context, from: String) {
        val app = c.applicationContext
        if (!allowed(app)) { UnlockState.log("ring from $from ignored: switched off in Settings"); return }
        if (ringing) return
        ringing = true; ctxRef = app
        UnlockState.log("ringing: asked by $from")
        val am = app.getSystemService(AudioManager::class.java)
        val plan = plan(am.getStreamVolume(AudioManager.STREAM_ALARM), am.getStreamMaxVolume(AudioManager.STREAM_ALARM))
        savedVolume = plan.restore
        runCatching { am.setStreamVolume(AudioManager.STREAM_ALARM, plan.ringAt, 0) }
        runCatching {
            val p = MediaPlayer()
            p.setAudioAttributes(AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_ALARM).setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION).build())
            app.resources.openRawResourceFd(R.raw.aura_ring).use { p.setDataSource(it.fileDescriptor, it.startOffset, it.length) }
            p.isLooping = true; p.setVolume(1f, 1f); p.prepare(); p.start()
            player = p
        }.onFailure { UnlockState.log("ring sound failed: ${it.message}") }
        runCatching {
            val v = app.getSystemService(VibratorManager::class.java).defaultVibrator
            v.vibrate(VibrationEffect.createWaveform(longArrayOf(0, 500, 250, 500, 250, 900, 600), intArrayOf(0, 255, 0, 255, 0, 255, 0), 0))
        }
        notify(app, from)
        // the screen: works from a locked phone via the full-screen intent; a direct start is a bonus
        runCatching { app.startActivity(Intent(app, RingActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_NO_ANIMATION)) }
        main.removeCallbacks(timeout); main.postDelayed(timeout, MAX_MS)
    }

    @Synchronized
    fun stop(c: Context, why: String) {
        if (!ringing) return
        ringing = false
        UnlockState.log("ring stopped: $why")
        main.removeCallbacks(timeout)
        runCatching { player?.stop(); player?.release() }; player = null
        val app = c.applicationContext
        runCatching { app.getSystemService(VibratorManager::class.java).defaultVibrator.cancel() }
        if (savedVolume >= 0) runCatching { app.getSystemService(AudioManager::class.java).setStreamVolume(AudioManager.STREAM_ALARM, savedVolume, 0) }
        savedVolume = -1
        runCatching { app.getSystemService(NotificationManager::class.java).cancel(NOTIF_ID) }
        app.sendBroadcast(Intent(RingActivity.CLOSE).setPackage(app.packageName))
    }

    private fun notify(app: Context, from: String) {
        val nm = app.getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(NotificationChannel(CHANNEL, "Ring my phone", NotificationManager.IMPORTANCE_HIGH).apply {
            setSound(null, null); enableVibration(false); lockscreenVisibility = Notification.VISIBILITY_PUBLIC
        })
        val flags = PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        val open = PendingIntent.getActivity(app, 1, Intent(app, RingActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK), flags)
        val stop = PendingIntent.getBroadcast(app, 2, Intent(ACTION_STOP).setPackage(app.packageName), flags)
        val n = Notification.Builder(app, CHANNEL)
            .setSmallIcon(R.drawable.ic_aura_glyph).setContentTitle("Your phone is ringing").setContentText("Asked by $from. Touch to stop.")
            .setCategory(Notification.CATEGORY_ALARM).setOngoing(true).setContentIntent(open).apply { if (!com.spacez.unlock_stage1.BuildConfig.PLAY) setFullScreenIntent(open, true) }
            .addAction(Notification.Action.Builder(null, "Stop", stop).build()).build()
        runCatching { nm.notify(NOTIF_ID, n) }
    }
}

/** Notification "Stop" button. */
class RingStopReceiver : android.content.BroadcastReceiver() {
    override fun onReceive(c: Context, i: Intent) { PhoneRinger.stop(c, "notification Stop") }
}

/** Full-screen "touch anywhere to stop" (plain views, no Flutter: it must appear in a blink, over the lock screen). */
class RingActivity : Activity() {
    private var rx: android.content.BroadcastReceiver? = null
    private var view: PulseView? = null

    override fun onCreate(b: Bundle?) {
        super.onCreate(b)
        setShowWhenLocked(true); setTurnScreenOn(true)
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        if (!PhoneRinger.ringing) { finish(); return }
        view = PulseView(this).also { setContentView(it) }
        rx = object : android.content.BroadcastReceiver() { override fun onReceive(c: Context, i: Intent) { finish() } }
        registerReceiver(rx, android.content.IntentFilter(CLOSE), RECEIVER_NOT_EXPORTED)
    }

    override fun dispatchTouchEvent(e: MotionEvent): Boolean {
        if (e.actionMasked == MotionEvent.ACTION_DOWN) { PhoneRinger.stop(this, "touch"); finish() }
        return true
    }

    override fun dispatchKeyEvent(e: KeyEvent): Boolean {
        if (e.action == KeyEvent.ACTION_DOWN) { PhoneRinger.stop(this, "key"); finish() }
        return true
    }

    override fun onDestroy() { runCatching { rx?.let(::unregisterReceiver) }; view?.stopAnim(); super.onDestroy() }

    companion object { const val CLOSE = "com.spacez.unlock_stage1.RING_CLOSE" }

    private class PulseView(c: Context) : View(c) {
        private val p = Paint(Paint.ANTI_ALIAS_FLAG)
        private val t = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = Color.WHITE; textAlign = Paint.Align.CENTER; isFakeBoldText = true }
        private val anim = android.animation.ValueAnimator.ofFloat(0f, 1f).apply {
            duration = 2100; repeatCount = android.animation.ValueAnimator.INFINITE; interpolator = android.view.animation.LinearInterpolator()
            addUpdateListener { invalidate() }; start()
        }
        fun stopAnim() = anim.cancel()
        override fun onDraw(cv: Canvas) {
            val w = width.toFloat(); val h = height.toFloat()
            p.shader = RadialGradient(w / 2, h * 0.3f, h * 0.9f, intArrayOf(0xFF17306B.toInt(), 0xFF0B1736.toInt(), 0xFF060D22.toInt()), floatArrayOf(0f, 0.55f, 1f), Shader.TileMode.CLAMP)
            cv.drawRect(0f, 0f, w, h, p); p.shader = null
            val cx = w / 2; val cy = h * 0.38f; val base = minOf(w, h) * 0.18f
            p.style = Paint.Style.STROKE; p.strokeWidth = 5f
            val f = anim.animatedFraction
            for (k in 0..2) {
                val q = (f + k / 3f) % 1f
                p.color = Color.argb((200 * (1 - q)).toInt(), 90, 162, 255)
                cv.drawCircle(cx, cy, base * (0.5f + 1.6f * q), p)
            }
            p.style = Paint.Style.FILL; p.color = 0xFF7DB4FF.toInt(); cv.drawCircle(cx, cy, base * 0.45f, p)
            p.color = 0xFF06122E.toInt(); cv.drawCircle(cx, cy, base * 0.18f, p)
            t.textSize = w * 0.085f; cv.drawText("Your phone is ringing", cx, h * 0.68f, t)
            t.isFakeBoldText = false; t.color = 0xFFA9BFE8.toInt(); t.textSize = w * 0.05f
            cv.drawText("Touch anywhere to stop", cx, h * 0.68f + w * 0.1f, t); t.color = Color.WHITE
        }
    }
}
