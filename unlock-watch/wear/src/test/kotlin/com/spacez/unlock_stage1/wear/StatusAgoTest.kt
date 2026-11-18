package com.spacez.unlock_stage1.wear

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class StatusAgoTest {
    private val h = 3_600_000L
    @Test fun relative() {
        val now = 100 * h
        assertEquals("Just now", StatusText.ago(now - 20_000, now))
        assertEquals("5 min ago", StatusText.ago(now - 5 * 60_000, now))
        assertEquals("2 h ago", StatusText.ago(now - 2 * h, now))
        assertEquals("Yesterday", StatusText.ago(now - 30 * h, now))
        assertNull(StatusText.ago(now - 49 * h, now))
        assertNull(StatusText.ago(0, now))
    }
    @Test fun detailHidesWhenStale() {
        val now = 100 * h
        val s = PcSummary(PcLook.UNLOCKED, "Unlocked", "By this watch", now - 2 * h)
        assertEquals("By this watch · 2 h ago", s.detailAt(now))
        assertEquals("", PcSummary(PcLook.UNLOCKED, "Unlocked", "By this watch", now - 60 * h).detailAt(now))
    }
}
