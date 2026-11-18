package com.spacez.unlock_stage1

import com.spacez.unlock_stage1.handoff.CatalogLogic
import com.spacez.unlock_stage1.handoff.CatalogLogic.App
import org.junit.Assert.assertEquals
import org.junit.Test

class CatalogLogicTest {
    private fun a(id: String, recent: Boolean = false) = App(id, id.uppercase(), recent)

    @Test fun firstPartStartsFreshLaterPartsAdd() {
        val old = listOf(a("x"))
        val p0 = CatalogLogic.merge(old, 0, listOf(a("a"), a("b")))
        assertEquals(listOf("a", "b"), p0.map { it.id })
        val p1 = CatalogLogic.merge(p0, 1, listOf(a("c"), a("a")))
        assertEquals(listOf("b", "c", "a"), p1.map { it.id })
    }

    @Test fun topIsOpenedFirstThenRecentWindowsThenListOrder() {
        val apps = listOf(a("w"), a("code", true), a("brave", true), a("zed"), a("calc"))
        assertEquals(listOf("zed", "code", "brave", "w"), CatalogLogic.top(apps, listOf("zed")).map { it.id })
        assertEquals(listOf("code", "brave", "w", "zed"), CatalogLogic.top(apps, emptyList()).map { it.id })
        assertEquals(listOf("calc", "zed", "code", "brave"), CatalogLogic.top(apps, listOf("calc", "zed", "gone")).map { it.id })
    }

    @Test fun shortcutIdIsStable() = assertEquals("lap-code", CatalogLogic.shortcutId("code"))
}
