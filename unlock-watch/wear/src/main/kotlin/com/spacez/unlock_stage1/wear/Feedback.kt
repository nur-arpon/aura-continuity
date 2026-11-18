package com.spacez.unlock_stage1.wear

import android.app.NotificationManager
import android.content.Context
import android.media.AudioAttributes
import android.media.AudioManager
import android.media.SoundPool
import android.os.VibrationEffect
import android.os.VibrationEffect.Composition
import android.os.Vibrator
import android.os.VibratorManager
import android.provider.Settings
import android.util.Log

/**
 * Haptics + sounds. Everything here runs only on an interaction or a real event (a PC unlock
 * answer): nothing is scheduled, nothing is held. The vibrator is used through rich
 * [Composition] primitives when the watch supports them, else a waveform with the same rhythm.
 *
 * Sounds are tiny synthesized WAVs (res/raw/cu_*.wav, made by unlock-watch/tools/sounds.py)
 * played through a lazily created SoundPool. They stay silent in silent / vibrate ringer mode,
 * Do Not Disturb, Theater mode and Bedtime/sleep, and when the Sound setting is off.
 */
object Feedback {
    enum class Cue { TICK, TAP, TOGGLE_ON, TOGGLE_OFF, LOCK, SUCCESS, APPROVED, ERROR, COUNTDOWN, SEND, SLIDE, DETENT }

    /** 0 = off, 1 = light, 2 = medium, 3 = max (default). */
    fun intensity(ctx: Context) = prefs(ctx).getInt("haptic_level", 3)
    fun setIntensity(ctx: Context, v: Int) = prefs(ctx).edit().putInt("haptic_level", v.coerceIn(0, 3)).apply()
    fun soundOn(ctx: Context) = prefs(ctx).getBoolean("sound", true)
    fun setSound(ctx: Context, v: Boolean) = prefs(ctx).edit().putBoolean("sound", v).apply()
    private fun prefs(ctx: Context) = ctx.getSharedPreferences("cu_feedback", Context.MODE_PRIVATE)

    fun play(ctx: Context, cue: Cue) {
        val app = ctx.applicationContext
        runCatching { vibrate(app, cue) }.onFailure { Log.w(TAG, "haptic $cue: ${it.message}") }
        runCatching { sound(app, cue) }
    }

    // ---------------------------------------------------------------- haptics

    private fun vibrator(ctx: Context): Vibrator? =
        ctx.getSystemService(VibratorManager::class.java)?.defaultVibrator?.takeIf { it.hasVibrator() }

    private fun vibrate(ctx: Context, cue: Cue) {
        val level = intensity(ctx)
        if (level == 0) return
        val v = vibrator(ctx) ?: return
        val scale = when (level) { 1 -> 0.45f; 2 -> 0.75f; else -> 1f }
        val effect = composition(v, cue, scale) ?: waveform(cue, scale)
        v.vibrate(effect)
    }

    private data class P(val id: Int, val s: Float, val delay: Int = 0)

    private fun primitives(cue: Cue): List<P> = when (cue) {
        Cue.TICK -> listOf(P(Composition.PRIMITIVE_TICK, 0.8f))
        // one rotary detent (volume step): a crisp click you can feel through a sleeve
        Cue.DETENT -> listOf(P(Composition.PRIMITIVE_CLICK, 0.9f), P(Composition.PRIMITIVE_TICK, 0.6f, 12))
        Cue.COUNTDOWN -> listOf(P(Composition.PRIMITIVE_TICK, 0.5f))
        Cue.TAP -> listOf(P(Composition.PRIMITIVE_CLICK, 0.8f))
        Cue.TOGGLE_ON -> listOf(P(Composition.PRIMITIVE_QUICK_RISE, 0.6f), P(Composition.PRIMITIVE_CLICK, 1f, 10))
        Cue.TOGGLE_OFF -> listOf(P(Composition.PRIMITIVE_QUICK_FALL, 0.6f), P(Composition.PRIMITIVE_LOW_TICK, 1f, 10))
        // the decisive "thunk": a heavy THUD with a sharp click on top
        Cue.LOCK -> listOf(P(Composition.PRIMITIVE_THUD, 1f), P(Composition.PRIMITIVE_CLICK, 1f, 30))
        // celebratory rise: spin, then a rising sweep and two bright clicks
        Cue.SUCCESS -> listOf(P(Composition.PRIMITIVE_SPIN, 0.7f), P(Composition.PRIMITIVE_QUICK_RISE, 1f, 40),
            P(Composition.PRIMITIVE_CLICK, 0.9f, 60), P(Composition.PRIMITIVE_CLICK, 1f, 70))
        // crisp double tap
        Cue.APPROVED -> listOf(P(Composition.PRIMITIVE_CLICK, 1f), P(Composition.PRIMITIVE_CLICK, 1f, 90))
        // slide change: the strongest cue on the watch, a thump and a hard click, felt through a sleeve
        Cue.SLIDE -> listOf(P(Composition.PRIMITIVE_THUD, 1f), P(Composition.PRIMITIVE_CLICK, 1f, 25))
        Cue.SEND -> listOf(P(Composition.PRIMITIVE_SLOW_RISE, 0.8f), P(Composition.PRIMITIVE_QUICK_FALL, 1f, 10), P(Composition.PRIMITIVE_TICK, 0.7f, 20))
        // distinct buzz: three low ticks with a thud
        Cue.ERROR -> listOf(P(Composition.PRIMITIVE_THUD, 0.8f), P(Composition.PRIMITIVE_LOW_TICK, 1f, 70), P(Composition.PRIMITIVE_LOW_TICK, 1f, 70))
    }

    private val support = HashMap<Int, Boolean>()

    private fun composition(v: Vibrator, cue: Cue, scale: Float): VibrationEffect? {
        val ps = primitives(cue)
        val ids = ps.map { it.id }.distinct()
        val ok = ids.all { id -> support.getOrPut(id) { v.areAllPrimitivesSupported(id) } }
        if (!ok) return null
        val c = VibrationEffect.startComposition()
        ps.forEach { c.addPrimitive(it.id, (it.s * scale).coerceIn(0f, 1f), it.delay) }
        return c.compose()
    }

    private fun waveform(cue: Cue, scale: Float): VibrationEffect {
        fun a(x: Int) = (x * scale).toInt().coerceIn(1, 255)
        return when (cue) {
            Cue.TICK, Cue.COUNTDOWN -> VibrationEffect.createOneShot(8, a(140))
            Cue.DETENT -> VibrationEffect.createOneShot(14, a(230))
            Cue.TAP -> VibrationEffect.createOneShot(14, a(170))
            Cue.TOGGLE_ON, Cue.TOGGLE_OFF -> VibrationEffect.createWaveform(longArrayOf(0, 12, 30, 18), intArrayOf(0, a(120), 0, a(220)), -1)
            Cue.LOCK -> VibrationEffect.createWaveform(longArrayOf(0, 45, 25, 15), intArrayOf(0, a(255), 0, a(200)), -1)
            Cue.SUCCESS -> VibrationEffect.createWaveform(longArrayOf(0, 20, 40, 25, 40, 35), intArrayOf(0, a(90), 0, a(170), 0, a(255)), -1)
            Cue.APPROVED -> VibrationEffect.createWaveform(longArrayOf(0, 15, 80, 15), intArrayOf(0, a(230), 0, a(230)), -1)
            Cue.SLIDE -> VibrationEffect.createWaveform(longArrayOf(0, 40, 20, 18), intArrayOf(0, a(255), 0, a(255)), -1)
            Cue.SEND -> VibrationEffect.createWaveform(longArrayOf(0, 25, 25, 25, 25, 20), intArrayOf(0, a(80), a(150), a(230), a(120), a(60)), -1)
            Cue.ERROR -> VibrationEffect.createWaveform(longArrayOf(0, 70, 50, 70, 50, 70), intArrayOf(0, a(200), 0, a(200), 0, a(200)), -1)
        }
    }

    // ---------------------------------------------------------------- sounds

    @Volatile private var pool: SoundPool? = null
    private val ids = HashMap<Cue, Int>()
    private val loaded = HashSet<Int>()

    private fun raw(cue: Cue) = when (cue) {
        Cue.TICK, Cue.COUNTDOWN, Cue.DETENT -> R.raw.cu_tick
        Cue.TAP, Cue.TOGGLE_ON, Cue.TOGGLE_OFF, Cue.SLIDE -> R.raw.cu_tap
        Cue.SEND -> R.raw.cu_success
        Cue.LOCK -> R.raw.cu_lock
        Cue.SUCCESS -> R.raw.cu_success
        Cue.APPROVED -> R.raw.cu_approved
        Cue.ERROR -> R.raw.cu_error
    }

    /** True when the watch is in a mode where an app should make no sound. */
    fun quietNow(ctx: Context): Boolean {
        val am = ctx.getSystemService(AudioManager::class.java) ?: return true
        if (am.ringerMode != AudioManager.RINGER_MODE_NORMAL) return true
        val nm = ctx.getSystemService(NotificationManager::class.java)
        if (nm != null && nm.currentInterruptionFilter != NotificationManager.INTERRUPTION_FILTER_ALL) return true
        // Wear OS Theater mode (screen and sounds off) is a global setting, not a ringer mode.
        if (Settings.Global.getInt(ctx.contentResolver, "theater_mode_on", 0) == 1) return true
        return false
    }

    @Synchronized private fun ensurePool(ctx: Context): SoundPool {
        pool?.let { return it }
        val p = SoundPool.Builder().setMaxStreams(3).setAudioAttributes(
            AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_ASSISTANCE_SONIFICATION)
                .setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION).build()).build()
        p.setOnLoadCompleteListener { _, id, status -> if (status == 0) synchronized(loaded) { loaded += id } }
        Cue.entries.map { raw(it) }.distinct().forEach { res ->
            val id = p.load(ctx, res, 1)
            Cue.entries.filter { raw(it) == res }.forEach { ids[it] = id }
        }
        pool = p
        return p
    }

    /** Preload while a screen is open so the first press plays with no delay. */
    fun warm(ctx: Context) { if (soundOn(ctx)) runCatching { ensurePool(ctx.applicationContext) } }

    /** Free the pool when the app goes to the background (nothing stays loaded). */
    @Synchronized fun release() { pool?.release(); pool = null; ids.clear(); synchronized(loaded) { loaded.clear() } }

    private fun sound(ctx: Context, cue: Cue) {
        if (!soundOn(ctx) || quietNow(ctx)) return
        val p = ensurePool(ctx)
        val id = ids[cue] ?: return
        val vol = when (cue) { Cue.TICK, Cue.COUNTDOWN, Cue.DETENT -> 0.35f; Cue.TAP, Cue.TOGGLE_ON, Cue.TOGGLE_OFF -> 0.55f; else -> 0.9f }
        if (synchronized(loaded) { id in loaded }) p.play(id, vol, vol, 1, 0, 1f)
    }
}
