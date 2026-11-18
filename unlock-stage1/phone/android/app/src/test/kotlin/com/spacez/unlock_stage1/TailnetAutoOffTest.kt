package com.spacez.unlock_stage1

import com.spacez.unlock_stage1.away.TailnetAutoOff
import com.spacez.unlock_stage1.away.TailnetAutoOff.Ev
import com.spacez.unlock_stage1.away.TailnetAutoOff.Verdict
import com.spacez.unlock_stage1.handoff.NotifCounts
import org.junit.Assert.assertEquals
import org.junit.Test

class TailnetAutoOffTest {
    private val art = "com.limelight.noir"

    @Test fun streamStateFollowsResumeAndPause() {
        assertEquals(false to null, TailnetAutoOff.streamState(emptyList()))
        assertEquals(true to 100L, TailnetAutoOff.streamState(listOf(Ev(art, 1, 100))))
        assertEquals(false to 500L, TailnetAutoOff.streamState(listOf(Ev(art, 1, 100), Ev(art, 2, 500))))
        // events from other apps are ignored; Moonlight counts too; order of the list does not matter
        assertEquals(false to 700L, TailnetAutoOff.streamState(listOf(Ev("com.limelight", 23, 700), Ev("com.other", 1, 800), Ev(art, 1, 100))))
    }

    @Test fun offOnlyTwoMinutesAfterLeavingAndWhenNothingElseRuns() {
        val armed = 1_000_000L
        // streaming right now: never
        assertEquals(Verdict.WAIT, TailnetAutoOff.decide(armed + 3_600_000, armed, true, armed + 10, false))
        // left 119 s ago: wait; 120 s ago: off
        assertEquals(Verdict.WAIT, TailnetAutoOff.decide(armed + 600_000 + 119_000, armed, false, armed + 600_000, false))
        assertEquals(Verdict.OFF, TailnetAutoOff.decide(armed + 600_000 + 120_000, armed, false, armed + 600_000, false))
        // another away-action running (lock / wake / unlock over the tailnet): wait
        assertEquals(Verdict.WAIT, TailnetAutoOff.decide(armed + 600_000 + 300_000, armed, false, armed + 600_000, true))
        // the stream never opened: counted from the moment Aura turned Tailscale on
        assertEquals(Verdict.WAIT, TailnetAutoOff.decide(armed + 60_000, armed, false, null, false))
        assertEquals(Verdict.OFF, TailnetAutoOff.decide(armed + 120_000, armed, false, null, false))
        // an old stream event from before arming does not shorten the wait
        assertEquals(Verdict.WAIT, TailnetAutoOff.decide(armed + 60_000, armed, false, armed - 500_000, false))
    }

    @Test fun unreadCountsSkipOngoingAndSummaries() {
        // (package, clearable and not ongoing, not a group summary)
        val c = NotifCounts.tally(listOf(
            Triple("chat", true, true), Triple("chat", true, true), Triple("chat", true, false),
            Triple("music", false, true), Triple("mail", true, true)))
        assertEquals(mapOf("chat" to 2, "mail" to 1), c)
    }
}
