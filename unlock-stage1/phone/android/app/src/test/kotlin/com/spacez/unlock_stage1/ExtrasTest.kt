package com.spacez.unlock_stage1

import com.spacez.aura.link.Kind
import com.spacez.aura.link.MediaCmd
import com.spacez.aura.link.MediaState
import com.spacez.aura.link.Message
import com.spacez.aura.link.PresentCmd
import com.spacez.unlock_stage1.extras.PcRemote
import com.spacez.unlock_stage1.extras.PhoneRinger
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config

/** Item 15 on the phone: what each button sends, how the PC's state is shown, how the ring is planned. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class ExtrasTest {
    private val sent = ArrayList<Message>()

    private fun stub(err: String? = null) { sent.clear(); PcRemote.sender = { _, m, _ -> sent += m; err } }

    @Test fun ring_button_sends_ring_and_reports() {
        stub()
        assertEquals("Ringing your laptop", PcRemote.ringPc(ctx(), true))
        assertEquals(Kind.RING, sent.last().kind); assertEquals(1, sent.last().body[0].toInt())
        assertTrue(PcRemote.laptopRinging)
        assertEquals("Stopped", PcRemote.ringPc(ctx(), false))
        assertEquals(0, sent.last().body[0].toInt()); assertTrue(!PcRemote.laptopRinging)
    }

    @Test fun a_failed_ring_says_why_and_does_not_pretend() {
        stub("Your PC did not answer")
        assertEquals("Your PC did not answer", PcRemote.ringPc(ctx(), true))
        assertTrue(!PcRemote.laptopRinging)
    }

    @Test fun media_and_presenter_buttons_send_the_right_bodies() {
        stub()
        for (c in listOf(MediaCmd.PLAY_PAUSE, MediaCmd.NEXT, MediaCmd.PREV, MediaCmd.VOL_UP, MediaCmd.VOL_DOWN, MediaCmd.MUTE)) assertNull(PcRemote.mediaCmd(ctx(), c))
        assertEquals(listOf(1, 2, 3, 4, 5, 6), sent.map { it.body[0].toInt() })
        assertTrue(sent.all { it.kind == Kind.MEDIA_CMD })
        sent.clear()
        for (c in listOf(PresentCmd.NEXT, PresentCmd.PREV, PresentCmd.BLACK, PresentCmd.START, PresentCmd.END)) assertNull(PcRemote.present(ctx(), c))
        assertEquals(listOf(1, 2, 3, 5, 6), sent.map { it.body[0].toInt() })
        assertTrue(sent.all { it.kind == Kind.PRESENT })
        stub("The PC is locked, so the remote does nothing")
        assertEquals("The PC is locked, so the remote does nothing", PcRemote.present(ctx(), PresentCmd.NEXT))
    }

    @Test fun the_media_card_follows_the_pcs_state_and_artwork() {
        val st = MediaState(true, 10_000, 200_000, "Kannil Kannil", "Kunal Ganjawala", 40, false)
        PcRemote.onMediaState(st.encode())
        PcRemote.onMediaArt(byteArrayOf(-1, -40, -1, -39))
        val m = PcRemote.status()
        assertEquals("Kannil Kannil", m["title"]); assertEquals(true, m["playing"]); assertEquals(40, m["volume"])
        assertArrayEquals(byteArrayOf(-1, -40, -1, -39), m["art"] as ByteArray)
        assertTrue(PcRemote.positionNow() >= 10_000)
        // a new track clears the old artwork until the new one arrives
        PcRemote.onMediaState(MediaState(true, 0, 1000, "Other", "X").encode())
        assertNull(PcRemote.status()["art"])
        PcRemote.onMediaArt(ByteArray(0)); assertNull(PcRemote.status()["art"])
        // garbage is ignored
        PcRemote.onMediaState(byteArrayOf(1, 2, 3)); assertEquals("Other", PcRemote.status()["title"])
    }

    @Test fun the_ring_is_planned_at_max_and_the_old_volume_comes_back() {
        val p = PhoneRinger.plan(current = 2, max = 7)
        assertEquals(7, p.ringAt); assertEquals(2, p.restore)
        assertEquals(0, PhoneRinger.plan(-3, 7).restore)
        assertEquals(7, PhoneRinger.plan(99, 7).restore)
        assertEquals(5 * 60_000L, PhoneRinger.MAX_MS)
    }

    @Test fun the_slide_cue_is_a_strong_distinct_pattern() {
        val slide = FeedbackPlan.primitives(FeedbackPlan.Cue.SLIDE)
        assertTrue(slide.size >= 2 && slide.all { it.s == 1f })
        val w = FeedbackPlan.waveform(FeedbackPlan.Cue.SLIDE, 1f)
        assertEquals(255, w.amps.max())
        assertNotNull(FeedbackPlan.Cue.from("slide"))
        assertTrue(FeedbackPlan.waveform(FeedbackPlan.Cue.SLIDE, 1f).timings.toList() != FeedbackPlan.waveform(FeedbackPlan.Cue.TAP, 1f).timings.toList())
    }

    private fun ctx(): android.content.Context = RuntimeEnvironment.getApplication()
}
