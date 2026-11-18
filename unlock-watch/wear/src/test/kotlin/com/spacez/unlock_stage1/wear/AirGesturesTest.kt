package com.spacez.unlock_stage1.wear

import com.spacez.aura.link.MediaCmd
import com.spacez.aura.link.PresentCmd
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** Synthetic sensor traces (100 Hz, watch face up so gravity is +z) for the pure Air control logic. */
class AirGesturesTest {
    /** Builds a trace: [f] gets the time in ms and returns the (gx, gy, gz, az-extra, ax-extra) of that instant. */
    private class Trace(val det: GestureDetector = GestureDetector()) {
        var t = 0L
        val got = ArrayList<Gesture>()
        fun run(ms: Int, gx: (Long) -> Float = { 0f }, gy: (Long) -> Float = { 0f }, gz: (Long) -> Float = { 0f },
                az: (Long) -> Float = { 9.8f }, ay: (Long) -> Float = { 0f }) {
            var i = 0
            while (i < ms) {
                val l = t
                det.feed(Sample(t, 0f, ay(l), az(l), gx(l), gy(l), gz(l))).forEach { got += it.g }
                t += 10; i += 10
            }
        }
        fun calm(ms: Int) = run(ms)
    }

    private fun pulse(from: Long, len: Long, v: Float): (Long) -> Float = { if (it in from until from + len) v else 0f }

    // ---------------------------------------------------------------------------- flicks / twists / shake
    @Test fun clean_flick_right_is_one_event_and_the_return_stroke_is_ignored() {
        val tr = Trace(); tr.calm(500)
        val s = tr.t
        tr.run(900, gz = { t -> pulse(s, 80, -8f)(t) + pulse(s + 120, 150, 2f)(t) })
        assertEquals(listOf(Gesture.FLICK_RIGHT), tr.got)
    }

    @Test fun flick_left_and_twists_have_the_right_sense() {
        val a = Trace(); a.calm(300); val sa = a.t
        a.run(700, gz = pulse(sa, 80, 8f)); assertEquals(listOf(Gesture.FLICK_LEFT), a.got)
        val b = Trace(); b.calm(300); val sb = b.t
        b.run(700, gy = pulse(sb, 80, 8f)); assertEquals(listOf(Gesture.TWIST_OUT), b.got)
        val c = Trace(); c.calm(300); val sc = c.t
        c.run(700, gy = pulse(sc, 80, -8f)); assertEquals(listOf(Gesture.TWIST_IN), c.got)
    }

    @Test fun shake_is_one_event_and_swallows_its_own_flicks() {
        val tr = Trace(); tr.calm(300)
        val s = tr.t
        tr.run(1500, gz = { t -> val k = (t - s) / 120; if (t >= s && k < 5 && (t - s) % 120 < 70) (if (k % 2 == 0L) 8f else -8f) else 0f })
        assertEquals(listOf(Gesture.SHAKE), tr.got)
    }

    @Test fun slow_mouse_like_motion_is_not_a_gesture() {
        val tr = Trace(); tr.calm(200)
        tr.run(2000, gz = { t -> 2.5f * Math.sin(t / 150.0).toFloat() }, gx = { 1.5f })
        assertTrue(tr.got.isEmpty())
    }

    @Test fun diagonal_swing_is_rejected() {
        val tr = Trace(); tr.calm(300); val s = tr.t
        tr.run(700, gz = pulse(s, 80, 8f), gy = pulse(s, 80, 8f))
        assertTrue(tr.got.isEmpty())
    }

    @Test fun flicks_have_a_cooldown() {
        val tr = Trace(); tr.calm(300); val s = tr.t
        tr.run(1500, gz = { t -> pulse(s, 70, -8f)(t) + pulse(s + 450, 70, -8f)(t) })
        assertEquals(1, tr.got.size)
    }

    @Test fun flick_threshold_is_tunable() {
        val tr = Trace(GestureDetector(GestureTuning(flickRad = 9f))); tr.calm(300); val s = tr.t
        tr.run(700, gz = pulse(s, 80, 8f))
        assertTrue(tr.got.isEmpty())
    }

    // ---------------------------------------------------------------------------- knocks
    private fun spike(at: Long) = { t: Long -> if (t in at until at + 20) 9.8f + 7f else 9.8f }

    @Test fun single_knock_waits_for_a_possible_second() {
        val tr = Trace(); tr.calm(500); val s = tr.t
        tr.run(300, az = spike(s)); assertTrue("must still wait", tr.got.isEmpty())
        tr.run(600); assertEquals(listOf(Gesture.KNOCK), tr.got)
    }

    @Test fun double_knock_fires_at_once_and_only_once() {
        val tr = Trace(); tr.calm(500); val s = tr.t
        tr.run(1500, az = { t -> spike(s)(t) + spike(s + 250)(t) - 9.8f })
        assertEquals(listOf(Gesture.DOUBLE_KNOCK), tr.got)
    }

    @Test fun impulse_while_the_arm_is_moving_is_not_a_knock() {
        val tr = Trace(); tr.calm(500); val s = tr.t
        tr.run(1200, az = spike(s + 300), gz = { 4f })
        assertTrue(tr.got.none { it == Gesture.KNOCK || it == Gesture.DOUBLE_KNOCK })
    }

    @Test fun slow_arm_acceleration_is_not_a_knock() {
        val tr = Trace(); tr.calm(300); val s0 = tr.t
        tr.run(3000, az = { t -> 9.8f + 4f * Math.sin((t - s0) / 160.0).toFloat() })
        assertTrue(tr.got.toString(), tr.got.isEmpty())
    }

    @Test fun sustained_buzz_is_a_clench_but_a_knock_is_not() {
        val tr = Trace(); tr.calm(500); val s = tr.t
        tr.run(800, az = { t -> if (t >= s) 9.8f + (if ((t / 10) % 2 == 0L) 0.9f else -0.9f) else 9.8f })
        assertTrue(Gesture.CLENCH in tr.got)
        val k = Trace(); k.calm(500); val sk = k.t
        k.run(1500, az = spike(sk)); assertTrue(Gesture.CLENCH !in k.got)
    }

    @Test fun raising_the_wrist_is_noticed() {
        val tr = Trace(); tr.calm(100)
        // arm hanging: gravity along y, then face up over 400 ms
        val s0 = tr.t
        tr.run(2400, az = { t -> if (t < s0 + 1500) 0.5f else 9.8f }, ay = { t -> if (t < s0 + 1500) 9.8f else 0f })
        assertTrue(tr.got.toString(), Gesture.RAISE in tr.got)
    }

    // ---------------------------------------------------------------------------- air mouse filter
    @Test fun mouse_follows_rotation_direction_and_stops_when_still() {
        val f = MotionFilter()
        var sx = 0f
        for (i in 0 until 100) sx += f.feed(i * 10L, yaw = -1f, pitch = 0f).first   // hand turning right
        assertTrue("moved right ($sx)", sx > 300f)
        var sy = 0f
        val g = MotionFilter()
        for (i in 0 until 100) sy += g.feed(i * 10L, yaw = 0f, pitch = 1f).second   // hand tipping up
        assertTrue("moved up ($sy)", sy < -300f)
    }

    @Test fun dead_zone_and_hold_still_freeze_the_cursor() {
        val f = MotionFilter()
        var total = 0f
        for (i in 0 until 300) total += Math.abs(f.feed(i * 10L, yaw = 0.03f, pitch = 0.02f).first)   // tremor
        assertEquals(0f, total, 0.001f)
        assertTrue(f.still)
        // a deliberate move leaves the frozen state at once
        var moved = 0f
        for (i in 300 until 340) moved += Math.abs(f.feed(i * 10L, yaw = 1f, pitch = 0f).first)
        assertTrue(moved > 50f); assertTrue(!f.still)
    }

    @Test fun faster_wrist_gets_more_than_proportional_pixels_and_sensitivity_scales() {
        fun dist(rate: Float, sens: Float): Float { val f = MotionFilter(sensitivity = sens); var s = 0f; for (i in 0 until 60) s += f.feed(i * 10L, -rate, 0f).first; return s }
        assertTrue(dist(2f, 1f) > 2f * dist(1f, 1f) * 0.99f * 0.8f)
        assertEquals(dist(1f, 1f) * 2f, dist(1f, 2f), dist(1f, 1f) * 0.01f)
    }

    // ---------------------------------------------------------------------------- cover + bezel + mapping
    @Test fun palm_cover_fires_once_after_the_hold_and_not_for_slow_dimming() {
        val c = CoverDetector(); var n = 0
        var t = 0L
        repeat(200) { if (c.feed(t, 300f)) n++; t += 10 }
        repeat(100) { if (c.feed(t, 0.5f)) n++; t += 10 }
        assertEquals(1, n)
        repeat(100) { if (c.feed(t, 300f)) n++; t += 10 }
        repeat(100) { if (c.feed(t, 0.5f)) n++; t += 10 }
        assertEquals(2, n)
        val d = CoverDetector(); var m = 0; var lux = 300f
        repeat(400) { lux = maxOf(60f, lux - 1f); if (d.feed(it * 10L, lux)) m++ }
        assertEquals(0, m)
        val dark = CoverDetector(); var k = 0
        repeat(300) { if (dark.feed(it * 10L, if (it < 100) 3f else 0.5f)) k++ }
        assertEquals("dark room: nothing to cover", 0, k)
    }

    @Test fun fast_bezel_spin_scrolls_a_page_slow_scrolls_lines() {
        val r = RotarySpin()
        var t = 0L
        val slow = r.feed(-20f, t); assertTrue(!r.fast); assertEquals(40, slow)
        val q = RotarySpin(); var last = 0
        for (i in 0 until 8) { last = q.feed(-40f, i * 15L) }
        assertTrue(q.fast); assertEquals(480, last)
    }

    @Test fun mapping_per_mode() {
        assertEquals(Act.Present(PresentCmd.NEXT, "Next slide"), AirMap.forGesture(AirMode.SLIDES, Gesture.FLICK_RIGHT))
        assertEquals(Act.Present(PresentCmd.PREV, "Previous slide"), AirMap.forGesture(AirMode.SLIDES, Gesture.FLICK_LEFT))
        assertEquals(Act.Media(MediaCmd.NEXT, "Next track"), AirMap.forGesture(AirMode.MEDIA, Gesture.FLICK_RIGHT))
        assertEquals(null, AirMap.forGesture(AirMode.MOUSE, Gesture.FLICK_RIGHT))
        assertEquals(AirMap.ALT_TAB, AirMap.forGesture(AirMode.MOUSE, Gesture.TWIST_OUT))
        assertEquals(AirMap.ALT_SHIFT_TAB, AirMap.forGesture(AirMode.MEDIA, Gesture.TWIST_IN))
        assertEquals(AirMap.WIN_D, AirMap.forGesture(AirMode.SLIDES, Gesture.SHAKE))
        assertEquals(Act.Click(false), AirMap.forGesture(AirMode.MOUSE, Gesture.PINCH))
        assertEquals(Act.Media(MediaCmd.PLAY_PAUSE, "Play / pause"), AirMap.forGesture(AirMode.MEDIA, Gesture.DOUBLE_KNOCK))
        assertEquals(Act.Present(PresentCmd.NEXT, "Next slide"), AirMap.forGesture(AirMode.SLIDES, Gesture.DOUBLE_KNOCK))
        assertEquals(AirMap.forGesture(AirMode.SLIDES, Gesture.FLICK_LEFT), AirMap.forGesture(AirMode.SLIDES, Gesture.FLICK_RIGHT, swap = true))
        assertTrue(Gesture.COVER !in AirMap.DEFAULT_ON && Gesture.CLENCH !in AirMap.DEFAULT_ON)
    }
}
