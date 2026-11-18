package com.spacez.unlock_stage1.wear

import android.content.Context
import android.content.Intent
import androidx.compose.ui.test.junit4.createEmptyComposeRule
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.swipeUp
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.core.app.ActivityScenario
import androidx.test.core.app.ApplicationProvider
import com.github.takahirom.roborazzi.captureRoboImage
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/** Renders the real screens on a 454 px round watch (Galaxy Watch7 class) into screenshots/render/. */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w227dp-h227dp-small-notlong-round-watch-xhdpi-keyshidden-nonav")
class ScreenshotTest {
    @get:Rule val compose = createEmptyComposeRule()
    private val ctx: Context get() = ApplicationProvider.getApplicationContext()
    private val out = "../screenshots/render"

    private fun paired(event: PcEvent? = PcEvent.LOCK_SENT) {
        PcLinkPrefs(ctx).apply { pcAddress = "02:00:00:AA:BB:CD"; pcId = "0011223344556677"; pcName = "MYLAPTOP"; enabled = true }
        WatchPrefs(ctx).paired = true
        event?.let { WatchPrefs(ctx).recordPc(it) }
    }

    private fun launch(route: String = "home"): ActivityScenario<MainActivity> {
        org.robolectric.Shadows.shadowOf(ctx as android.app.Application).grantPermissions(
            android.Manifest.permission.POST_NOTIFICATIONS, android.Manifest.permission.BLUETOOTH_CONNECT, android.Manifest.permission.BLUETOOTH_ADVERTISE)
        return ActivityScenario.launch(Intent(ctx, MainActivity::class.java).putExtra("route", route))
    }

    private fun shot(name: String) { compose.waitForIdle(); compose.mainClock.advanceTimeBy(1500); compose.onRoot().captureRoboImage("$out/$name.png") }

    @Test fun homeUnpaired() { launch(); shot("01-home-unpaired") }
    @Test fun homeLocked() { paired(); launch(); shot("02-home-locked") }
    @Test fun homeUnlocked() { paired(PcEvent.UNLOCK_APPROVED); launch(); shot("02b-home-unlocked") }
    @Test fun homeOutOfReach() { paired(PcEvent.LOCK_FAILED); launch(); shot("02c-home-out-of-reach") }
    @Test fun homeScrolled() { paired(); launch(); compose.onRoot().performTouchInput { swipeUp() }; shot("02d-home-scrolled") }
    @Test fun homeTestMode() { paired(); WatchPrefs(ctx).testUntil = System.currentTimeMillis() + 1_800_000; launch(); shot("02e-home-testmode") }
    @Test fun pairCountdown() { launch("pair"); shot("03-pair-countdown") }
    @Test fun pairSuccess() {
        launch("pair"); compose.mainClock.advanceTimeBy(800)
        paired(PcEvent.PAIRED); shot("04-pair-success")
    }
    @Test fun settings() { paired(); launch("settings"); shot("05-settings") }
    @Test fun settingsScrolled() { paired(); launch("settings"); compose.onRoot().performTouchInput { swipeUp() }; shot("05b-settings-scrolled") }
    @Test fun about() { paired(); launch("about"); shot("06-about") }
    @Test fun remoteMenu() { paired(); launch("remote"); shot("09-remote-menu") }
    @Test fun music() {
        paired()
        WatchAura.injectMediaForTest(com.spacez.aura.link.MediaState(true, 1000, 200000, "Kannil Kannil", "Kunal Ganjawala", 62, false))
        launch("music"); shot("10-music")
    }
    @Test fun mouse() { paired(); launch("mouse"); shot("12-mouse") }
    @Test fun remoteScrolled() { paired(); launch("remote"); compose.onRoot().performTouchInput { swipeUp() }; shot("09b-remote-scrolled") }
    @Test fun settingsBottom() { paired(); launch("settings"); repeat(3) { compose.onRoot().performTouchInput { swipeUp() } }; shot("05c-settings-bottom") }
    @Test fun settingsEnd() { paired(); launch("settings"); repeat(6) { compose.onRoot().performTouchInput { swipeUp() } }; shot("05d-settings-end") }
    @Test fun aboutBottom() { paired(); launch("about"); repeat(4) { compose.onRoot().performTouchInput { swipeUp() } }; shot("06b-about-bottom") }
    @Test fun slides() { paired(); launch("slides"); shot("11-slides") }
    @Test fun homeWithRemote() { paired(); launch(); compose.onRoot().performTouchInput { swipeUp() }; shot("02f-home-remote-row") }
    @Test fun lockFailed() { paired(PcEvent.UNLOCK_APPROVED); launch(); compose.onNodeWithText("Lock PC").performClick(); compose.mainClock.advanceTimeBy(600); shot("07-lock-failed") }
}

/** Renders the real tile layout with the platform tile renderer (same inflater the watch uses). */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w227dp-h227dp-small-notlong-round-watch-xhdpi-keyshidden-nonav")
class TileScreenshotTest {
    private val ctx: Context get() = ApplicationProvider.getApplicationContext()

    private fun render(name: String) {
        val dm = ctx.resources.displayMetrics
        val device = androidx.wear.protolayout.DeviceParametersBuilders.DeviceParameters.Builder()
            .setScreenWidthDp(227).setScreenHeightDp(227).setScreenDensity(dm.density)
            .setScreenShape(androidx.wear.protolayout.DeviceParametersBuilders.SCREEN_SHAPE_ROUND)
            .setDevicePlatform(androidx.wear.protolayout.DeviceParametersBuilders.DEVICE_PLATFORM_WEAR_OS).build()
        val svc = StatusTileService()
        val layout = androidx.wear.protolayout.LayoutElementBuilders.Layout.Builder().setRoot(svc.layout(ctx, device)).build()
        val parent = android.widget.FrameLayout(ctx).apply { setBackgroundColor(Palette.NAVY_950) }
        val activity = org.robolectric.Robolectric.buildActivity(android.app.Activity::class.java).setup().get()
        activity.setContentView(parent, android.view.ViewGroup.LayoutParams(454, 454))
        val r = androidx.wear.tiles.renderer.TileRenderer(activity, java.util.concurrent.Executors.newSingleThreadExecutor()) {}
        val f = r.inflateAsync(layout, svc.resources(), parent)
        org.robolectric.shadows.ShadowLooper.idleMainLooper()
        repeat(20) { if (!f.isDone) { Thread.sleep(50); org.robolectric.shadows.ShadowLooper.idleMainLooper() } }
        f.get()
        parent.measure(android.view.View.MeasureSpec.makeMeasureSpec(454, android.view.View.MeasureSpec.EXACTLY), android.view.View.MeasureSpec.makeMeasureSpec(454, android.view.View.MeasureSpec.EXACTLY))
        parent.layout(0, 0, 454, 454)
        parent.captureRoboImage("../screenshots/render/$name.png")
    }

    @Test fun tileLocked() {
        PcLinkPrefs(ctx).apply { pcAddress = "02:00:00:AA:BB:CD"; pcId = "00"; pcName = "MYLAPTOP"; enabled = true }
        WatchPrefs(ctx).recordPc(PcEvent.LOCK_SENT); WatchPrefs(ctx).last = "On wrist · Unlocked · Ready"
        render("08-tile-locked")
    }
    @Test fun tileUnpaired() { render("08b-tile-unpaired") }
}
