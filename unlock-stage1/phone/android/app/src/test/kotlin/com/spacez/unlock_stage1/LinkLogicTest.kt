package com.spacez.unlock_stage1

import com.spacez.unlock_stage1.handoff.HandoffLogic
import com.spacez.unlock_stage1.handoff.LinkLogic
import com.spacez.unlock_stage1.handoff.LinkLogic.Mode
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class LinkLogicTest {
    private fun s(url: String, title: String = "t") = HandoffLogic.Suggestion("web", title, url, null, false, 0)

    @Test fun appForMatchesHostsAndSubdomains() {
        assertEquals("youtube", LinkLogic.appFor("https://www.youtube.com/watch?v=abcdefghijk")?.key)
        assertEquals("youtube", LinkLogic.appFor("https://youtu.be/abcdefghijk")?.key)
        assertEquals("youtube", LinkLogic.appFor("https://music.youtube.com/watch?v=abcdefghijk")?.key)
        assertEquals("linkedin", LinkLogic.appFor("https://au.linkedin.com/in/someone")?.key)
        assertEquals("x", LinkLogic.appFor("https://twitter.com/a")?.key)
        assertNull(LinkLogic.appFor("https://example.com/"))
        assertNull(LinkLogic.appFor("https://notyoutube.com/"))
        assertNull(LinkLogic.appFor("https://youtube.com.evil.com/"))
    }

    @Test fun decideOrder() {
        val yt = "https://youtu.be/abcdefghijk"
        assertEquals(Mode.APP, LinkLogic.decide(yt, emptyMap()))
        assertEquals(Mode.BROWSER, LinkLogic.decide(yt, mapOf("youtube" to Mode.BROWSER)))
        assertEquals(Mode.APP, LinkLogic.decide(yt, mapOf("youtube" to Mode.BROWSER), Mode.APP))
        assertEquals(Mode.BROWSER, LinkLogic.decide(yt, emptyMap(), Mode.BROWSER))
        assertEquals(Mode.BROWSER, LinkLogic.decide("https://example.com/", mapOf("youtube" to Mode.APP)))
        assertEquals(Mode.APP, LinkLogic.decide("https://example.com/", emptyMap(), Mode.APP))
    }

    @Test fun modeParse() {
        assertEquals(Mode.APP, Mode.parse("app")); assertEquals(Mode.BROWSER, Mode.parse("BROWSER"))
        assertNull(Mode.parse("x")); assertNull(Mode.parse(null))
        assertEquals("browser", Mode.BROWSER.wire)
    }

    @Test fun moreRowsDropMainAndDuplicates() {
        val main = s("https://youtu.be/abcdefghijk")
        val rows = LinkLogic.moreRows(main, listOf(
            s("https://www.youtube.com/watch?v=abcdefghijk&t=5"), // same video as main
            s("https://a.com/x#frag"), s("https://a.com/x/"), // duplicates
            s("https://b.com/"), s("https://c.com/"), s("https://d.com/"), s("https://e.com/")))
        assertEquals(listOf("https://a.com/x#frag", "https://b.com/", "https://c.com/", "https://d.com/"), rows.map { it.url })
        assertEquals(2, LinkLogic.moreRows(null, listOf(s("https://a.com/"), s("https://b.com/")), 5).size)
        assertTrue(LinkLogic.moreRows(main, emptyList()).isEmpty())
    }

    @Test fun bubbleRules() {
        assertTrue(LinkLogic.bubbleWanted(true, true, true))
        assertFalse(LinkLogic.bubbleWanted(false, true, true))
        assertFalse(LinkLogic.bubbleWanted(true, false, true))
        assertFalse(LinkLogic.bubbleWanted(true, true, false))
        assertTrue(LinkLogic.bubbleAlive(1000, 1000 + LinkLogic.BUBBLE_TTL_MS - 1))
        assertFalse(LinkLogic.bubbleAlive(1000, 1000 + LinkLogic.BUBBLE_TTL_MS))
        assertFalse(LinkLogic.bubbleAlive(1000, 999))
    }

    @Test fun youtubeHandlerPicking() {
        fun h(a: String, b: String, c: Boolean) = LinkLogic.Handler(a, b, c)
        val sam = h("com.sec.android.app.sbrowser", "Samsung Internet", true)
        val yt = h("com.google.android.youtube", "YouTube", false)
        val mor = h("app.morphe.android.youtube", "Morphe", false)
        val nip = h("org.schabi.newpipe", "NewPipe", false)
        // the owner's pick wins while installed
        assertEquals("org.schabi.newpipe", LinkLogic.pickYouTube(listOf(sam, yt, mor, nip), null, "org.schabi.newpipe"))
        // a pick that was uninstalled falls back
        assertEquals("app.morphe.android.youtube", LinkLogic.pickYouTube(listOf(sam, mor), null, "gone.app"))
        // the system default wins when it is a real app
        assertEquals("org.schabi.newpipe", LinkLogic.pickYouTube(listOf(sam, mor, nip), "org.schabi.newpipe", null))
        // a browser as "default" does not count: first non-browser, Morphe before YouTube before others
        assertEquals("app.morphe.android.youtube", LinkLogic.pickYouTube(listOf(sam, nip, yt, mor), "com.sec.android.app.sbrowser", null))
        // only browsers: plain VIEW
        assertNull(LinkLogic.pickYouTube(listOf(sam), null, null))
    }

    @Test fun notificationRowsFitThenSeeAll() {
        assertEquals(3 to false, com.spacez.unlock_stage1.handoff.PcBubble.fit(3))
        assertEquals(6 to false, com.spacez.unlock_stage1.handoff.PcBubble.fit(6))
        assertEquals(5 to true, com.spacez.unlock_stage1.handoff.PcBubble.fit(25))
        assertEquals(20, LinkLogic.moreRows(null, (1..20).map { s("https://a.com/$it") }, Int.MAX_VALUE).size)
    }
}
