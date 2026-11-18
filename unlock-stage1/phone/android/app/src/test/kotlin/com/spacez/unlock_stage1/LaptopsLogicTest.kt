package com.spacez.unlock_stage1

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class LaptopsLogicTest {
    private fun c(id: String, up: Boolean, last: Long = 0, added: Long = 0) = LaptopsLogic.Cand(id, up, last, added)

    @Test fun onlyThePrimaryIsAlwaysThePrimary() {
        assertEquals("p", LaptopsLogic.pick(listOf(c("p", false)), null))
        assertEquals("p", LaptopsLogic.pick(listOf(c("p", true, 5)), null))
        assertNull(LaptopsLogic.pick(emptyList(), null))
    }

    @Test fun aConnectedLaptopBeatsADisconnectedOne() {
        assertEquals("s", LaptopsLogic.pick(listOf(c("p", false, 900), c("s", true, 10)), null))
    }

    @Test fun amongConnectedTheMostRecentSessionWins() {
        assertEquals("s", LaptopsLogic.pick(listOf(c("p", true, 100), c("s", true, 200)), null))
        assertEquals("p", LaptopsLogic.pick(listOf(c("p", true, 300), c("s", true, 200)), null))
    }

    @Test fun nothingConnectedMeansTheMostRecentlyLinkedOrConnected() {
        assertEquals("s", LaptopsLogic.pick(listOf(c("p", false, 0, 0), c("s", false, 0, 50)), null)) // paired last
        assertEquals("p", LaptopsLogic.pick(listOf(c("p", false, 500, 0), c("s", false, 100, 50)), null)) // used last
    }

    @Test fun aPinnedLaptopAlwaysWinsWhileItExists() {
        assertEquals("p", LaptopsLogic.pick(listOf(c("p", false), c("s", true, 999)), "p"))
        assertEquals("s", LaptopsLogic.pick(listOf(c("p", false), c("s", true, 999)), "gone")) // pin to a removed laptop = automatic
    }

    @Test fun qrExtrasAreCleaned() {
        val x = LaptopsLogic.cleanExtras("companion", "  HOMESERVER\u0007 ", "aa:bb:cc:dd:ee:ff", "100.88.77.66")
        assertEquals(LaptopsLogic.QrExtras("companion", "HOMESERVER", "AA:BB:CC:DD:EE:FF", "100.88.77.66"), x)
        val bad = LaptopsLogic.cleanExtras("admin", "", "not-a-mac", "8.8.8.8")
        assertEquals(LaptopsLogic.QrExtras(null, null, null, null), bad)
    }

    private fun r(id: String, name: String?, addr: String?) = LaptopsLogic.Ref(id, name, addr)
    private fun q(name: String?, addr: String?) = LaptopsLogic.QrExtras("companion", name, addr, null)

    @Test fun anExtraPhoneRepairsItsPrimarySlotForTheSamePc() {
        val p = LaptopsLogic.plan("extra", r("old", "MYLAPTOP", "AA:BB:CC:DD:EE:01"), emptyList(), "new", q("MYLAPTOP", "AA:BB:CC:DD:EE:01"))
        assertEquals(LaptopsLogic.Plan(true, emptyList(), false), p)
    }

    @Test fun primarySlotMatchesByNameWhenTheAddressIsUnknown() {
        assertEquals(true, LaptopsLogic.plan("extra", r("old", "mylaptop", null), emptyList(), "new", q("MYLAPTOP", "AA:BB:CC:DD:EE:01")).replacePrimary)
        assertEquals(true, LaptopsLogic.plan("extra", r("old", "MYLAPTOP", "AA:BB:CC:DD:EE:01"), emptyList(), "new", q("MYLAPTOP", null)).replacePrimary)
    }

    @Test fun differentAddressesAreDifferentLaptopsEvenWithTheSameName() {
        val p = LaptopsLogic.plan("extra", r("old", "MYLAPTOP", "AA:BB:CC:DD:EE:01"), emptyList(), "new", q("MYLAPTOP", "AA:BB:CC:DD:EE:02"))
        assertEquals(LaptopsLogic.Plan(false, emptyList(), true), p)
    }

    @Test fun aMainPhoneNeverRepairsItsPrimaryFromACompanionQr() {
        val p = LaptopsLogic.plan("main", r("old", "MYLAPTOP", "AA:BB:CC:DD:EE:01"), emptyList(), "new", q("MYLAPTOP", "AA:BB:CC:DD:EE:01"))
        assertEquals(LaptopsLogic.Plan(false, emptyList(), true), p)
    }

    @Test fun aSecondaryForTheSamePcIsReplacedNotDuplicated() {
        val p = LaptopsLogic.plan("main", r("p", "WORK", "11:11:11:11:11:11"), listOf(r("s1", "HOMESERVER", null), r("s2", "OTHER", null)), "new", q("HOMESERVER", null))
        assertEquals(LaptopsLogic.Plan(false, listOf("s1"), true), p)
    }

    @Test fun rescanningTheSameCodeReplacesItsOwnEntry() {
        val p = LaptopsLogic.plan("main", null, listOf(r("s1", "Laptop", null)), "s1", q(null, null))
        assertEquals(LaptopsLogic.Plan(false, listOf("s1"), true), p)
    }

    @Test fun repairingThePrimaryAlsoDropsTheDuplicateSecondary() {
        val p = LaptopsLogic.plan("extra", r("old", "MYLAPTOP", null), listOf(r("new", "MYLAPTOP", "AA:BB:CC:DD:EE:01")), "new", q("MYLAPTOP", "AA:BB:CC:DD:EE:01"))
        assertEquals(LaptopsLogic.Plan(true, listOf("new"), false), p)
    }

    @Test fun noNameAndNoAddressNeverMatches() {
        assertEquals(false, LaptopsLogic.samePc(null, null, null, null))
        assertEquals(false, LaptopsLogic.samePc("", null, "", null))
    }
}
