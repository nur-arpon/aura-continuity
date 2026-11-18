package com.spacez.unlock_stage1

import com.spacez.unlock_stage1.handoff.ChatHeadLogic
import com.spacez.unlock_stage1.handoff.ChatHeadLogic.AppAction
import com.spacez.unlock_stage1.handoff.ChatHeadLogic.LApp
import com.spacez.unlock_stage1.handoff.ChatHeadLogic.Tab
import org.junit.Assert.*
import org.junit.Test

class ChatHeadLogicTest {
    private fun app(browser: Boolean, vararg tabs: Tab, url: String? = null) = LApp("x", "X", browser, "t", url, 61_000, tabs.toList())

    @Test fun browserWithOpenableTabsFansOutAgain_otherAppsGoToTheLaptopScreen() {
        assertEquals(AppAction.TABS, ChatHeadLogic.appAction(app(true, Tab("a", "https://a.com", true))))
        assertEquals(AppAction.OPEN_URL, ChatHeadLogic.appAction(app(true, url = "https://a.com")))
        assertEquals(AppAction.OPEN_LAPTOP, ChatHeadLogic.appAction(app(false)))
        assertEquals(AppAction.OPEN_LAPTOP, ChatHeadLogic.appAction(app(true, Tab("no link", null, true))))
    }

    @Test fun tabsWithoutALinkAreNotOfferedAndEightIsTheMost() {
        val tabs = (1..12).map { Tab("t$it", if (it == 2) null else "https://s$it.com", it == 1) }.toTypedArray()
        val v = ChatHeadLogic.visibleTabs(app(true, *tabs))
        assertEquals(8, v.size)
        assertTrue(v.none { it.url == null })
        assertEquals("t1", v[0].title)
    }

    @Test fun onlyTheActiveYouTubeTabKeepsItsSecond() {
        val a = app(true)
        assertEquals(61_000L, ChatHeadLogic.posFor(a, Tab("v", "https://www.youtube.com/watch?v=dQw4w9WgXcQ", true)))
        assertNull(ChatHeadLogic.posFor(a, Tab("v", "https://www.youtube.com/watch?v=dQw4w9WgXcQ", false)))
        assertNull(ChatHeadLogic.posFor(a, Tab("p", "https://example.com/", true)))
    }

    @Test fun arcPointsTowardsTheMiddleOfTheScreen() {
        val left = ChatHeadLogic.arc(3, 100f, towardsLeft = true)
        assertTrue(left.all { it.first < 0 })
        assertEquals(0f, left[1].second, 0.01f) // the middle bubble is level with the origin
        val right = ChatHeadLogic.arc(3, 100f, towardsLeft = false)
        assertTrue(right.all { it.first > 0 })
        assertEquals(1, ChatHeadLogic.arc(1, 100f, true).size)
    }

    @Test fun ringsHoldEveryAppWithoutOverlap() {
        val gap = 78f
        for (n in listOf(1, 4, 5, 6, 9, 13)) {
            val p = ChatHeadLogic.rings(n, 120f, 96f, gap, towardsLeft = true)
            assertEquals(n, p.size)
            assertTrue(p.all { it.first < 1f }) // the fan points left
            for (i in p.indices) for (j in i + 1 until p.size) {
                val d = Math.hypot((p[i].first - p[j].first).toDouble(), (p[i].second - p[j].second).toDouble())
                assertTrue("n=$n bubbles $i and $j are only $d apart", d >= gap * 0.9)
            }
        }
        // 5 on the first ring (radius 120), the rest further out
        val r = ChatHeadLogic.rings(13, 120f, 96f, gap, true).map { Math.hypot(it.first.toDouble(), it.second.toDouble()) }
        assertEquals(5, r.count { it < 130 })
        assertEquals(8, r.count { it > 200 })
        // near the top edge the fan tilts down
        val top = ChatHeadLogic.rings(3, 120f, 96f, gap, true, ChatHeadLogic.tilt(100f, 2000f))
        assertTrue(top[1].second > 0f)
    }

    @Test fun theHeadSurvivesALinkDrop() {
        val now = 10_000_000L
        assertTrue(ChatHeadLogic.linkOk(true, 0, now))
        assertTrue(ChatHeadLogic.linkOk(false, now - 60_000, now))
        assertFalse(ChatHeadLogic.linkOk(false, now - ChatHeadLogic.LINK_GRACE_MS - 1, now))
        assertFalse(ChatHeadLogic.linkOk(false, 0, now))
    }

    @Test fun bubblesStayOnScreen() {
        val (x, y) = ChatHeadLogic.clampCentre(-50f, 5000f, 1080, 2340, 78f, 78f)
        assertTrue(x >= 78f && y <= 2340 - 78f - 78f)
    }

    @Test fun quietRulesHideTheHead() {
        fun show(fs: Boolean = false, call: Boolean = false, locked: Boolean = false, until: Long = 0, link: Boolean = true) =
            ChatHeadLogic.shouldShow(true, true, link, true, 1_000, until, fs, call, locked)
        assertTrue(show())
        assertFalse(show(fs = true)); assertFalse(show(call = true)); assertFalse(show(locked = true)); assertFalse(show(link = false))
        assertFalse(show(until = 5_000)); assertTrue(show(until = 900))
        assertFalse(ChatHeadLogic.shouldShow(true, false, true, true, 1, 0, false, false, false))
    }

    @Test fun titlesLettersFavicons() {
        assertEquals("Melbourne to…", ChatHeadLogic.shortTitle("Melbourne to Dhaka | Google Flights", 13))
        assertEquals("Y", ChatHeadLogic.letter(" youtube"))
        assertEquals("youtube.com", ChatHeadLogic.hostOf("https://YouTube.com/watch?v=1"))
    }

    @Test fun springOvershootsThenSettles() {
        assertEquals(0f, ChatHeadLogic.spring(0f), 0.01f)
        assertEquals(1f, ChatHeadLogic.spring(1f), 0.001f)
        assertTrue((1..99).map { ChatHeadLogic.spring(it / 100f) }.any { it > 1.02f })
    }
}
