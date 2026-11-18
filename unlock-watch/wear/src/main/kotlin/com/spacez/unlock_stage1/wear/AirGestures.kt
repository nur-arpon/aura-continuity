package com.spacez.unlock_stage1.wear

import com.spacez.aura.link.MediaCmd
import com.spacez.aura.link.PresentCmd
import kotlin.math.abs
import kotlin.math.exp
import kotlin.math.hypot
import kotlin.math.max
import kotlin.math.min
import kotlin.math.sqrt

/*
 * Air control: everything that decides things lives here, pure Kotlin (no Android), unit-tested with synthetic sensor
 * traces in AirGesturesTest. The screen (AirControlScreen.kt) only feeds sensor events in and sends the results out.
 *
 * Units: accelerometer m/s^2, gyroscope rad/s (Android sensor frame: x = 3 o'clock, y = 12 o'clock, z = out of the face).
 */

/** One combined motion sample; [t] is a millisecond clock (any origin, only differences matter). */
data class Sample(val t: Long, val ax: Float, val ay: Float, val az: Float, val gx: Float, val gy: Float, val gz: Float)

enum class Gesture(val label: String, val real: Boolean) {
    FLICK_RIGHT("Flick right", true), FLICK_LEFT("Flick left", true),
    TWIST_OUT("Twist out", true), TWIST_IN("Twist in", true),
    SHAKE("Shake", true),
    KNOCK("Knock", true), DOUBLE_KNOCK("Double knock", true),
    CLENCH("Fist clench", false), COVER("Palm cover", false), RAISE("Raise wrist", false), PINCH("Double pinch", false),
}

data class Detected(val g: Gesture, val t: Long, val strength: Float)

// ------------------------------------------------------------------------------------------------ air mouse

/**
 * Gyro rates -> cursor pixels: low-pass smoothing, a dead zone, a gentle acceleration curve, gyro-bias learning and
 * "hold still to freeze" (tremor / drift is ignored once the wrist has been quiet for [stillAfterMs]).
 * yaw = turning the hand left (+) / right (-) about the world vertical; pitch = tipping the hand up (+) / down (-).
 */
class MotionFilter(
    var sensitivity: Float = 1f,
    var pxPerRad: Float = 1200f,
    var smoothingMs: Float = 40f,
    var deadZone: Float = 0.05f,
    var accel: Float = 0.6f,
    var invertX: Boolean = false,
    var invertY: Boolean = false,
    private val stillAfterMs: Long = 400,
) {
    private var sx = 0f; private var sy = 0f
    private var lastT = -1L
    private var biasX = 0f; private var biasY = 0f
    private var stillSince = -1L
    var still = false; private set

    fun reset() { sx = 0f; sy = 0f; lastT = -1L; stillSince = -1L; still = false }

    /** @return cursor movement in pixels since the previous call (fractions kept; the caller accumulates them). */
    fun feed(t: Long, yaw: Float, pitch: Float): Pair<Float, Float> {
        val dt = if (lastT < 0) 0f else min((t - lastT) / 1000f, 0.05f)
        lastT = t
        val rawX = -yaw * (if (invertX) -1f else 1f)
        val rawY = -pitch * (if (invertY) -1f else 1f)
        val a = if (dt <= 0f) 1f else 1f - exp(-dt * 1000f / smoothingMs)
        sx += a * (rawX - biasX - sx); sy += a * (rawY - biasY - sy)
        val speed = hypot(sx, sy)
        if (speed < 0.08f) {
            if (stillSince < 0) stillSince = t
            if (t - stillSince >= stillAfterMs) still = true
        } else if (speed > 0.18f) { stillSince = -1; still = false }
        else if (!still) stillSince = -1
        if (still) { biasX += 0.02f * (rawX - biasX); biasY += 0.02f * (rawY - biasY); return 0f to 0f }
        val v = speed - deadZone
        if (v <= 0f || dt <= 0f) return 0f to 0f
        val scale = v * (1f + accel * v) / speed * pxPerRad * sensitivity * dt
        return sx * scale to sy * scale
    }
}

// ------------------------------------------------------------------------------------------------ gestures

data class GestureTuning(
    /** gyro peak (rad/s) a flick / twist must reach. Air-mouse moves stay under ~3. */
    val flickRad: Float = 5f,
    /** accelerometer high-pass peak (m/s^2) of a knock */
    val knockMs2: Float = 3.5f,
    /** a knock is rejected when the gyro moved more than this (rad/s) in the last 300 ms: that is arm movement */
    val knockGyroMax: Float = 2f,
)

/**
 * Wrist gestures from accelerometer + gyroscope.
 *  - flick left/right: one fast turn of the hand about the vertical (the slower return stroke is ignored)
 *  - twist out/in: one fast turn of the forearm about its own axis
 *  - shake: three or more alternating fast swings within 700 ms (suppresses the flicks it is made of)
 *  - knock / double knock: a short accelerometer impulse while the arm is calm (table or case), single vs double
 *  - clench (best effort): sustained small vibration burst with a calm arm
 *  - raise (best effort): gravity swings from "arm down" to "face up toward me"
 */
class GestureDetector(var tuning: GestureTuning = GestureTuning()) {
    private class Swing(val sign: Int, val peak: Float, val t: Long)
    private class Axis {
        var count = 0; var lastSign = 0; var lastEnd = Long.MIN_VALUE / 2; var pending: Swing? = null
        fun reset() { count = 0; lastSign = 0; pending = null }
    }
    private inner class SwingDetector {
        var active = false; var sign = 0; var peak = 0f; var startT = 0L
        fun feed(t: Long, v: Float): Swing? {
            val hi = tuning.flickRad
            val s = if (v > 0) 1 else if (v < 0) -1 else 0
            if (!active) {
                if (abs(v) >= hi) { active = true; sign = s; peak = abs(v); startT = t }
                return null
            }
            if (s != sign && abs(v) >= hi) { // straight reversal: close the old swing, open the new one
                val done = if (t - startT <= 350) Swing(sign, peak, t) else null
                sign = s; peak = abs(v); startT = t
                return done
            }
            if (s == sign && abs(v) > peak) peak = abs(v)
            if (abs(v) < hi * 0.4f || s != sign) {
                active = false
                return if (t - startT <= 350) Swing(sign, peak, t) else null
            }
            return null
        }
    }

    private var lastT = -1L
    private var gx0 = 0f; private var gy0 = 0f; private var gz0 = 0f; private var gInit = false
    private var lpx = 0f; private var lpy = 0f; private var lpz = 0f
    private var phx = 0f; private var phy = 0f; private var phz = 0f
    private val yawSwing = SwingDetector(); private val rollSwing = SwingDetector()
    private val yawAxis = Axis(); private val rollAxis = Axis()
    private var peakYaw = 0f; private var peakRoll = 0f
    private var motionCooldownUntil = Long.MIN_VALUE
    private class Rec(val t: Long, val hp: Float, val gm: Float, val flip: Boolean)
    private val ring = ArrayDeque<Rec>()
    private var lastImpulse = Long.MIN_VALUE / 2
    private var pendingKnock = -1L
    private var knockCooldown = Long.MIN_VALUE
    private var clenchCooldown = Long.MIN_VALUE
    private var lowSince = -1L
    private var lastLow = Long.MIN_VALUE / 2
    private var raiseCooldown = Long.MIN_VALUE
    val live = Live()

    /** values for the test page */
    class Live { @Volatile var gyro = 0f; @Volatile var hp = 0f; @Volatile var yaw = 0f; @Volatile var roll = 0f }

    fun feed(s: Sample): List<Detected> {
        val out = ArrayList<Detected>(1)
        val dt = if (lastT < 0) 0.01f else min((s.t - lastT) / 1000f, 0.1f).coerceAtLeast(0.001f)
        lastT = s.t
        if (!gInit) { gx0 = s.ax; gy0 = s.ay; gz0 = s.az; lpx = s.ax; lpy = s.ay; lpz = s.az; gInit = true }
        // gravity (slow) and a short low-pass (to high-pass the knock out of)
        val ga = 1f - exp(-dt / 0.5f)
        gx0 += ga * (s.ax - gx0); gy0 += ga * (s.ay - gy0); gz0 += ga * (s.az - gz0)
        val hpx = s.ax - lpx; val hpy = s.ay - lpy; val hpz = s.az - lpz
        val hp = sqrt(hpx * hpx + hpy * hpy + hpz * hpz)
        val flip = hpx * phx + hpy * phy + hpz * phz < 0f
        phx = hpx; phy = hpy; phz = hpz
        val la = 1f - exp(-dt / 0.08f)
        lpx += la * (s.ax - lpx); lpy += la * (s.ay - lpy); lpz += la * (s.az - lpz)

        val gn = sqrt(gx0 * gx0 + gy0 * gy0 + gz0 * gz0)
        val yaw = if (gn > 1f) (s.gx * gx0 + s.gy * gy0 + s.gz * gz0) / gn else s.gz
        val roll = s.gy
        val gm = sqrt(s.gx * s.gx + s.gy * s.gy + s.gz * s.gz)
        live.gyro = gm; live.hp = hp; live.yaw = yaw; live.roll = roll
        val decay = exp(-dt / 0.15f)
        peakYaw = max(abs(yaw), peakYaw * decay); peakRoll = max(abs(roll), peakRoll * decay)

        // --- swings -> flick / twist / shake
        yawSwing.feed(s.t, yaw)?.let { onSwing(yawAxis, it, peakRoll, s.t, out) }
        rollSwing.feed(s.t, roll)?.let { onSwing(rollAxis, it, peakYaw, s.t, out) }
        for ((ax, isYaw) in listOf(yawAxis to true, rollAxis to false)) {
            val p = ax.pending ?: continue
            if (s.t - p.t >= 250) {
                ax.pending = null
                if (s.t >= motionCooldownUntil) {
                    val g = if (isYaw) (if (p.sign > 0) Gesture.FLICK_LEFT else Gesture.FLICK_RIGHT)
                            else (if (p.sign > 0) Gesture.TWIST_OUT else Gesture.TWIST_IN)
                    out += Detected(g, s.t, p.peak / tuning.flickRad)
                    motionCooldownUntil = s.t + 600
                }
            }
        }

        // --- knock / clench / raise
        ring.addLast(Rec(s.t, hp, gm, flip))
        while (ring.isNotEmpty() && s.t - ring.first().t > 320) ring.removeFirst()
        val gyroMax = ring.maxOf { it.gm }
        if (hp >= tuning.knockMs2 && gyroMax < tuning.knockGyroMax && s.t - lastImpulse > 120 && s.t >= knockCooldown) {
            lastImpulse = s.t
            if (pendingKnock >= 0 && s.t - pendingKnock in 120..600) {
                out += Detected(Gesture.DOUBLE_KNOCK, s.t, hp / tuning.knockMs2)
                pendingKnock = -1; knockCooldown = s.t + 500
            } else pendingKnock = s.t
        }
        if (pendingKnock >= 0 && s.t - pendingKnock >= 600) { out += Detected(Gesture.KNOCK, pendingKnock, 1f); pendingKnock = -1 }

        if (s.t >= clenchCooldown && gyroMax < 0.6f && ring.first().t <= s.t - 240) {
            val win = ring.filter { s.t - it.t <= 250 }
            val busy = win.count { it.hp in 0.5f..(tuning.knockMs2 * 0.8f) }
            if (win.size >= 10 && win.count { it.flip } >= 4 && busy >= win.size * 0.4f && win.none { it.hp >= tuning.knockMs2 * 0.8f }) {
                out += Detected(Gesture.CLENCH, s.t, busy.toFloat() / win.size); clenchCooldown = s.t + 1500
            }
        }

        if (gz0 < 3f) { lowSince = lowSince.takeIf { it >= 0 } ?: s.t; lastLow = s.t } else lowSince = -1
        if (gz0 > 6.5f && s.t - lastLow in 1..900 && s.t >= raiseCooldown) {
            out += Detected(Gesture.RAISE, s.t, 1f); raiseCooldown = s.t + 3000; lastLow = Long.MIN_VALUE / 2
        }
        return out
    }

    private fun onSwing(ax: Axis, sw: Swing, otherPeak: Float, now: Long, out: MutableList<Detected>) {
        if (sw.t - ax.lastEnd > 700) ax.reset()
        val p = ax.pending
        // the return stroke after a flick is weaker than the flick: not a second gesture
        if (p != null && sw.sign != p.sign && sw.peak < 0.7f * p.peak) { ax.lastEnd = sw.t; return }
        if (otherPeak > 0.7f * sw.peak) { ax.reset(); ax.lastEnd = sw.t; return } // diagonal / rotating: not a clean gesture
        ax.count = if (sw.sign == ax.lastSign) 1 else ax.count + 1
        ax.lastSign = sw.sign; ax.lastEnd = sw.t
        when {
            ax.count >= 3 -> {
                yawAxis.reset(); rollAxis.reset()
                if (now >= motionCooldownUntil) { out += Detected(Gesture.SHAKE, now, sw.peak / tuning.flickRad); motionCooldownUntil = now + 1200 }
            }
            ax.count == 2 -> ax.pending = null
            else -> ax.pending = sw
        }
    }
}

/** Palm over the screen: the ambient light collapses and stays down. Needs the light sensor (best effort). */
class CoverDetector(private val holdMs: Long = 350) {
    private var baseline = -1f
    private var lowSince = -1L
    private var armed = true
    fun feed(t: Long, lux: Float): Boolean {
        if (baseline < 0) { baseline = lux; return false }
        val low = baseline >= 20f && lux < max(1.5f, baseline * 0.12f)
        if (!low) {
            baseline += 0.05f * (lux - baseline)
            lowSince = -1
            if (lux > baseline * 0.4f) armed = true
            return false
        }
        if (lowSince < 0) lowSince = t
        if (armed && t - lowSince >= holdMs) { armed = false; return true }
        return false
    }
}

// ------------------------------------------------------------------------------------------------ bezel

/** Bezel scroll with speed: slow = line scroll, fast spin = page scroll (or bigger zoom steps). */
class RotarySpin(private val fastPxPerSec: Float = 1500f, private val base: Float = 2f, private val fastMult: Float = 6f) {
    private val win = ArrayDeque<Pair<Long, Float>>()
    var fast = false; private set

    /** @return PC wheel units (120 = one notch); clockwise (positive px) gives a negative value = scroll down. */
    fun feed(px: Float, t: Long): Int {
        win.addLast(t to abs(px))
        while (win.isNotEmpty() && t - win.first().first > 150) win.removeFirst()
        val v = win.sumOf { it.second.toDouble() }.toFloat() / 0.15f
        fast = v >= fastPxPerSec
        return (-px * base * (if (fast) fastMult else 1f)).toInt()
    }
}

// ------------------------------------------------------------------------------------------------ modes + mapping

enum class AirMode(val label: String, val legend: String) {
    MOUSE("Mouse", "Tilt your wrist to move · Tap to click · Turn bezel to scroll"),
    SLIDES("Slides", "Flick right = next · Flick left = back · Double-knock = next"),
    MEDIA("Media", "Flick = next/prev track · Double-knock = play/pause · Bezel = volume"),
    WINDOWS("Apps", "Twist = switch app · Shake = show desktop"),
}

/** What a gesture does on the PC. Sent through the existing INPUT / MEDIA_CMD / PRESENT messages. */
sealed class Act(val label: String) {
    data class Combo(val c: String, val name: String) : Act(name)
    data class Media(val cmd: Int, val name: String) : Act(name)
    data class Present(val cmd: Int, val name: String) : Act(name)
    data class Click(val right: Boolean) : Act(if (right) "Right click" else "Click")
    object DoubleClick : Act("Double click")
}

object AirMap {
    val ALT_TAB = Act.Combo("alt_tab", "Switch app")
    val ALT_SHIFT_TAB = Act.Combo("alt_shift_tab", "Switch app back")
    val WIN_TAB = Act.Combo("win_tab", "All windows")
    val WIN_D = Act.Combo("win_d", "Show desktop")
    val WIN_DOWN = Act.Combo("win_down", "Minimise window")

    /** The one thing a mode is mostly for: used by the double pinch and the double knock. */
    fun mainAction(m: AirMode): Act = when (m) {
        AirMode.MOUSE -> Act.Click(false)
        AirMode.SLIDES -> Act.Present(PresentCmd.NEXT, "Next slide")
        AirMode.MEDIA -> Act.Media(MediaCmd.PLAY_PAUSE, "Play / pause")
        AirMode.WINDOWS -> ALT_TAB
    }

    /** [swap] reverses the left/right and in/out sense for people who wear the watch differently. */
    fun forGesture(m: AirMode, g: Gesture, swap: Boolean = false): Act? {
        val gg = if (!swap) g else when (g) {
            Gesture.FLICK_LEFT -> Gesture.FLICK_RIGHT; Gesture.FLICK_RIGHT -> Gesture.FLICK_LEFT
            Gesture.TWIST_IN -> Gesture.TWIST_OUT; Gesture.TWIST_OUT -> Gesture.TWIST_IN
            else -> g
        }
        return when (gg) {
            Gesture.FLICK_RIGHT -> when (m) {
                AirMode.SLIDES -> Act.Present(PresentCmd.NEXT, "Next slide")
                AirMode.MEDIA -> Act.Media(MediaCmd.NEXT, "Next track")
                AirMode.WINDOWS -> WIN_TAB
                AirMode.MOUSE -> null
            }
            Gesture.FLICK_LEFT -> when (m) {
                AirMode.SLIDES -> Act.Present(PresentCmd.PREV, "Previous slide")
                AirMode.MEDIA -> Act.Media(MediaCmd.PREV, "Previous track")
                AirMode.WINDOWS -> WIN_DOWN
                AirMode.MOUSE -> null
            }
            Gesture.TWIST_OUT -> ALT_TAB
            Gesture.TWIST_IN -> ALT_SHIFT_TAB
            Gesture.SHAKE -> WIN_D
            Gesture.DOUBLE_KNOCK, Gesture.PINCH -> mainAction(m)
            Gesture.COVER, Gesture.CLENCH -> when (m) {
                AirMode.MEDIA -> Act.Media(MediaCmd.PLAY_PAUSE, "Play / pause")
                AirMode.SLIDES -> Act.Present(PresentCmd.BLACK, "Blank screen")
                else -> Act.Media(MediaCmd.MUTE, "Mute")
            }
            Gesture.KNOCK, Gesture.RAISE -> null // shown on the test page only
        }
    }

    /** Safe defaults: the deliberate gestures on; the ones that can fire by accident (cover, clench) off. */
    val DEFAULT_ON: Set<Gesture> = setOf(
        Gesture.FLICK_RIGHT, Gesture.FLICK_LEFT, Gesture.TWIST_OUT, Gesture.TWIST_IN,
        Gesture.SHAKE, Gesture.DOUBLE_KNOCK, Gesture.PINCH,
    )
}


// ------------------------------------------------------------------------------------------------ air mouse engine

/**
 * The Air mouse switch + everything between a gyro sample and an INPUT "move": OFF = nothing is ever emitted; ON = the
 * cursor follows the wrist continuously (no holding). A finger on the screen freezes the cursor (a tap jolts the wrist)
 * until [settleMs] after it lifts. [idleExpired] says when the switch should flip back OFF ([idleMs] without any use).
 * Pure Kotlin so the unit tests can drive it with synthetic gyro samples; the screen only sends what [feed] returns,
 * through the very same WatchAura.input path the touchpad uses.
 */
class AirMouse(
    val filter: MotionFilter = MotionFilter(),
    private val coalescer: MoveCoalescer = MoveCoalescer(gain = 1f, minGapMs = 16),
    private val idleMs: Long = 120_000,
    private val settleMs: Long = 250,
) {
    var on = false; private set
    private var lastActive = 0L
    private var touching = false
    private var freezeUntil = 0L

    fun setOn(v: Boolean, nowMs: Long) {
        on = v; lastActive = nowMs; touching = false; freezeUntil = 0L
        filter.reset(); coalescer.reset()
    }
    /** any use (tap, bezel, gesture) keeps the switch ON */
    fun poke(nowMs: Long) { lastActive = nowMs }
    fun touch(down: Boolean, nowMs: Long) { touching = down; if (!down) freezeUntil = nowMs + settleMs; poke(nowMs) }
    fun idleExpired(nowMs: Long) = on && nowMs - lastActive >= idleMs

    /** @return (dx, dy) to send now, or null. [yaw] / [pitch]: gyro rad/s about the vertical / the 3 o'clock axis. */
    fun feed(sensorMs: Long, yaw: Float, pitch: Float, nowMs: Long): Pair<Int, Int>? {
        if (!on) return null
        if (touching || nowMs < freezeUntil) { filter.reset(); return null }
        val (dx, dy) = filter.feed(sensorMs, yaw, pitch)
        val out = coalescer.feed(dx, dy, nowMs)
        if (out != null) lastActive = nowMs
        return out
    }
}
