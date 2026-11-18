package com.spacez.unlock_stage1

import com.spacez.unlock_stage1.handoff.NotifLogic
import com.spacez.unlock_stage1.handoff.PhoneHubLogic
import com.spacez.unlock_stage1.handoff.PhoneHubLogic.S
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class NowPlayingEverywhereTest {
    private val own = "com.spacez.unlock_stage1"

    @Test fun mirrorsAudibleChatNotifications() {
        assertTrue(NotifLogic.mirror(own, "com.whatsapp", false, false, 4, "msg", true))
    }

    @Test fun skipsAuraOngoingSilentSummaryAndSystem() {
        assertFalse(NotifLogic.mirror(own, own, false, false, 4, null, true))
        assertFalse(NotifLogic.mirror(own, "x", true, false, 4, null, true))
        assertFalse(NotifLogic.mirror(own, "x", false, false, 2, null, true))
        assertFalse(NotifLogic.mirror(own, "x", false, true, 4, null, true))
        assertFalse(NotifLogic.mirror(own, "x", false, false, 4, "transport", true))
        assertFalse(NotifLogic.mirror(own, "android", false, false, 4, null, true))
        assertFalse(NotifLogic.mirror(own, "x", false, false, 4, null, false))
    }

    @Test fun hubResendsOnlyOnRealChange() {
        val a = listOf(S("p", "Song", "Me", true, 10_000))
        assertFalse(PhoneHubLogic.changed(a, listOf(S("p", "Song", "Me", true, 12_000)), 2_000))
        assertTrue(PhoneHubLogic.changed(a, listOf(S("p", "Song", "Me", false, 12_000)), 2_000))
        assertTrue(PhoneHubLogic.changed(a, listOf(S("p", "Song", "Me", true, 60_000)), 2_000)) // a seek
        assertTrue(PhoneHubLogic.changed(a, listOf(S("p", "Other", "Me", true, 12_000)), 2_000))
        assertTrue(PhoneHubLogic.changed(null, a, 0))
        assertTrue(PhoneHubLogic.changed(a, emptyList(), 0))
    }
}
