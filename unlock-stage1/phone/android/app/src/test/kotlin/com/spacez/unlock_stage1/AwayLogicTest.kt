package com.spacez.unlock_stage1

import com.spacez.unlock_stage1.away.AwayLogic
import com.spacez.unlock_stage1.handoff.HandoffLogic
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Item 17 (Aura Anywhere): the pure phone-side logic. Same rules as aura-link/service/src/tailnet.rs + away.rs. */
class AwayLogicTest {
    @Test fun tailnet_ranges() {
        assertTrue(AwayLogic.isTailnetV4("100.101.102.103"))
        assertTrue(AwayLogic.isTailnetV4("100.64.100.9"))
        assertTrue(AwayLogic.isTailnetV4("100.127.255.255"))
        assertFalse(AwayLogic.isTailnetV4("100.128.0.1"))
        assertFalse(AwayLogic.isTailnetV4("100.63.255.255"))
        assertFalse(AwayLogic.isTailnetV4("192.168.1.118"))
        assertFalse(AwayLogic.isTailnetV4("100.101.102"))
        assertFalse(AwayLogic.isTailnetV4("100.101.102.999"))
    }

    @Test fun tailnet_message_round_trip_and_validation() {
        assertEquals("{\"ip\":\"100.101.102.103\",\"port\":47140}", AwayLogic.hello("100.101.102.103"))
        assertEquals("100.64.100.9" to 47140, AwayLogic.parseTailnet("{\"ip\":\"100.64.100.9\",\"port\":47140}"))
        assertNull("a LAN address is never accepted", AwayLogic.parseTailnet("{\"ip\":\"192.168.1.118\",\"port\":47140}"))
        assertNull(AwayLogic.parseTailnet("{\"ip\":\"100.64.100.9\",\"port\":80}"))
        assertNull(AwayLogic.parseTailnet("junk"))
    }

    @Test fun gate_opens_only_for_a_while() {
        val g = AwayLogic.Gate()
        assertFalse(g.isOpen(1_000))
        g.open(1_000, 30_000)
        assertTrue(g.isOpen(30_999))
        assertFalse(g.isOpen(31_000))
        g.open(2_000, 1_000) // a shorter request never shortens an open window
        assertTrue(g.isOpen(30_000))
    }

    @Test fun status_card() {
        assertEquals("unpaired", AwayLogic.card(false, true, true, "100.1.1.1", "100.2.2.2").code)
        val m = AwayLogic.card(true, false, false, null, null)
        assertEquals("missing", m.code); assertEquals("install", m.action)
        val off = AwayLogic.card(true, true, false, null, "100.64.100.9")
        assertEquals("off", off.code); assertEquals("open", off.action); assertTrue(off.title.contains("tap to turn on"))
        assertEquals("no-pc", AwayLogic.card(true, true, true, "100.101.102.103", null).code)
        val r = AwayLogic.card(true, true, true, "100.101.102.103", "100.64.100.9")
        assertEquals("ready", r.code); assertTrue(r.text.contains("100.64.100.9")); assertEquals("none", r.action)
    }

    @Test fun last_activity_from_the_pc_opens_at_the_exact_second() {
        val j = "{\"app\":\"youtube\",\"title\":\"Slugterra - Episode 12\",\"url\":\"https://www.youtube.com/watch?v=dQw4w9WgXcQ\",\"pos_ms\":754000,\"playing\":false,\"at_ms\":1000,\"last\":true,\"pc_locked\":true,\"fresh\":false}"
        val l = AwayLogic.Last.parse(j)!!
        assertEquals("Slugterra - Episode 12", l.title())
        assertTrue(l.pcLocked); assertFalse(l.fresh)
        assertEquals("YouTube · 12:34", l.subtitle(99_000))
        assertTrue(l.s.openUrl(99_000), l.s.openUrl(99_000).contains("t=754"))
        // a video that kept playing: position moves on with the clock
        val p = AwayLogic.Last.parse(j.replace("\"playing\":false", "\"playing\":true"))!!
        assertEquals("YouTube · 12:44", p.subtitle(11_000))
        assertNull(AwayLogic.Last.parse("{}"))
        assertNull("only http(s)", AwayLogic.Last.parse("{\"url\":\"javascript:alert(1)\"}"))
        assertEquals("1:02:03", AwayLogic.clock(3_723_000))
    }

    @Test fun unlock_results() {
        val ok = AwayLogic.parseUnlockResult("{\"ok\":true,\"why\":\"unlocked\",\"at_ms\":1}")!!
        assertEquals("Your PC is unlocked", AwayLogic.resultText(ok))
        assertEquals("Your PC is already unlocked", AwayLogic.resultText(AwayLogic.parseUnlockResult("{\"ok\":true,\"why\":\"already unlocked\"}")!!))
        val no = AwayLogic.parseUnlockResult("{\"ok\":false,\"why\":\"The PC did not unlock: its sign-in screen was not asking.\"}")!!
        assertTrue(AwayLogic.resultText(no).contains("sign-in screen"))
        assertNull(AwayLogic.parseUnlockResult("{\"why\":\"x\"}"))
    }

    @Test
    fun moreFromYourPcListParsesAndFormats() {
        val j = """{"app":"youtube","title":"The Emperor | Slugterra","url":"https://youtube.com/watch?v=RLyI9gghapM","pos_ms":754000,"playing":false,"at_ms":1000,"last":true,"pc_locked":true,"fresh":true,
          "recent":[{"app":"web","title":"YouTube","url":"https://youtube.com/","pos_ms":null,"playing":false,"at_ms":1000},
                    {"app":"youtube","title":"Harry } \"Potter\"","url":"https://www.youtube.com/watch?v=JRKyEfBcXFQ","pos_ms":61000,"playing":false,"at_ms":1000},
                    {"app":"web","title":"bad","url":"javascript:1"}]}"""
        val l = AwayLogic.Last.parse(j)!!
        assertTrue(l.pcLocked)
        assertEquals(2, l.recent.size)
        assertEquals("Harry } \"Potter\"", l.recent[1].title)
        val row = AwayLogic.row(l.recent[1], 1000 + 5 * 60_000)
        assertEquals("YouTube · 1:01", row.sub)
        assertEquals("5 min ago", row.ago)
        assertEquals("youtube.com", AwayLogic.row(l.recent[0], 1000).sub)
        assertTrue(HandoffLogic.parseRecent("""{"url":"https://a.com"}""").isEmpty())
    }
}
