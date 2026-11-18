package com.spacez.unlock_stage1

import android.content.Intent
import com.spacez.unlock_stage1.extras.Moonlight
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

/** "Open laptop" only launches Moonlight / Artemis (or its Play page) and learns the PC's addresses from Aura Link. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class MoonlightTest {
    private val app get() = RuntimeEnvironment.getApplication()
    private val infoJson = """{"v":1,"host":"OWNER","lan":["192.168.1.5"],"tailnet":"100.70.1.2","port":47989,"installed":true,"running":true}"""

    private fun install(pkg: String) = shadowOf(app.packageManager).installPackage(android.content.pm.PackageInfo().apply { packageName = pkg })

    @Test fun parses_the_pc_message_and_drops_public_addresses() {
        val i = Moonlight.parseInfo(infoJson)!!
        assertEquals(listOf("192.168.1.5"), i.lan); assertEquals("100.70.1.2", i.tailnet); assertEquals(47989, i.port)
        val bad = Moonlight.parseInfo("""{"lan":["8.8.8.8","10.0.0.2"],"tailnet":"8.8.4.4","port":80}""")!!
        assertEquals(listOf("10.0.0.2"), bad.lan); assertNull(bad.tailnet); assertEquals(47989, bad.port)
        assertNull(Moonlight.parseInfo("""{"lan":["8.8.8.8"],"tailnet":null}""")); assertNull(Moonlight.parseInfo("nope"))
    }

    @Test fun home_uses_lan_away_uses_tailnet() {
        val i = Moonlight.parseInfo(infoJson)!!
        assertEquals("192.168.1.5", Moonlight.bestAddress(i, listOf("10.1.2.3", "192.168.1.77")))
        assertEquals("100.70.1.2", Moonlight.bestAddress(i, listOf("100.70.1.9", "10.20.30.40")))
        assertEquals("192.168.1.5", Moonlight.bestAddress(i.copy(tailnet = null), emptyList()))
    }

    @Test fun missing_client_goes_to_play_store() {
        val msg = Moonlight.open(app)
        assertTrue(msg.startsWith("Install Moonlight"))
        val i = shadowOf(app).nextStartedActivity
        assertEquals(Intent.ACTION_VIEW, i.action)
        assertTrue(i.data.toString().contains("id=com.limelight"))
    }

    @Test fun installed_client_is_launched_and_the_address_copied() {
        install(Moonlight.PKG)
        shadowOf(app.packageManager).addActivityIfNotPresent(android.content.ComponentName(Moonlight.PKG, "com.limelight.PcView"))
        shadowOf(app.packageManager).addIntentFilterForActivity(android.content.ComponentName(Moonlight.PKG, "com.limelight.PcView"),
            android.content.IntentFilter(Intent.ACTION_MAIN).apply { addCategory(Intent.CATEGORY_LAUNCHER) })
        Moonlight.save(app, infoJson)
        val msg = Moonlight.open(app)
        assertTrue(msg, msg.startsWith("Opening your laptop in Moonlight"))
        assertNotNull(shadowOf(app).nextStartedActivity)
    }

    @Test fun artemis_wins_when_both_are_installed() {
        install(Moonlight.PKG); install(Moonlight.ARTEMIS_PKG)
        assertEquals(Moonlight.ARTEMIS_PKG, Moonlight.clientPackage(app))
    }
}
