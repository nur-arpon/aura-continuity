package com.spacez.unlock_stage1

import android.app.NotificationManager
import android.content.Context
import android.media.AudioAttributes
import android.media.AudioManager
import android.media.SoundPool
import android.os.Handler
import android.os.Looper
import android.os.VibrationEffect
import android.os.VibrationEffect.Composition
import android.os.Vibrator
import android.os.VibratorManager
import android.util.Log

/**
 * Aura haptics + sounds (item 13), one place for the app, the widget and the tiles.
 *
 * Every action has its own pattern ([FeedbackPlan]); intensity is Max by default (owner: "extreme
 * haptics"). Rich [Composition] primitives where the phone supports them, else an amplitude
 * waveform with the same rhythm. Sounds are tiny synthesized WAVs (res/raw/aura_*.wav, made by
 * tools/sounds.py, no third-party assets) through a lazily created SoundPool that is released
 * again 20 s after the last sound. Silent in silent / vibrate mode and Do Not Disturb, and when
 * the Sounds switch is off. Nothing is scheduled: everything runs on a user action or a result.
 */
object Feedback {
    private const val TAG = "AuraFx"
    private fun prefs(c: Context) = c.getSharedPreferences("aura_feedback", Context.MODE_PRIVATE)

    /** 0 = off, 1 = light, 2 = medium, 3 = max (default). */
    fun intensity(c: Context) = prefs(c).getInt("haptic_level", 3)
    fun setIntensity(c: Context, v: Int) = prefs(c).edit().putInt("haptic_level", v.coerceIn(0, 3)).apply()
    fun soundOn(c: Context) = prefs(c).getBoolean("sound", true)
    fun setSound(c: Context, v: Boolean) = prefs(c).edit().putBoolean("sound", v).apply()

    fun play(c: Context, name: String) = FeedbackPlan.Cue.from(name)?.let { play(c, it) }

    fun play(c: Context, cue: FeedbackPlan.Cue) {
        val app = c.applicationContext
        runCatching { vibrate(app, cue) }.onFailure { Log.w(TAG, "haptic $cue: ${it.message}") }
        runCatching { sound(app, cue) }.onFailure { Log.w(TAG, "sound $cue: ${it.message}") }
    }

    // ---------------------------------------------------------------- haptics

    private val support = HashMap<Int, Boolean>()

    private fun vibrate(c: Context, cue: FeedbackPlan.Cue) {
        val level = intensity(c)
        if (level == 0) return
        val v: Vibrator = c.getSystemService(VibratorManager::class.java)?.defaultVibrator?.takeIf { it.hasVibrator() } ?: return
        val scale = FeedbackPlan.scale(level)
        val ps = FeedbackPlan.primitives(cue)
        val ok = android.os.Build.VERSION.SDK_INT >= 30 && ps.map { it.id }.distinct().all { id -> support.getOrPut(id) { v.areAllPrimitivesSupported(id) } }
        val effect = if (ok) {
            val comp = VibrationEffect.startComposition()
            ps.forEach { comp.addPrimitive(it.id, (it.s * scale).coerceIn(0f, 1f), it.delay) }
            comp.compose()
        } else {
            val w = FeedbackPlan.waveform(cue, scale)
            if (v.hasAmplitudeControl()) VibrationEffect.createWaveform(w.timings, w.amps, -1)
            else VibrationEffect.createWaveform(w.timings, -1)
        }
        v.vibrate(effect)
    }

    // ---------------------------------------------------------------- sounds

    private val main = Handler(Looper.getMainLooper())
    private var pool: SoundPool? = null
    private val ids = HashMap<Int, Int>()          // raw res -> sound id
    private val loaded = HashSet<Int>()
    private val waiting = HashMap<Int, Float>()      // sound id -> volume, played as soon as it loads
    private val releaseLater = Runnable { release() }

    private fun raw(cue: FeedbackPlan.Cue) = when (cue) {
        FeedbackPlan.Cue.TICK, FeedbackPlan.Cue.DETENT, FeedbackPlan.Cue.EDGE -> R.raw.aura_tick
        FeedbackPlan.Cue.TAP, FeedbackPlan.Cue.TOGGLE_ON, FeedbackPlan.Cue.TOGGLE_OFF, FeedbackPlan.Cue.SLIDE -> R.raw.aura_tap
        FeedbackPlan.Cue.LOCK -> R.raw.aura_lock
        FeedbackPlan.Cue.UNLOCK -> R.raw.aura_unlock
        FeedbackPlan.Cue.SEND -> R.raw.aura_send
        FeedbackPlan.Cue.ARRIVE -> R.raw.aura_arrive
        FeedbackPlan.Cue.SUCCESS, FeedbackPlan.Cue.APPROVED -> R.raw.aura_success
        FeedbackPlan.Cue.ERROR -> R.raw.aura_error
    }

    /** Silent / vibrate ringer or Do Not Disturb: no sound at all. */
    fun quietNow(c: Context): Boolean {
        val am = c.getSystemService(AudioManager::class.java) ?: return true
        val nm = c.getSystemService(NotificationManager::class.java)
        return FeedbackPlan.quiet(am.ringerMode == AudioManager.RINGER_MODE_NORMAL,
            nm == null || nm.currentInterruptionFilter == NotificationManager.INTERRUPTION_FILTER_ALL)
    }

    /** Preload while the app is open so the first press plays with no delay. */
    fun warm(c: Context) = main.post { if (soundOn(c) && !quietNow(c)) runCatching { ensurePool(c.applicationContext) } }

    @Synchronized private fun ensurePool(c: Context): SoundPool {
        pool?.let { return it }
        val p = SoundPool.Builder().setMaxStreams(3).setAudioAttributes(
            AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_ASSISTANCE_SONIFICATION)
                .setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION).build()).build()
        p.setOnLoadCompleteListener { sp, id, status ->
            if (status != 0) return@setOnLoadCompleteListener
            val vol = synchronized(this) { loaded += id; waiting.remove(id) }
            if (vol != null) sp.play(id, vol, vol, 1, 0, 1f)
        }
        FeedbackPlan.Cue.entries.map { raw(it) }.distinct().forEach { res -> ids[res] = p.load(c, res, 1) }
        pool = p
        return p
    }

    @Synchronized fun release() {
        main.removeCallbacks(releaseLater)
        pool?.release(); pool = null; ids.clear(); loaded.clear(); waiting.clear()
    }

    private fun sound(c: Context, cue: FeedbackPlan.Cue) {
        if (cue == FeedbackPlan.Cue.DETENT || cue == FeedbackPlan.Cue.EDGE) return   // felt, never heard (scrolling, the dial)
        main.post {
            val vol = FeedbackPlan.volume(cue)
            synchronized(this) {
                val p = ensurePool(c)
                val id = ids[raw(cue)] ?: return@post
                if (id in loaded) p.play(id, vol, vol, 1, 0, 1f) else waiting[id] = vol
            }
            main.removeCallbacks(releaseLater)
            main.postDelayed(releaseLater, 20_000)
        }
    }
}

/** Pure part of [Feedback] (unit-tested on the JVM): which pattern for which action. */
object FeedbackPlan {
    enum class Cue {
        TICK, TAP, TOGGLE_ON, TOGGLE_OFF, LOCK, UNLOCK, SEND, ARRIVE, SUCCESS, APPROVED, ERROR, SLIDE, DETENT, EDGE;
        companion object { fun from(name: String) = entries.firstOrNull { it.name.equals(name, ignoreCase = true) } }
    }

    data class P(val id: Int, val s: Float, val delay: Int = 0)
    class Wave(val timings: LongArray, val amps: IntArray)

    fun scale(level: Int) = when (level) { 1 -> 0.45f; 2 -> 0.72f; else -> 1f }

    fun quiet(ringerNormal: Boolean, interruptionsAll: Boolean) = !ringerNormal || !interruptionsAll

    fun volume(cue: Cue) = when (cue) {
        Cue.TICK -> 0.3f
        Cue.TAP, Cue.TOGGLE_ON, Cue.TOGGLE_OFF -> 0.5f
        else -> 0.9f
    }

    fun primitives(cue: Cue): List<P> = when (cue) {
        // Samsung's TICK / LOW_TICK primitives are ~20 ms and almost imperceptible (the dial "gave no
        // vibration" although every notch reached the motor): CLICK is the lightest one you can feel.
        Cue.TICK -> listOf(P(Composition.PRIMITIVE_CLICK, 0.55f))
        // a watch-bezel notch: crisp and short, so a fast turn still reads as separate clicks
        Cue.DETENT -> listOf(P(Composition.PRIMITIVE_CLICK, 0.7f))
        // hit the end of a list or the dial's stop: a soft wall
        Cue.EDGE -> listOf(P(Composition.PRIMITIVE_THUD, 0.6f), P(Composition.PRIMITIVE_LOW_TICK, 1f, 15))
        Cue.TAP -> listOf(P(Composition.PRIMITIVE_CLICK, 1f))
        Cue.TOGGLE_ON -> listOf(P(Composition.PRIMITIVE_QUICK_RISE, 0.7f), P(Composition.PRIMITIVE_CLICK, 1f, 10))
        Cue.TOGGLE_OFF -> listOf(P(Composition.PRIMITIVE_QUICK_FALL, 0.7f), P(Composition.PRIMITIVE_LOW_TICK, 1f, 10))
        // the door closes: a heavy thud, a hard latch click, a short settle
        Cue.LOCK -> listOf(P(Composition.PRIMITIVE_THUD, 1f), P(Composition.PRIMITIVE_CLICK, 1f, 30), P(Composition.PRIMITIVE_LOW_TICK, 0.8f, 40))
        // the door opens: a latch click, then a rising sweep that ends bright
        Cue.UNLOCK -> listOf(P(Composition.PRIMITIVE_CLICK, 1f), P(Composition.PRIMITIVE_QUICK_RISE, 1f, 40), P(Composition.PRIMITIVE_TICK, 1f, 30))
        // off it goes: a slow swell that falls away
        Cue.SEND -> listOf(P(Composition.PRIMITIVE_SLOW_RISE, 0.8f), P(Composition.PRIMITIVE_QUICK_FALL, 1f, 10), P(Composition.PRIMITIVE_TICK, 0.7f, 20))
        // something landed: a spin that settles with two clicks
        Cue.ARRIVE -> listOf(P(Composition.PRIMITIVE_SPIN, 0.6f), P(Composition.PRIMITIVE_CLICK, 1f, 30), P(Composition.PRIMITIVE_CLICK, 0.7f, 70))
        // celebration: spin, rising sweep, two bright clicks
        Cue.SUCCESS -> listOf(P(Composition.PRIMITIVE_SPIN, 0.8f), P(Composition.PRIMITIVE_QUICK_RISE, 1f, 40),
            P(Composition.PRIMITIVE_CLICK, 0.9f, 60), P(Composition.PRIMITIVE_CLICK, 1f, 70))
        Cue.APPROVED -> listOf(P(Composition.PRIMITIVE_CLICK, 1f), P(Composition.PRIMITIVE_CLICK, 1f, 90))
        // presenter: one decisive thump-click per slide
        Cue.SLIDE -> listOf(P(Composition.PRIMITIVE_THUD, 1f), P(Composition.PRIMITIVE_CLICK, 1f, 25))
        // unmistakable: thud and two low ticks
        Cue.ERROR -> listOf(P(Composition.PRIMITIVE_THUD, 1f), P(Composition.PRIMITIVE_LOW_TICK, 1f, 70), P(Composition.PRIMITIVE_LOW_TICK, 1f, 70))
    }

    fun waveform(cue: Cue, scale: Float): Wave {
        fun a(x: Int) = (x * scale).toInt().coerceIn(1, 255)
        fun w(vararg pairs: Pair<Long, Int>) = Wave(longArrayOf(0) + pairs.map { it.first }.toLongArray(), intArrayOf(0) + pairs.map { it.second }.toIntArray())
        return when (cue) {
            Cue.TICK -> w(14L to a(170))
            Cue.DETENT -> w(14L to a(255))
            Cue.EDGE -> w(30L to a(200), 15L to 0, 10L to a(150))
            Cue.TAP -> w(16L to a(220))
            Cue.TOGGLE_ON -> w(12L to a(140), 30L to 0, 18L to a(255))
            Cue.TOGGLE_OFF -> w(18L to a(255), 30L to 0, 10L to a(120))
            Cue.LOCK -> w(50L to a(255), 25L to 0, 18L to a(230), 30L to 0, 8L to a(150))
            Cue.UNLOCK -> w(14L to a(230), 40L to 0, 20L to a(120), 20L to a(190), 20L to a(255))
            Cue.SEND -> w(25L to a(80), 25L to a(150), 25L to a(230), 25L to a(120), 20L to a(60))
            Cue.ARRIVE -> w(20L to a(140), 40L to 0, 16L to a(255), 60L to 0, 14L to a(180))
            Cue.SUCCESS -> w(20L to a(110), 40L to 0, 25L to a(190), 40L to 0, 35L to a(255))
            Cue.APPROVED -> w(15L to a(240), 80L to 0, 15L to a(240))
            Cue.SLIDE -> w(40L to a(255), 20L to 0, 18L to a(255))
            Cue.ERROR -> w(70L to a(230), 50L to 0, 70L to a(230), 50L to 0, 70L to a(230))
        }
    }
}
