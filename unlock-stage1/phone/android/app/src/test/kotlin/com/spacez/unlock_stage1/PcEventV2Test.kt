package com.spacez.unlock_stage1

import com.spacez.unlock_stage1.watchlink.PcEvent
import com.spacez.unlock_stage1.watchlink.PcEventFrame
import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * The phone's own key derivation (UnlockCrypto.levelKey / pcIdOf, the code that imports the Keystore keys)
 * must give the level-1 key and pc_id the PC derives, and the lock event built from them must match the
 * shared vector byte for byte (Rust verifies the same bytes in autolock events.rs and watch-hint).
 */
class PcEventV2Test {
    private val v = java.io.File("../../../../unlock-watch/shared/test-vectors/pc-event-v2.txt").readLines()
        .filter { it.isNotBlank() && !it.startsWith("#") }.associate { it.substringBefore('=') to it.substringAfter('=') }

    @Test fun phoneDerivationSignsWithLevelOneKey() {
        val secret = v.getValue("secret").toByteArray()
        val key1 = UnlockCrypto.levelKey(secret, UnlockCrypto.AUTO)
        assertEquals(v.getValue("key1"), key1.toHex())
        val pcId = UnlockCrypto.pcIdOf(secret)
        assertEquals(v.getValue("pc_id"), pcId.toHex())
        val mac = javax.crypto.Mac.getInstance("HmacSHA256").apply { init(javax.crypto.spec.SecretKeySpec(key1, "HmacSHA256")) }
        val frame = PcEventFrame.build(mac, PcEvent.LOCK_NOW, 1_700_000_000_000L, 7, pcId)
        assertEquals(v.getValue("frame"), frame.toHex())
        assertEquals(v.getValue("wire"), PcEventFrame.wire(frame).toHex())
    }

    /** Level-0 fallback (phone locked > 5 min): same bytes autolock events.rs accepts as lock-only. */
    @Test fun noAuthLevelZeroFrameMatchesTheSharedVector() {
        val secret = v.getValue("secret").toByteArray()
        val key0 = UnlockCrypto.levelKey(secret, UnlockCrypto.DENY)
        assertEquals(v.getValue("key0"), key0.toHex())
        val mac = javax.crypto.Mac.getInstance("HmacSHA256").apply { init(javax.crypto.spec.SecretKeySpec(key0, "HmacSHA256")) }
        val frame = PcEventFrame.build(mac, PcEvent.HINT_OFF_WRIST, 1_700_000_000_000L, 8, UnlockCrypto.pcIdOf(secret))
        assertEquals(v.getValue("frame0"), frame.toHex())
        assertEquals(v.getValue("wire0"), PcEventFrame.wire(frame).toHex())
    }

    @Test fun otherLevelKeysDoNotMatch() {
        val secret = v.getValue("secret").toByteArray()
        for (lvl in listOf(0, 2, 3)) assert(UnlockCrypto.levelKey(secret, lvl).toHex() != v.getValue("key1"))
    }
}
