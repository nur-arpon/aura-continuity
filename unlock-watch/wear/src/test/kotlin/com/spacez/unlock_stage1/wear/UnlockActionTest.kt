package com.spacez.unlock_stage1.wear

import android.content.Context
import androidx.test.core.app.ApplicationProvider
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/** The watch refuses to push "Unlock PC" unless paired AND worn + unlocked (no off-body sensor here = not worn). */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class UnlockActionTest {
    private val ctx: Context get() = ApplicationProvider.getApplicationContext()

    @Test fun refusesWhenNotPaired() = runBlocking {
        assertEquals(UnlockResult.NOT_PAIRED, UnlockAction.run(ctx))
    }

    @Test fun refusesWhenNotWornWithoutSending() = runBlocking {
        PcLinkPrefs(ctx).apply { pcAddress = "02:00:00:AA:BB:CD"; pcId = "0011223344556677"; pcName = "MYLAPTOP" }
        assertEquals(UnlockResult.NOT_READY, UnlockAction.run(ctx))
        assertEquals(null, WatchPrefs(ctx).pcEvent) // nothing recorded, nothing sent
    }
}
