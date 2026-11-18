package com.spacez.unlock_stage1

import com.spacez.unlock_stage1.update.UpdateLogic
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class UpdateLogicTest {
    private val cert = byteArrayOf(1, 2, 3)

    @Test fun namesAreParsed() {
        assertEquals(UpdateLogic.Name("1.0.0", 20), UpdateLogic.parseName("Aura-1.0.0+20.apk"))
        for (bad in listOf("Aura-1.0.0.apk", "Aura-1.0.0+x.apk", "aura-1.0.0+2.apk", "Aura-1.0.0+2.apk.part", "Other-1.0+2.apk", "../Aura-1.0.0+2.apk"))
            assertNull(bad, UpdateLogic.parseName(bad))
    }

    @Test fun onlyHigherBuildWithSameCertAndPackageIsOffered() {
        val p = "com.spacez.unlock_stage1"
        assertNull(UpdateLogic.verdict(p, p, 20, 19, listOf(cert), listOf(byteArrayOf(1, 2, 3))))
        assertNotNull(UpdateLogic.verdict(p, p, 19, 19, listOf(cert), listOf(cert)))
        assertNotNull(UpdateLogic.verdict(p, p, 18, 19, listOf(cert), listOf(cert)))
        assertNotNull(UpdateLogic.verdict("evil.app", p, 20, 19, listOf(cert), listOf(cert)))
        assertNotNull(UpdateLogic.verdict(p, p, 20, 19, listOf(byteArrayOf(9)), listOf(cert)))
        assertNotNull(UpdateLogic.verdict(p, p, 20, 19, emptyList(), listOf(cert)))
        assertNotNull(UpdateLogic.verdict(p, p, 20, 19, listOf(cert), emptyList()))
    }

    @Test fun signerSetsCompareByContent() {
        assertTrue(UpdateLogic.sameSigners(listOf(byteArrayOf(1), byteArrayOf(2)), listOf(byteArrayOf(2), byteArrayOf(1))))
        assertFalse(UpdateLogic.sameSigners(listOf(byteArrayOf(1)), listOf(byteArrayOf(1), byteArrayOf(2))))
    }
}
