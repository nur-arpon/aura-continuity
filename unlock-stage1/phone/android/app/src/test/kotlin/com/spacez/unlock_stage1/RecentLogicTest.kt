package com.spacez.unlock_stage1

import com.spacez.unlock_stage1.handoff.HandoffLogic
import com.spacez.unlock_stage1.handoff.RecentLogic
import com.spacez.unlock_stage1.handoff.RecentLogic.Entry
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class RecentLogicTest {
    private fun yt(t: String, ch: String? = "Chan", pos: Long? = 1000, at: Long = 1) = Entry("youtube", t, null, ch, pos, 600_000, true, at)
    private fun web(u: String, t: String = "Page") = Entry("web", t, u, null, null, null, false, 1)

    @Test fun ringDedupesAndCaps() {
        val r = RecentLogic.Ring()
        assertTrue(r.note(yt("A")))
        assertTrue(r.note(web("https://a.example/")))
        assertTrue(r.note(yt("B")))
        r.note(yt("B", pos = 1))
        assertEquals(listOf("B", "Page", "A"), r.list().map { it.title })
        // the same link again moves to the front (and takes the new title)
        assertTrue(r.note(web("https://a.example/#x", "Page 2")))
        assertEquals("Page 2", r.list().first().title)
        assertEquals(3, r.list().size)
        for (i in 0 until 20) r.note(web("https://e.example/$i", "P$i"))
        assertEquals(RecentLogic.MAX, r.list().size)
    }

    @Test fun positionOnlyChangeIsNotAChange() {
        val r = RecentLogic.Ring()
        r.note(yt("A"))
        assertFalse(r.note(yt("A", pos = 99_000)))
        assertTrue(r.note(yt("A2")))
    }

    @Test fun unusableEntriesAreRefused() {
        val r = RecentLogic.Ring()
        assertFalse(r.note(Entry("facebook", "no link", null, null, null, null, false, 1)))
        assertFalse(r.note(yt("")))
        assertTrue(r.list().isEmpty())
        assertNull(RecentLogic.fromState(mapOf("app" to "whatsapp", "title" to "chat"), 5))
        assertEquals("YT", RecentLogic.fromState(mapOf("app" to "youtube", "title" to "YT", "playing" to true), 5)?.title)
    }

    @Test fun wireShapeMatchesWhatThePcParses() {
        val l = listOf(yt("Lo-fi \"beats\"", "Chill"), web("https://blog.rust-lang.org/", "Rust"))
        val json = RecentLogic.toJson(l, 1234)
        val top = HandoffLogic.parseFlat(json)
        assertEquals(true, top["state"]); assertEquals(true, top["recent_list"]); assertEquals(1234L, top["sent_ms"])
        // the array walker the phone uses for the PC's list reads this one too (it keeps entries that carry a link)
        assertEquals(listOf("https://blog.rust-lang.org/"), HandoffLogic.parseRecent(json).map { it.url })
        assertTrue(json.contains("\"duration_ms\":600000"))
        assertTrue(json.contains("Lo-fi \\\"beats\\\""))
        assertFalse(json.contains("\"url\":null"))
    }

    @Test fun persistenceRoundTrips() {
        val l = listOf(yt("A", "C"), web("https://x.example/"))
        val back = RecentLogic.deserialize(RecentLogic.serialize(l))
        assertEquals(l.map { it.title to it.url }, back.map { it.title to it.url })
        assertEquals(600_000L, back[0].durationMs)
        assertTrue(RecentLogic.deserialize(null).isEmpty())
    }
}
