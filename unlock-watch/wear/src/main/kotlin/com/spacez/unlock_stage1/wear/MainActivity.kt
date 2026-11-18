package com.spacez.unlock_stage1.wear

import android.Manifest
import android.content.Context
import android.content.SharedPreferences
import android.content.pm.PackageManager
import android.os.Bundle
import android.util.Log
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.lifecycleScope
import androidx.wear.compose.material3.AppScaffold
import androidx.wear.compose.navigation.SwipeDismissableNavHost
import androidx.wear.compose.navigation.composable
import androidx.wear.compose.navigation.rememberSwipeDismissableNavController
import com.google.android.gms.wearable.Wearable
import com.spacez.unlock_stage1.watchlink.Kind
import com.spacez.unlock_stage1.watchlink.WatchProtocol
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.tasks.await
import kotlinx.coroutines.withContext

/** Everything the screens show. Rebuilt from prefs on every change (listener, no polling). */
data class UiState(
    val pcPaired: Boolean = false,
    val pcName: String = "",
    val pc: PcSummary = PcSummary(PcLook.IDLE, "…", "", 0),
    val handsFree: Boolean = false,
    val lockWhenOff: Boolean = false,
    val phonePaired: Boolean = false,
    val phoneReachable: Boolean? = null,
    val wristKnown: Boolean = false,
    val onWrist: Boolean = false,
    val watchLocked: Boolean = false,
    val wristRead: Boolean = false,
    val listening: Boolean = false,
    val testUntil: Long = 0,
    val fingerprint: String = "",
    val pairUntil: Long = 0,
    val lastEvent: PcEvent? = null,
    val lastEventMs: Long = 0,
    val sound: Boolean = true,
    val haptics: Int = 3,
    val pcAddress: String = "",
    val ble: String = "",
    val permissionsOk: Boolean = true,
    val expressive: Boolean = true,
    val colour: String = COLOUR_WATCH,
) {
    val anyPaired get() = pcPaired || phonePaired
    val ready get() = wristKnown && onWrist && !watchLocked
    val testActive get() = System.currentTimeMillis() < testUntil
}

/**
 * Home, Pair with PC, Settings, About. Nothing runs while the screen is off: the state is read
 * on resume and on pref changes, and every button is a single message. Not an always-on app.
 */
class MainActivity : ComponentActivity() {
    var ui by mutableStateOf(UiState())
        private set
    private val prefListener = SharedPreferences.OnSharedPreferenceChangeListener { _, _ -> reload() }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        askPermissions()
        val start = intent?.getStringExtra("route") ?: "home"
        WatchPrefs(this).let { ui = ui.copy(expressive = it.expressive, colour = it.colour) }
        setContent { ContinuityTheme(ui.expressive, ui.colour) { App(this, start) } }
    }

    private val needed = arrayOf(Manifest.permission.POST_NOTIFICATIONS, Manifest.permission.BLUETOOTH_CONNECT, Manifest.permission.BLUETOOTH_ADVERTISE)
    private fun missing() = needed.filter { checkSelfPermission(it) != PackageManager.PERMISSION_GRANTED }
    fun askPermissions() { missing().takeIf { it.isNotEmpty() }?.let { requestPermissions(it.toTypedArray(), 1) } }

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<String>, grantResults: IntArray) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        PcLinkService.sync(this)
        reload()
    }

    override fun onStart() {
        super.onStart()
        WatchPrefs(this).register(prefListener)
        PcLinkPrefs(this).register(prefListener)
    }

    override fun onResume() {
        super.onResume()
        PcLinkService.sync(this) // the app is visible: (re)start the listener if it is wanted
        Feedback.warm(this)
        reload()
        lifecycleScope.launch {
            val s = withContext(Dispatchers.IO) {
                WatchState.snapshot(this@MainActivity, Kind.PUSH, ByteArray(WatchProtocol.NONCE_LEN), sign = false)
            }
            WatchState.remember(this@MainActivity, s)
            ui = ui.copy(wristKnown = s.wristKnown, onWrist = s.onWrist, watchLocked = s.deviceLocked, wristRead = true)
            Log.i(TAG, "app opened: ${WatchState.describe(s)} keyguard=${s.keyguardLocked} sensor=${WatchState.offBodySensor(this@MainActivity) != null}")
            val reach = runCatching {
                Wearable.getNodeClient(this@MainActivity).connectedNodes.await().any { it.isNearby }
            }.getOrNull()
            ui = ui.copy(phoneReachable = reach)
        }
    }

    override fun onStop() {
        WatchPrefs(this).unregister(prefListener)
        PcLinkPrefs(this).unregister(prefListener)
        Feedback.release()
        super.onStop()
    }

    fun reload() {
        val lp = PcLinkPrefs(this)
        val wp = WatchPrefs(this)
        lifecycleScope.launch {
            val fp = if (ui.fingerprint.isEmpty()) withContext(Dispatchers.IO) { runCatching { WatchKeys.fingerprint() }.getOrDefault("") } else ui.fingerprint
            ui = ui.copy(
                pcPaired = lp.paired, pcName = lp.pcName, pc = StatusText.pc(this@MainActivity),
                handsFree = lp.enabled, lockWhenOff = wp.liveLog, phonePaired = wp.paired,
                listening = PcLinkService.listening, testUntil = wp.testUntil, fingerprint = fp,
                pairUntil = lp.pairUntil, lastEvent = wp.pcEvent, lastEventMs = wp.pcEventMs,
                sound = Feedback.soundOn(this@MainActivity), haptics = Feedback.intensity(this@MainActivity),
                pcAddress = lp.pcAddress ?: "", ble = WatchState.blePeripheralProbe(this@MainActivity),
                permissionsOk = missing().isEmpty(), expressive = wp.expressive, colour = wp.colour,
            )
        }
    }

    fun cue(c: Feedback.Cue) = Feedback.play(this, c)
}

@Composable
fun App(act: MainActivity, start: String) {
    val nav = rememberSwipeDismissableNavController()
    // Listener state (listening flag) lives in the service; reload when the screen changes.
    DisposableEffect(nav) {
        val l = androidx.navigation.NavController.OnDestinationChangedListener { _, _, _ -> act.reload() }
        nav.addOnDestinationChangedListener(l)
        onDispose { nav.removeOnDestinationChangedListener(l) }
    }
    AppScaffold {
        SwipeDismissableNavHost(navController = nav, startDestination = start) {
            composable("home") { HomeScreen(act, act.ui, onPair = { nav.navigate("pair") }, onSettings = { nav.navigate("settings") }, onRemote = { nav.navigate("remote") },
                onMusic = { nav.navigate("music") }, onSlides = { nav.navigate("slides") }, onMouse = { nav.navigate("mouse") }, onAir = { nav.navigate("air") }) }
            composable("pair") {
                PairScreen(act, act.ui, onDone = {
                    if (nav.previousBackStackEntry != null) nav.popBackStack()
                    else nav.navigate("home") { popUpTo("pair") { inclusive = true } }
                })
            }
            composable("settings") { SettingsScreen(act, act.ui, onPair = { nav.navigate("pair") }, onAbout = { nav.navigate("about") }) }
            composable("about") { AboutScreen(act.ui) }
            composable("remote") { RemoteScreen(act, act.ui, onMusic = { nav.navigate("music") }, onSlides = { nav.navigate("slides") }, onMouse = { nav.navigate("mouse") }) }
            composable("mouse") { MouseScreen(act) }
            composable("air") { AirControlScreen(act) }
            composable("music") { MusicScreen(act) }
            composable("slides") { SlidesScreen(act) }
        }
    }
}

