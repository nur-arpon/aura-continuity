package com.spacez.unlock_stage1

import com.spacez.unlock_stage1.away.AwayLogic
import com.spacez.unlock_stage1.handoff.HandoffLogic
import com.spacez.unlock_stage1.zerotap.Contacts
import com.spacez.unlock_stage1.zerotap.ContinueRules
import com.spacez.unlock_stage1.zerotap.ContinueRules.Decision
import com.spacez.unlock_stage1.zerotap.ContinueRules.Item
import com.spacez.unlock_stage1.zerotap.ScreenReader
import com.spacez.unlock_stage1.zerotap.YouTubeLookup
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Item 19, zero-tap continue: one test per DECISION RULE (aura-anywhere/CONTINUE-RULES.md), each a
 * recorded scenario. PC -> phone half; the phone -> PC half is handoff/src-tauri/src/core/zerotap.rs.
 */
class ContinueRulesTest {
    private val now = 1_800_000_000_000L
    private val ep = "https://www.youtube.com/watch?v=RLyI9gghapM"
    private fun yt(playing: Boolean, pos: Long, agoMs: Long, url: String = ep, title: String = "The Emperor | Slugterra") =
        Item("youtube", title, url, pos, playing, startedMs = now - agoMs, touchedMs = now - agoMs)
    private fun go(d: Decision) = (d as Decision.Go)

    @Test fun rule_several_playing_audible_and_newest_wins() {
        val a = yt(true, 1000, 50_000, "https://www.youtube.com/watch?v=AAAAAAAAAAA", "a").copy(audible = false, startedMs = now - 1_000)
        val b = yt(true, 1000, 50_000, "https://www.youtube.com/watch?v=BBBBBBBBBBB", "b").copy(startedMs = now - 40_000)
        val c = yt(true, 1000, 50_000, "https://www.youtube.com/watch?v=CCCCCCCCCCC", "c").copy(startedMs = now - 9_000)
        assertEquals("c", ContinueRules.pickSource(listOf(a, b, c))!!.title)
    }

    @Test fun rule_nothing_playing_last_interacted_wins() {
        val a = yt(false, 1000, 90_000, "https://www.youtube.com/watch?v=AAAAAAAAAAA", "a")
        val b = yt(false, 1000, 10_000, "https://www.youtube.com/watch?v=BBBBBBBBBBB", "b")
        assertEquals("b", ContinueRules.pickSource(listOf(a, b))!!.title)
    }

    @Test fun rule_home_page_never_wins() {
        val home = Item("web", "YouTube", "https://www.youtube.com/", playing = true, touchedMs = now)
        val page = Item("web", "Article", "https://example.com/a", touchedMs = now - 5_000)
        assertEquals("Article", ContinueRules.pickSource(listOf(home, page))!!.title)
        assertTrue(ContinueRules.decide(home, null, 0, null, now) is Decision.Skip)
    }

    @Test fun rule_two_minute_window() {
        assertTrue(ContinueRules.decide(yt(false, 60_000, 30_000), null, 0, null, now) is Decision.Go)
        assertEquals(Decision.Skip("not used in the last 2 minutes"), ContinueRules.decide(yt(false, 60_000, 121_000), null, 0, null, now))
        assertTrue(ContinueRules.decide(yt(true, 60_000, 600_000), null, 0, null, now) is Decision.Go)
    }

    @Test fun rule_exact_second_extrapolated_while_playing() {
        // recorded 2026-10-21: PC read 2:00 at the request, the phone opened 10 s later
        assertEquals("$ep&t=130s", go(ContinueRules.decide(yt(true, 120_000, 10_000), null, 0, null, now)).url)
        assertEquals("$ep&t=120s", go(ContinueRules.decide(yt(false, 120_000, 10_000), null, 0, null, now)).url)
    }

    @Test fun rule_clock_skew_removed_by_sent_ms() {
        // the PC clock is 4 s behind the phone: at_ms/sent_ms are PC clock, rx is phone clock
        val json = """{"app":"youtube","title":"x","url":"$ep","pos_ms":120000,"playing":true,"at_ms":${now - 4000 - 1000},"sent_ms":${now - 4000},"idle_ms":500}"""
        val l = AwayLogic.Last.parse(json, now)!!
        assertEquals(now - 1000, l.s.atMs)
        assertEquals(500L, l.idleMs)
        assertEquals("$ep&t=121s", l.s.openUrl(now))
    }

    @Test fun rule_owner_started_another_video_within_3s() {
        val mine = yt(true, 5_000, 1_000, "https://www.youtube.com/watch?v=OWNERPICKED", "his own pick")
        assertEquals(Decision.Skip("the owner started something else"), ContinueRules.decide(yt(true, 60_000, 5_000), mine, now - 2_000, null, now))
        // started long before he opened the app: not his new pick
        assertTrue(ContinueRules.decide(yt(true, 60_000, 5_000), null, now - 10_000, null, now) is Decision.Go)
    }

    @Test fun rule_both_playing_different_things_hands_off() {
        val phone = yt(true, 5_000, 60_000, "https://www.youtube.com/watch?v=OTHERVIDEO1", "music")
        assertEquals(Decision.Skip("both devices are playing different things"), ContinueRules.decide(yt(true, 60_000, 5_000), phone, 0, null, now))
    }

    @Test fun rule_ads_wait_short_clips_skip() {
        assertTrue(ContinueRules.decide(yt(true, 1_000, 1_000).copy(ad = true), null, 0, null, now) is Decision.Wait)
        assertEquals(Decision.Skip("clip shorter than 30 s"), ContinueRules.decide(yt(true, 1_000, 1_000).copy(durationMs = 19_000), null, 0, null, now))
    }

    @Test fun rule_shorts_live_playlists() {
        assertEquals("https://www.youtube.com/shorts/abcdefghijk", ContinueRules.resumeUrl(yt(true, 9_000, 0, "https://www.youtube.com/shorts/abcdefghijk"), now))
        assertEquals(ep, ContinueRules.resumeUrl(yt(true, 9_000, 0).copy(live = true), now))
        assertEquals("$ep&list=PL1&index=4&t=90s", ContinueRules.resumeUrl(yt(false, 90_000, 0, "$ep&pp=x&list=PL1&index=4"), now))
    }

    @Test fun rule_same_video_already_open_just_seek() {
        val open = yt(false, 3_000, 0)
        assertTrue(go(ContinueRules.decide(yt(true, 60_000, 1_000), null, 0, open, now)).seekOnly)
    }

    @Test fun rule_private_never_shared() {
        assertNull(ContinueRules.pickSource(listOf(yt(true, 1, 1).copy(private = true))))
        assertEquals(Decision.Skip("private window"), ContinueRules.decide(yt(true, 1, 1).copy(private = true), null, 0, null, now))
    }

    @Test fun rule_several_youtube_apps_last_used_default_morphe() {
        val all = listOf("app.morphe.android.youtube", "com.google.android.youtube", "app.morphe.android.apps.youtube.music")
        assertEquals("app.morphe.android.youtube", ContinueRules.phoneApp(ep, all, emptyMap()))
        assertEquals("com.google.android.youtube", ContinueRules.phoneApp(ep, all, mapOf("com.google.android.youtube" to 5L)))
        assertEquals("app.morphe.android.apps.youtube.music", ContinueRules.phoneApp("https://music.youtube.com/watch?v=RLyI9gghapM", all, emptyMap()))
        assertNull(ContinueRules.phoneApp("https://example.com/", all, emptyMap()))
    }

    @Test fun rule_app_mapping_both_ways() {
        assertEquals("facebook", ContinueRules.urlKind("https://www.facebook.com/reel/123"))
        assertEquals("claude", ContinueRules.urlKind("https://claude.ai/chat/0b5e"))
        assertEquals("https://m.me/100012345", ContinueRules.phoneLink("https://www.messenger.com/t/100012345/"))
        assertTrue(ContinueRules.fits("com.sec.android.app.sbrowser", Item("web", "A", "https://example.com/")))
        assertFalse(ContinueRules.fits("com.sec.android.app.sbrowser", Item("youtube", "A", ep)))
        assertTrue(ContinueRules.fits("app.morphe.android.youtube", Item("youtube", "A", ep)))
        assertTrue(ContinueRules.fits("com.anthropic.claude", Item("claude", "c", "https://claude.ai/chat/1")))
        assertFalse(ContinueRules.fits("com.whatsapp", Item("claude", "c", "https://claude.ai/chat/1")))
    }

    @Test fun rule_after_continue_setting() {
        assertEquals(setOf("pause", "mute", "leave"), ContinueRules.AFTER)
    }

    @Test fun pc_item_last_used_from_idle_time() {
        val s = HandoffLogic.Suggestion("web", "A", "https://example.com/", null, false, now)
        assertEquals(now - 3_600_000, ContinueRules.fromPc(s, 3_600_000, now).touchedMs)
        assertEquals(now, ContinueRules.fromPc(s.copy(playing = true), 3_600_000, now).touchedMs)
    }

    @Test fun screen_reader_apps() {
        val sb = listOf(ScreenReader.Node("com.sec.android.app.sbrowser:id/location_bar_edit_text", "example.com/news/1", null))
        assertEquals("https://example.com/news/1", ScreenReader.read("com.sec.android.app.sbrowser", sb)!!.url)
        assertNull(ScreenReader.read("com.sec.android.app.sbrowser", listOf(ScreenReader.Node("com.sec.android.app.sbrowser:id/location_bar_edit_text", "how to cook", null))))
        val wa = listOf(ScreenReader.Node("com.whatsapp:id/toolbar", null, "Navigate up"), ScreenReader.Node("com.whatsapp:id/conversation_contact_name", "Mum", null))
        assertEquals("Mum", ScreenReader.read("com.whatsapp", wa)!!.title)
        assertNull(ScreenReader.read("com.whatsapp", listOf(ScreenReader.Node("com.whatsapp:id/toolbar", "WhatsApp", null))))
        val cl = listOf(ScreenReader.Node("", null, "Back"), ScreenReader.Node("", "Claude", null), ScreenReader.Node("", "Mobile sync features across devices", null))
        assertEquals("Mobile sync features across devices", ScreenReader.read("com.anthropic.claude", cl)!!.title)
        assertNull(ScreenReader.read("com.spotify.music", cl))
    }

    @Test fun whatsapp_numbers() {
        assertEquals("61400111222", Contacts.e164Digits("+61 400 111 222", "AU"))
        assertEquals("61400111222", Contacts.e164Digits("0400 111 222", "AU"))
        assertEquals("8801711000000", Contacts.e164Digits("00880 1711 000000", "AU"))
        assertNull(Contacts.e164Digits("123", "AU"))
    }

    @Test fun youtube_lookup_confidence() {
        val page = """..."videoId":"RLyI9gghapM"...,"videoId":"RLyI9gghapM"..."videoId":"aqz-KE-bpKQ"..."""
        assertEquals(listOf("RLyI9gghapM", "aqz-KE-bpKQ"), YouTubeLookup.candidates(page))
        // m.youtube.com escapes its JSON (the phone was redirected there: no candidates before the fix)
        assertEquals(listOf("WiYDdS0Rhsw"), YouTubeLookup.candidates("""\x7b\x22videoId\x22:\x22WiYDdS0Rhsw\x22"""))
        val o = """{"title":"The Emperor | Slugterra | Season 3 Compilation","author_name":"Slugterra - WildBrain"}"""
        assertTrue(YouTubeLookup.confident(o, "The Emperor | Slugterra | Season 3 Compilation", "Slugterra - WildBrain"))
        assertFalse(YouTubeLookup.confident(o, "The Emperor | Slugterra | Season 3 Compilation", "Someone else"))
        assertFalse(YouTubeLookup.confident(o, "Another title", null))
    }

    @Test fun rule_link_only_for_the_same_title_and_channel() {
        val c = com.spacez.unlock_stage1.zerotap.LinkCache(3)
        c.put("Old video", "Chan A", "https://www.youtube.com/watch?v=OLDOLDOLDOL")
        assertNull("a different title never reuses a link", c.get("New video", "Chan A"))
        assertNull("same title, other channel", c.get("Old video", "Chan B"))
        assertEquals("https://www.youtube.com/watch?v=OLDOLDOLDOL", c.get(" old VIDEO ", "chan a"))
        c.put("Share", null, "https://www.youtube.com/watch?v=SHARESHARES")
        assertEquals("https://www.youtube.com/watch?v=SHARESHARES", c.get("Share", "Whoever"))
        assertNull(c.get("Other", null))
    }

    @Test fun lookupIdentityMatchesDuration() {
        val page = """{"videoRenderer":{"videoId":"AAAAAAAAAAA","lengthText":{"accessibility":{},"simpleText":"12:34"}}},{"videoRenderer":{"videoId":"BBBBBBBBBBB","lengthText":{"simpleText":"1:02:03"}}},{"videoRenderer":{"videoId":"CCCCCCCCCCC"}}"""
        assertEquals(listOf("AAAAAAAAAAA" to 754L, "BBBBBBBBBBB" to 3723L, "CCCCCCCCCCC" to null), YouTubeLookup.candidatesWithLen(page))
        assertTrue(YouTubeLookup.durationMatches(754_000, 754))
        assertTrue(YouTubeLookup.durationMatches(754_000, 751))
        assertFalse(YouTubeLookup.durationMatches(754_000, 760))
        assertFalse(YouTubeLookup.durationMatches(754_000, 1800))
        assertFalse(YouTubeLookup.durationMatches(754_000, null))
        assertTrue(YouTubeLookup.durationMatches(null, 754))
        assertEquals(null, YouTubeLookup.parseClock("x"))
    }

    @Test fun rule_lookup_budget_is_two_seconds() {
        assertEquals(2_000L, YouTubeLookup.BUDGET_MS)
    }
}
