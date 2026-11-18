package com.spacez.unlock_stage1

import com.spacez.unlock_stage1.handoff.HandoffLogic
import com.spacez.unlock_stage1.handoff.HandoffLogic.Suggestion
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class HandoffLogicTest {
    @Test fun youtube_links_are_recognised() {
        assertEquals("dQw4w9WgXcQ", HandoffLogic.videoId("https://www.youtube.com/watch?v=dQw4w9WgXcQ&list=x"))
        assertEquals("dQw4w9WgXcQ", HandoffLogic.videoId("https://youtu.be/dQw4w9WgXcQ?si=abc"))
        assertEquals("dQw4w9WgXcQ", HandoffLogic.videoId("https://m.youtube.com/watch?v=dQw4w9WgXcQ"))
        assertEquals("dQw4w9WgXcQ", HandoffLogic.videoId("https://www.youtube.com/shorts/dQw4w9WgXcQ"))
        assertNull(HandoffLogic.videoId("https://example.com/watch?v=dQw4w9WgXcQ"))
        assertNull(HandoffLogic.videoId("https://www.youtube.com/feed/subscriptions"))
        assertNull(HandoffLogic.videoId("javascript:alert(1)"))
    }

    @Test fun time_is_put_on_youtube_links_only() {
        assertEquals("https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=61s", HandoffLogic.withTime("https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=5s", 61_900))
        assertEquals("https://youtu.be/dQw4w9WgXcQ?si=a&t=3600s", HandoffLogic.withTime("https://youtu.be/dQw4w9WgXcQ?si=a", 3_600_000))
        assertEquals("https://youtu.be/dQw4w9WgXcQ", HandoffLogic.withTime("https://youtu.be/dQw4w9WgXcQ?t=90", 2_000))
        assertEquals("https://example.com/a", HandoffLogic.withTime("https://example.com/a", 61_000))
        assertEquals("https://youtu.be/dQw4w9WgXcQ", HandoffLogic.withTime("https://youtu.be/dQw4w9WgXcQ", null))
        assertEquals(90L, HandoffLogic.tParam("https://youtu.be/x?t=1m30s"))
        assertEquals(90L, HandoffLogic.tParam("https://youtu.be/x?t=90"))
        assertNull(HandoffLogic.tParam("https://youtu.be/x"))
    }

    @Test fun playing_position_moves_on_but_not_backwards() {
        assertEquals(65_500L, HandoffLogic.positionNow(61_000, true, 1_000_000, 1_004_500))
        assertEquals(61_000L, HandoffLogic.positionNow(61_000, false, 1_000_000, 9_000_000))
        assertEquals(61_000L, HandoffLogic.positionNow(61_000, true, 1_000_000, 10))
        assertNull(HandoffLogic.positionNow(null, true, 1, 2))
    }

    @Test fun shared_text_yields_its_link() {
        assertEquals("https://youtu.be/dQw4w9WgXcQ?si=x", HandoffLogic.firstUrl("Never Gonna Give You Up https://youtu.be/dQw4w9WgXcQ?si=x"))
        assertNull(HandoffLogic.firstUrl("no link here"))
    }

    @Test fun suggestion_json_matches_the_pc_format() {
        // exactly what handoff/src-tauri/src/core/handoff.rs Suggestion::to_json writes (serde_json, keys sorted)
        val pc = """{"app":"youtube","at_ms":1000000,"playing":true,"pos_ms":61000,"title":"A \"talk\"\nline","url":"https://www.youtube.com/watch?v=dQw4w9WgXcQ"}"""
        val s = Suggestion.parse(pc)!!
        assertEquals("youtube", s.app); assertEquals("A \"talk\"\nline", s.title); assertEquals(61_000L, s.posMs); assertTrue(s.playing)
        assertEquals("https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=65s", s.openUrl(1_004_500))
        assertEquals(s, Suggestion.parse(s.toJson()))
        assertNull(Suggestion.parse("""{"url":"file:///sdcard/x"}"""))
        assertNull(Suggestion.parse("{}"))
        val nested = HandoffLogic.parseFlat("""{"a":{"b":[1,"}"]},"url":"https://x","n":null,"t":"é"}""")
        assertEquals("https://x", nested["url"]); assertEquals("é", nested["t"]); assertTrue(nested.containsKey("n"))
    }

    @Test fun debouncer_only_on_change_after_settle_and_gap() {
        val d = HandoffLogic.Debouncer(5_000, 60_000)
        d.observe("a", 0); assertEquals(5_000L, d.nextWake()); assertNull(d.due(4_999)); assertEquals("a", d.due(5_000))
        d.observe("a", 6_000); assertNull(d.due(70_000)); assertNull(d.nextWake())
        d.observe("b", 7_000); d.observe("c", 8_000); assertEquals(65_000L, d.nextWake()); assertEquals("c", d.due(65_000))
        d.observe(null, 70_000); assertNull(d.nextWake())
    }

    @Test fun clipboard_never_bounces_back() {
        val e = HandoffLogic.ClipEcho()
        e.noteApplied("from pc".toByteArray())
        assertFalse(e.shouldSend("from pc".toByteArray()))
        assertTrue(e.shouldSend("typed here".toByteArray()))
        assertFalse(e.shouldSend("typed here".toByteArray()))
    }
}
