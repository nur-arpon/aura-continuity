package com.spacez.unlock_stage1

import com.spacez.unlock_stage1.FeedbackPlan.Cue
import com.spacez.unlock_stage1.away.WidgetLogic
import com.spacez.unlock_stage1.handoff.HandoffLogic
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** Item 13 (premium Aura phone app): haptic plans, the widget's status line, inbox sizes. */
class AuraAppLogicTest {
    @Test fun every_cue_has_a_pattern_and_max_is_the_strongest() {
        for (c in Cue.entries) {
            assertTrue("$c primitives", FeedbackPlan.primitives(c).isNotEmpty())
            val max = FeedbackPlan.waveform(c, FeedbackPlan.scale(3))
            val light = FeedbackPlan.waveform(c, FeedbackPlan.scale(1))
            assertEquals("$c timings/amps", max.timings.size, max.amps.size)
            assertTrue("$c max >= light", max.amps.max() >= light.amps.max())
            assertTrue("$c amps in range", max.amps.all { it in 0..255 })
            FeedbackPlan.primitives(c).forEach { assertTrue("$c scale", it.s in 0f..1f) }
        }
        // the default intensity (3) is full strength
        assertEquals(1f, FeedbackPlan.scale(3), 0f)
        assertTrue(FeedbackPlan.scale(1) < FeedbackPlan.scale(2))
    }

    @Test fun actions_feel_different() {
        val lock = FeedbackPlan.primitives(Cue.LOCK).map { it.id }
        val unlock = FeedbackPlan.primitives(Cue.UNLOCK).map { it.id }
        val send = FeedbackPlan.primitives(Cue.SEND).map { it.id }
        val error = FeedbackPlan.primitives(Cue.ERROR).map { it.id }
        assertNotEquals(lock, unlock)
        assertNotEquals(lock, error)
        assertNotEquals(send, unlock)
        val pats = Cue.entries.map { FeedbackPlan.waveform(it, 1f).timings.toList() + FeedbackPlan.waveform(it, 1f).amps.toList() }
        assertEquals("every cue has its own waveform", pats.size, pats.toSet().size)
    }

    @Test fun cue_names_from_flutter() {
        assertEquals(Cue.LOCK, Cue.from("lock"))
        assertEquals(Cue.TOGGLE_ON, Cue.from("toggle_on"))
        assertEquals(null, Cue.from("nope"))
    }

    @Test fun sounds_respect_silent_and_dnd() {
        assertFalse(FeedbackPlan.quiet(ringerNormal = true, interruptionsAll = true))
        assertTrue(FeedbackPlan.quiet(ringerNormal = false, interruptionsAll = true))
        assertTrue(FeedbackPlan.quiet(ringerNormal = true, interruptionsAll = false))
    }

    @Test fun widget_status_line() {
        assertEquals("Not paired", WidgetLogic.state(false, null, false, false).text)
        assertEquals("Locked · nearby", WidgetLogic.state(true, true, true, false).text)
        assertEquals("Locked", WidgetLogic.state(true, true, false, true).text)
        assertEquals(WidgetLogic.State("Nearby", WidgetLogic.Tone.GOOD), WidgetLogic.state(true, false, true, true))
        assertEquals("Away · Tailscale", WidgetLogic.state(true, null, false, true).text)
        assertEquals(WidgetLogic.Tone.WARN, WidgetLogic.state(true, null, false, false).tone)
    }

    @Test fun inbox_sizes() {
        assertEquals("900 B", HandoffLogic.size(900))
        assertEquals("812 KB", HandoffLogic.size(812 * 1024))
        assertEquals("4.2 MB", HandoffLogic.size(4_400_000))
        assertEquals("1.50 GB", HandoffLogic.size(1_610_612_736))
    }
}
