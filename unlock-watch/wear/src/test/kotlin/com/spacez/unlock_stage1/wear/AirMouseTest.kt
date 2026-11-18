package com.spacez.unlock_stage1.wear

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Synthetic 100 Hz gyro samples (watch face up, gravity +z) through the same chain the screen uses: GestureDetector.live.yaw / gx -> AirMouse -> INPUT move. */
class AirMouseTest {
    private class Rig(val m: AirMouse = AirMouse(idleMs = 1000)) {
        val det = GestureDetector()
        var t = 0L
        var sx = 0; var sy = 0; var moves = 0
        fun run(ms: Int, gx: Float = 0f, gz: Float = 0f) {
            var i = 0
            while (i < ms) {
                val s = Sample(t, 0f, 0f, 9.8f, gx, 0f, gz)
                det.feed(s)
                m.feed(s.t, det.live.yaw, s.gx, t)?.let { (x, y) -> sx += x; sy += y; moves++ }
                t += 10; i += 10
            }
        }
    }

    @Test fun off_by_default_emits_nothing_even_with_a_big_wrist_turn() {
        val r = Rig(); r.run(300); r.run(500, gz = 1.5f, gx = 1f)
        assertEquals(0, r.moves)
    }

    @Test fun on_and_turning_the_wrist_emits_moves_without_any_holding() {
        val r = Rig(); r.run(300); r.m.setOn(true, r.t)
        r.run(500, gz = 1.0f)   // hand turned left at 1 rad/s -> cursor goes left
        assertTrue("moves=${r.moves}", r.moves > 10)
        assertTrue("sx=${r.sx}", r.sx < -100)
        assertEquals(0, r.sy)
    }

    @Test fun tipping_the_hand_moves_the_cursor_vertically() {
        val r = Rig(); r.run(300); r.m.setOn(true, r.t)
        r.run(500, gx = 0.8f)   // hand tipped up -> cursor goes up (negative y)
        assertTrue("sy=${r.sy}", r.sy < -100)
    }

    @Test fun a_still_wrist_and_tiny_drift_send_nothing() {
        val r = Rig(); r.run(300); r.m.setOn(true, r.t)
        r.run(2000, gz = 0.02f, gx = -0.02f)
        assertEquals(0, r.moves)
    }

    @Test fun finger_on_screen_freezes_the_cursor_and_it_resumes_after_it_lifts() {
        val r = Rig(); r.run(300); r.m.setOn(true, r.t)
        r.m.touch(true, r.t); r.run(400, gz = 1.0f)
        assertEquals(0, r.moves)
        r.m.touch(false, r.t); r.run(200, gz = 1.0f)   // still inside the 250 ms settle
        assertEquals(0, r.moves)
        r.run(500, gz = 1.0f)
        assertTrue(r.moves > 5)
    }

    @Test fun switching_off_stops_it_and_switching_on_again_works() {
        val r = Rig(); r.run(300); r.m.setOn(true, r.t); r.run(300, gz = 1f)
        val n = r.moves; assertTrue(n > 0)
        r.m.setOn(false, r.t); r.run(300, gz = 1f)
        assertEquals(n, r.moves)
        r.m.setOn(true, r.t); r.run(400, gz = 1f)
        assertTrue(r.moves > n)
    }

    @Test fun idle_switch_off_after_the_idle_time_and_use_keeps_it_on() {
        val m = AirMouse(idleMs = 120_000)
        m.setOn(true, 0)
        assertFalse(m.idleExpired(119_000))
        m.poke(100_000)
        assertFalse(m.idleExpired(200_000))
        assertTrue(m.idleExpired(220_000))
        m.setOn(false, 220_000)
        assertFalse(m.idleExpired(500_000))
    }

    @Test fun coalescer_keeps_sub_pixel_remainders() {
        val c = MoveCoalescer(gain = 1f, minGapMs = 16)
        assertNull(c.feed(0.4f, 0f, 100))
        assertNull(c.feed(0.4f, 0f, 120))
        assertEquals(1 to 0, c.feed(0.4f, 0f, 140))
    }
}
