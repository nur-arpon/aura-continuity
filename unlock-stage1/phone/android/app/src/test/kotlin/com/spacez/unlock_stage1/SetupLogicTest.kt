package com.spacez.unlock_stage1

import com.spacez.unlock_stage1.setup.SetupLogic
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class SetupLogicTest {
    private val none = SetupLogic.ORDER.associateWith { false }

    @Test fun walkGoesInOrderAndEnds() {
        var g = none
        val seen = mutableListOf<String>()
        while (true) { val n = SetupLogic.next(g) ?: break; seen += n; g = g + (n to true) }
        assertEquals(SetupLogic.ORDER, seen)
        assertNull(SetupLogic.next(g))
    }

    @Test fun grantedAndDeclinedStepsAreNotAskedAgain() {
        val g = none + ("notifications" to true) + ("bluetooth" to true)
        assertEquals("usage", SetupLogic.next(g))
        assertEquals("listener", SetupLogic.next(g, skipped = setOf("usage")))
        assertNull(SetupLogic.next(g, skipped = SetupLogic.ORDER.toSet()))
    }

    @Test fun unavailableStepsAreIgnored() {
        val avail = SetupLogic.ORDER - "bubbles"
        val g = SetupLogic.ORDER.associateWith { it != "bubbles" }
        assertNull(SetupLogic.next(g, available = avail))
        assertEquals(emptyList<String>(), SetupLogic.missing(g, avail))
        assertEquals(listOf("bubbles"), SetupLogic.missing(g))
    }

    @Test fun watchStates() {
        assertEquals("none", SetupLogic.watchState(0, 0, false, false))
        assertEquals("app-missing", SetupLogic.watchState(1, 0, false, false))
        assertEquals("unpaired", SetupLogic.watchState(1, 1, false, false))
        assertEquals("connected", SetupLogic.watchState(1, 1, true, true))
        assertEquals("away", SetupLogic.watchState(1, 1, true, false))
        assertEquals("away", SetupLogic.watchState(0, 0, true, false))
    }
}
