package com.spacez.unlock_stage1.watchlink

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class PhonePointerTest {
    @Test fun starts_in_the_middle_and_stays_on_screen() {
        val p = PhonePointer(1000, 2000)
        assertEquals(500f, p.x); assertEquals(1000f, p.y)
        p.move(-9999, 9999)
        assertEquals(0f, p.x); assertEquals(1999f, p.y)
    }
    @Test fun moves_scale_by_gain() {
        val p = PhonePointer(1000, 2000); p.move(10, -10, gain = 2f)
        assertEquals(520f, p.x); assertEquals(980f, p.y)
    }
    @Test fun scroll_down_wheel_swipes_the_finger_up_and_zero_does_nothing() {
        val p = PhonePointer(1000, 2000)
        val s = p.scrollSwipe(-120)!!
        assertTrue(s[3] < s[1]); assertEquals(s[0], s[2]); assertEquals(300f, s[1] - s[3], 0.01f)
        val up = p.scrollSwipe(120)!!; assertTrue(up[3] > up[1])
        assertNull(p.scrollSwipe(0))
    }
    @Test fun a_huge_wheel_value_is_capped_inside_the_screen() {
        val p = PhonePointer(1000, 2000); val s = p.scrollSwipe(-100000)!!
        assertTrue(s[1] in 1f..1998f && s[3] in 1f..1998f)
    }
}
