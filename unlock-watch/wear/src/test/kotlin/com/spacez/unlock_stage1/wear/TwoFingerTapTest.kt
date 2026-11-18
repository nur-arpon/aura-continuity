package com.spacez.unlock_stage1.wear

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class TwoFingerTapTest {
    private fun two(x1: Float = 10f, x2: Float = 80f) = mapOf(1L to (x1 to 50f), 2L to (x2 to 50f))
    private val none = emptyMap<Long, Pair<Float, Float>>()

    @Test fun quick_two_finger_touch_is_a_tap() {
        val t = TwoFingerTap()
        assertFalse(t.update(mapOf(1L to (10f to 50f)), 0))
        assertFalse(t.update(two(), 20))
        assertFalse(t.update(mapOf(2L to (80f to 50f)), 120))   // one finger lifts
        assertTrue(t.update(none, 140))
    }
    @Test fun fires_once_only() {
        val t = TwoFingerTap(); t.update(two(), 0)
        assertTrue(t.update(none, 100)); assertFalse(t.update(none, 110))
    }
    @Test fun a_slow_two_finger_press_is_not_a_tap() {
        val t = TwoFingerTap(); t.update(two(), 0)
        assertFalse(t.update(none, 400))
    }
    @Test fun two_fingers_that_slide_are_a_scroll_not_a_tap() {
        val t = TwoFingerTap(); t.update(two(), 0)
        t.update(two(x1 = 60f, x2 = 130f), 60)
        assertFalse(t.update(none, 120))
    }
    @Test fun one_finger_never_counts_and_state_resets_after_a_tap() {
        val t = TwoFingerTap()
        assertFalse(t.update(mapOf(1L to (10f to 50f)), 0)); assertFalse(t.update(none, 80))
        t.update(two(), 1000); assertTrue(t.update(none, 1090))
        t.update(two(), 2000); assertTrue(t.update(none, 2100))
    }
    @Test fun active_while_two_fingers_are_down() {
        val t = TwoFingerTap(); assertFalse(t.active)
        t.update(two(), 0); assertTrue(t.active); t.update(none, 50); assertFalse(t.active)
    }
}
