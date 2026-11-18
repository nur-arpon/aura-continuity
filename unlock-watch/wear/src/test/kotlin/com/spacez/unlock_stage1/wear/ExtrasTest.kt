package com.spacez.unlock_stage1.wear

import android.content.Context
import androidx.test.core.app.ApplicationProvider
import com.spacez.aura.link.Kind
import com.spacez.aura.link.MediaCmd
import com.spacez.aura.link.PresentCmd
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/** Item 15 on the watch: the bezel stepper, the plain-words failures, the extras gate. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class ExtrasTest {
    private val ctx: Context get() = ApplicationProvider.getApplicationContext()

    @Test fun bezel_turns_into_whole_steps_both_ways() {
        val s = RotaryStepper(48f)
        assertEquals(0, s.feed(20f))
        assertEquals(1, s.feed(30f))        // 50 total -> one step, 2 left over
        assertEquals(0, s.feed(40f))        // 42
        assertEquals(1, s.feed(10f))        // 52 -> step
        assertEquals(-2, s.feed(-100f))     // two steps back
        assertEquals(3, s.feed(150f))
        s.reset(); assertEquals(0, s.feed(47.9f))
    }

    @Test fun a_fast_spin_fires_one_command_per_cooldown() {
        val d = RotaryDetent(48f, 250)
        var fired = 0
        for (i in 0 until 100) fired += kotlin.math.abs(d.feed(60f, 1000L + i * 7L))   // 700 ms of hard spinning
        assertEquals(3, fired)                                                      // t=1000, 1252, 1504 -> never a flood
        assertEquals(-1, RotaryDetent().feed(-100f, 5000L))
    }

    @Test fun mouse_moves_coalesce_to_60hz() {
        val m = MoveCoalescer(gain = 1f, minGapMs = 16)
        assertEquals(2 to 1, m.feed(2f, 1f, 1000L))
        assertEquals(null, m.feed(1f, 1f, 1005L))            // too soon, kept
        assertEquals(3 to 2, m.feed(2f, 1f, 1020L))          // 1+2, 1+1
        assertEquals(null, m.feed(0.4f, 0f, 1040L))          // sub-pixel stays
    }

    @Test fun failure_words_fit_a_round_screen() {
        assertEquals("Pair first", WatchRemoteText.short("Pair with PC first"))
        assertEquals("Off on the PC", WatchRemoteText.short("The presentation remote is switched off on the PC (Aura > Remote)"))
        assertEquals("PC is locked", WatchRemoteText.short("The PC is locked, so the remote does nothing"))
        assertEquals("PC out of reach", WatchRemoteText.short("PC not reachable"))
    }

    @Test fun nothing_is_sent_before_pairing_or_when_extras_are_off() {
        assertEquals("Pair with PC first", WatchAura.ring(ctx, true))
        PcLinkPrefs(ctx).apply { pcAddress = "02:00:00:AA:BB:CD"; pcId = "0011223344556677"; pcName = "MYLAPTOP" }
        WatchAura.setExtras(ctx, false)
        assertEquals("Remote extras are off in Settings", WatchAura.present(ctx, PresentCmd.NEXT))
        assertEquals("Remote extras are off in Settings", WatchAura.mediaCmd(ctx, MediaCmd.PLAY_PAUSE))
        assertTrue(WatchAura.extrasOn(ctx).not())
        WatchAura.setExtras(ctx, true); assertTrue(WatchAura.extrasOn(ctx))
    }

    @Test fun new_cues_exist_and_are_strong() {
        assertNotNull(Feedback.Cue.valueOf("SLIDE")); assertNotNull(Feedback.Cue.valueOf("SEND"))
        assertEquals(0x42, Kind.RING); assertEquals(0x44, 0x44)
    }
}
