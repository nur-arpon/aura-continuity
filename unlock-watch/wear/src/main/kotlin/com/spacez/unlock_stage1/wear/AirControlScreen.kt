package com.spacez.unlock_stage1.wear

import android.content.Context
import android.hardware.Sensor
import android.hardware.SensorEvent
import android.hardware.SensorEventListener
import android.hardware.SensorManager
import android.os.SystemClock
import android.view.KeyEvent
import androidx.compose.foundation.background
import androidx.compose.foundation.focusable
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.pager.HorizontalPager
import androidx.compose.foundation.pager.rememberPagerState
import androidx.compose.foundation.shape.RoundedCornerShape
import kotlinx.coroutines.delay
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.input.key.nativeKeyCode
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.rotary.onRotaryScrollEvent
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.activity.compose.BackHandler
import androidx.wear.compose.foundation.lazy.TransformingLazyColumn
import androidx.wear.compose.foundation.lazy.rememberTransformingLazyColumnState
import androidx.wear.compose.material3.Button
import androidx.wear.compose.material3.ButtonDefaults
import androidx.wear.compose.material3.CompactButton
import androidx.wear.compose.material3.FilledIconButton
import androidx.wear.compose.material3.FilledTonalIconButton
import androidx.wear.compose.material3.Icon
import androidx.wear.compose.material3.ListHeader
import androidx.wear.compose.material3.MaterialTheme
import androidx.wear.compose.material3.ScreenScaffold
import androidx.wear.compose.material3.SurfaceTransformation
import androidx.wear.compose.material3.SwitchButton
import androidx.wear.compose.material3.Text
import androidx.wear.compose.material3.lazy.rememberTransformationSpec
import androidx.wear.compose.material3.lazy.transformedHeight
import com.spacez.aura.link.MediaCmd
import com.spacez.aura.link.PresentCmd
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull
import org.json.JSONObject
import kotlin.math.hypot

/** Saved Air control settings (per watch, not synced). */
object AirPrefs {
    private fun p(c: Context) = c.getSharedPreferences("cu_air", Context.MODE_PRIVATE)
    val SENS = floatArrayOf(0.5f, 0.75f, 1f, 1.5f, 2f, 3f)
    val FLICK = floatArrayOf(3.5f, 4.5f, 5f, 6.5f, 8f)
    val KNOCK = floatArrayOf(2f, 2.75f, 3.5f, 5f, 7f)
    fun enabled(c: Context): Set<Gesture> {
        val s = p(c).getString("on", null) ?: return AirMap.DEFAULT_ON
        return s.split(',').mapNotNull { n -> Gesture.entries.firstOrNull { it.name == n } }.toSet()
    }
    fun setEnabled(c: Context, g: Collection<Gesture>, on: Boolean) {
        val cur = enabled(c).toMutableSet(); if (on) cur += g else cur -= g.toSet()
        p(c).edit().putString("on", cur.joinToString(",") { it.name }).apply()
    }
    fun int(c: Context, k: String, d: Int) = p(c).getInt(k, d)
    fun setInt(c: Context, k: String, v: Int) = p(c).edit().putInt(k, v).apply()
    fun bool(c: Context, k: String, d: Boolean) = p(c).getBoolean(k, d)
    fun setBool(c: Context, k: String, v: Boolean) = p(c).edit().putBoolean(k, v).apply()
    fun mode(c: Context) = AirMode.entries.getOrElse(int(c, "mode", 0)) { AirMode.MOUSE }
    fun tuning(c: Context) = GestureTuning(flickRad = FLICK[int(c, "flick", 2).coerceIn(0, FLICK.size - 1)], knockMs2 = KNOCK[int(c, "knock", 2).coerceIn(0, KNOCK.size - 1)])
}

/** Registers the gyroscope + accelerometer (+ light) only between [start] and [stop]; the screen calls them with its lifecycle. */
class AirSensors(ctx: Context, private val onSample: (Sample) -> Unit, private val onLux: (Long, Float) -> Unit) : SensorEventListener {
    private val sm = ctx.getSystemService(SensorManager::class.java)
    private var ax = 0f; private var ay = 0f; private var az = 9.8f
    private var running = false
    val hasGyro get() = sm?.getDefaultSensor(Sensor.TYPE_GYROSCOPE) != null
    val hasLight get() = sm?.getDefaultSensor(Sensor.TYPE_LIGHT) != null
    fun start(light: Boolean) {
        if (running || sm == null) return
        running = true
        sm.getDefaultSensor(Sensor.TYPE_ACCELEROMETER)?.let { sm.registerListener(this, it, 10_000) }
        sm.getDefaultSensor(Sensor.TYPE_GYROSCOPE)?.let { sm.registerListener(this, it, 10_000) }
        if (light) sm.getDefaultSensor(Sensor.TYPE_LIGHT)?.let { sm.registerListener(this, it, SensorManager.SENSOR_DELAY_NORMAL) }
    }
    fun stop() { if (running) { sm?.unregisterListener(this); running = false } }
    override fun onAccuracyChanged(s: Sensor?, a: Int) {}
    override fun onSensorChanged(e: SensorEvent) {
        val t = e.timestamp / 1_000_000
        when (e.sensor.type) {
            Sensor.TYPE_ACCELEROMETER -> { ax = e.values[0]; ay = e.values[1]; az = e.values[2] }
            Sensor.TYPE_GYROSCOPE -> onSample(Sample(t, ax, ay, az, e.values[0], e.values[1], e.values[2]))
            Sensor.TYPE_LIGHT -> onLux(t, e.values[0])
        }
    }
}

private const val IDLE_OFF_MS = 120_000L

/**
 * Air control. Mouse mode: one big switch, "Air mouse: ON/OFF" (OFF every time the screen opens). While ON the cursor
 * follows the wrist continuously; tap = click, double-tap = double click, long-press = right click, bezel = scroll.
 * It switches itself OFF after 2 minutes unused or when the screen goes off. Slides / Media / Apps modes: wrist gestures
 * (see AirGestures.kt) + bezel. Every action buzzes and names itself in a small toast. Sensors only run while this
 * screen is in front; it keeps the screen on and the PC link warm. Cursor moves go out through WatchAura.input, the
 * same call the touchpad screen uses.
 */
@Composable
fun AirControlScreen(act: MainActivity) {
    KeepScreenOn()
    val ctx = LocalContext.current
    val scope = rememberCoroutineScope()
    val focus = remember { FocusRequester() }
    var mode by remember { mutableStateOf(AirPrefs.mode(ctx)) }
    var page by remember { mutableStateOf(if (AirPrefs.bool(ctx, "intro_seen", false)) "run" else "intro") }   // intro | run | settings | test
    var airOn by remember { mutableStateOf(false) }
    var toPhone by remember { mutableStateOf(InputTarget.phone(ctx)) }
    val phoneState by PhoneInput.state.collectAsState()
    var toast by remember { mutableStateOf("") }
    var toastN by remember { mutableStateOf(0) }
    val log = remember { mutableStateListOf<String>() }
    var liveText by remember { mutableStateOf("") }
    var cfgRev by remember { mutableStateOf(0) }   // bumped when settings change so the run page re-reads them

    val enabled = remember(cfgRev) { AirPrefs.enabled(ctx) }
    val swap = remember(cfgRev) { AirPrefs.bool(ctx, "swap", false) }
    val longRight = remember(cfgRev) { AirPrefs.bool(ctx, "longright", true) }
    val det = remember { GestureDetector() }
    det.tuning = remember(cfgRev) { AirPrefs.tuning(ctx) }
    val mouse = remember { AirMouse(idleMs = IDLE_OFF_MS) }
    mouse.filter.sensitivity = remember(cfgRev) { AirPrefs.SENS[AirPrefs.int(ctx, "sens", 2).coerceIn(0, AirPrefs.SENS.size - 1)] }
    mouse.filter.invertX = remember(cfgRev) { AirPrefs.bool(ctx, "invx", false) }
    mouse.filter.invertY = remember(cfgRev) { AirPrefs.bool(ctx, "invy", false) }
    val spinScroll = remember { RotarySpin() }
    val detent = remember { RotaryDetent(threshold = 40f, cooldownMs = 160) }
    val cover = remember { CoverDetector() }
    var suppressUntil by remember { mutableStateOf(0L) }

    fun input(o: JSONObject) = WatchAura.input(ctx, o.toString())
    fun say(t: String) { toast = t; toastN++ }
    fun setAir(v: Boolean) {
        if (v == airOn) return
        airOn = v; mouse.setOn(v, SystemClock.elapsedRealtime())
        if (v) scope.launch(Dispatchers.IO) { WatchAura.keepWarm(ctx) }
    }
    fun fire(a: Act, cue: Feedback.Cue = Feedback.Cue.SLIDE) {
        act.cue(cue); say(a.label); mouse.poke(SystemClock.elapsedRealtime())
        when (a) {
            is Act.Combo -> input(JSONObject().put("t", "combo").put("c", a.c))
            is Act.Click -> input(JSONObject().put("t", "click").put("b", if (a.right) "right" else "left"))
            is Act.DoubleClick -> repeat(2) { input(JSONObject().put("t", "click").put("b", "left")) }
            is Act.Media -> scope.launch { val e = withContext(Dispatchers.IO) { WatchAura.mediaCmd(ctx, a.cmd) }; if (e != null) { say(WatchRemoteText.short(e)); act.cue(Feedback.Cue.ERROR) } }
            is Act.Present -> scope.launch { val e = withContext(Dispatchers.IO) { WatchAura.present(ctx, a.cmd) }; if (e != null) { say(WatchRemoteText.short(e)); act.cue(Feedback.Cue.ERROR) } }
        }
    }

    fun addLog(g: Gesture, strength: Float?) {
        log.add(0, g.label + (if (strength != null) "  x%.1f".format(strength) else "") + (if (g.real) "" else "  (best effort)"))
        while (log.size > 5) log.removeAt(log.size - 1)
        act.cue(Feedback.Cue.TICK)
    }

    fun handle(g: Gesture, strength: Float?) {
        val now = SystemClock.elapsedRealtime()
        if (g != Gesture.PINCH && now < suppressUntil) return   // a touch causes its own jolts
        if (page == "test") { addLog(g, strength); return }
        if (page != "run" || mode == AirMode.MOUSE || g !in enabled) return   // Mouse mode: the wrist is the cursor, no gestures
        val a = AirMap.forGesture(mode, g, swap) ?: return
        fire(a)
    }

    fun onSample(s: Sample) {
        det.feed(s).forEach { handle(it.g, it.strength) }
        if (page == "test") { liveText = "gyro %.1f  hit %.1f".format(det.live.gyro, det.live.hp) }
        if (mode == AirMode.MOUSE && page == "run") {
            mouse.feed(s.t, det.live.yaw, s.gx, SystemClock.elapsedRealtime())?.let { (x, y) -> input(JSONObject().put("t", "move").put("dx", x).put("dy", y)) }
        }
    }

    // sensors: only while the run / test page is in front
    val wantLight = page == "test" || Gesture.COVER in enabled
    val sensing = page == "run" || page == "test"
    DisposableEffect(sensing, wantLight) {
        val sensors = AirSensors(ctx, { onSample(it) }, { t, lux ->
            if (cover.feed(t, lux)) handle(Gesture.COVER, null)
        })
        val obs = LifecycleEventObserver { _, ev ->
            if (ev == Lifecycle.Event.ON_RESUME && sensing) sensors.start(wantLight)
            else if (ev == Lifecycle.Event.ON_PAUSE) { sensors.stop(); setAir(false) }   // ambient / screen off: sensors and Air mouse off
        }
        if (sensing && act.lifecycle.currentState.isAtLeast(Lifecycle.State.RESUMED)) sensors.start(wantLight)
        act.lifecycle.addObserver(obs)
        onDispose { act.lifecycle.removeObserver(obs); sensors.stop() }
    }
    LaunchedEffect(page) { if (page != "run") setAir(false); focus.requestFocus() }
    LaunchedEffect(Unit) { while (true) { delay(5_000); if (mouse.idleExpired(SystemClock.elapsedRealtime())) { setAir(false); say("Air mouse off (idle)"); act.cue(Feedback.Cue.TOGGLE_OFF) } } }
    LaunchedEffect(toastN) { if (toastN > 0) { delay(1_600); toast = "" } }
    BackHandler(enabled = page != "run") { page = if (page == "test") "settings" else "run"; cfgRev++ }

    fun cycleMode() {
        mode = AirMode.entries[(mode.ordinal + 1) % AirMode.entries.size]; AirPrefs.setInt(ctx, "mode", mode.ordinal)
        setAir(false); toast = ""; act.cue(Feedback.Cue.TAP)
    }

    ScreenScaffold { _ ->
        Box(
            Modifier.fillMaxSize()
                .pointerInput(Unit) { awaitEachGesture { while (true) { awaitPointerEventFromInitial(); suppressUntil = SystemClock.elapsedRealtime() + 450 } } }
                .onPreviewKeyEvent { e ->
                    val code = e.key.nativeKeyCode
                    if (e.type != KeyEventType.KeyDown) return@onPreviewKeyEvent false
                    // a double pinch / stem button may arrive as one of these keys (best effort: Samsung may keep it for itself)
                    when (code) {
                        KeyEvent.KEYCODE_STEM_PRIMARY, KeyEvent.KEYCODE_STEM_1, KeyEvent.KEYCODE_STEM_2, KeyEvent.KEYCODE_STEM_3,
                        KeyEvent.KEYCODE_MEDIA_PLAY_PAUSE, KeyEvent.KEYCODE_MEDIA_PLAY, KeyEvent.KEYCODE_MEDIA_PAUSE, KeyEvent.KEYCODE_HEADSETHOOK -> {
                            if (page == "test") log.add(0, "Key $code (pinch?)").also { while (log.size > 5) log.removeAt(log.size - 1) }
                            handle(Gesture.PINCH, null); true
                        }
                        KeyEvent.KEYCODE_BACK, KeyEvent.KEYCODE_HOME -> false
                        else -> { if (page == "test") { log.add(0, "Key $code"); while (log.size > 5) log.removeAt(log.size - 1) }; false }
                    }
                }
                .onRotaryScrollEvent { e ->
                    val now = SystemClock.elapsedRealtime()
                    val px = e.verticalScrollPixels
                    if (page == "test") { val n = detent.feed(px, now); if (n != 0) { log.add(0, "Bezel " + (if (n > 0) "clockwise" else "back")); while (log.size > 5) log.removeAt(log.size - 1); act.cue(Feedback.Cue.DETENT) }; return@onRotaryScrollEvent true }
                    if (page != "run") return@onRotaryScrollEvent false
                    mouse.poke(now)
                    when (mode) {
                        AirMode.MOUSE -> { val dy = spinScroll.feed(px, now); if (dy != 0) { input(JSONObject().put("t", "scroll").put("dy", dy)); if (spinScroll.fast) say("Page scroll") } }
                        AirMode.SLIDES -> detent.feed(px, now).let { if (it != 0) fire(Act.Present(if (it > 0) PresentCmd.NEXT else PresentCmd.PREV, if (it > 0) "Next slide" else "Previous slide")) }
                        AirMode.MEDIA -> detent.feed(px, now).let { if (it != 0) fire(Act.Media(if (it > 0) MediaCmd.VOL_UP else MediaCmd.VOL_DOWN, if (it > 0) "Volume up" else "Volume down"), Feedback.Cue.DETENT) }
                        AirMode.WINDOWS -> detent.feed(px, now).let { if (it != 0) fire(if (it > 0) AirMap.ALT_TAB else AirMap.ALT_SHIFT_TAB, Feedback.Cue.DETENT) }
                    }
                    true
                }
                .focusRequester(focus).focusable(),
        ) {
            when (page) {
                "intro" -> AirIntro(onDone = { AirPrefs.setBool(ctx, "intro_seen", true); act.cue(Feedback.Cue.TAP); page = "run" })
                "settings" -> AirSettingsPage(act, onTest = { page = "test" }, onChanged = { cfgRev++ })
                "test" -> Column(Modifier.fillMaxSize().padding(horizontal = 22.dp, vertical = 20.dp), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.Center) {
                    Text("Gesture test", style = MaterialTheme.typography.labelMedium, color = K.primary)
                    Text(log.firstOrNull() ?: "Try a gesture", style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold, textAlign = TextAlign.Center, maxLines = 2)
                    Text(liveText, style = MaterialTheme.typography.labelSmall, color = K.text2)
                    log.drop(1).take(3).forEach { Text(it, style = MaterialTheme.typography.bodySmall, color = K.text2, maxLines = 1, overflow = TextOverflow.Ellipsis) }
                    Text("Nothing is sent to the PC", style = MaterialTheme.typography.labelSmall, color = K.accent)
                }
                else -> {
                    Column(
                        Modifier.fillMaxSize()
                            .pointerInput(mode, longRight) {
                                if (mode != AirMode.MOUSE) return@pointerInput
                                detectTapGestures(
                                    onPress = { mouse.touch(true, SystemClock.elapsedRealtime()); tryAwaitRelease(); mouse.touch(false, SystemClock.elapsedRealtime()) },
                                    onTap = { if (airOn) fire(Act.Click(false), Feedback.Cue.TAP) },
                                    onDoubleTap = { if (airOn) fire(Act.DoubleClick, Feedback.Cue.TAP) },
                                    onLongPress = { if (airOn && longRight) fire(Act.Click(true), Feedback.Cue.SEND) },
                                )
                            }
                            .padding(horizontal = 18.dp).padding(top = 14.dp, bottom = 12.dp),
                        horizontalAlignment = Alignment.CenterHorizontally,
                    ) {
                        Row(horizontalArrangement = Arrangement.spacedBy(4.dp), verticalAlignment = Alignment.CenterVertically) {
                            CompactButton(onClick = { cycleMode() }) { Text(mode.label, maxLines = 1) }
                            CompactButton(onClick = { act.cue(Feedback.Cue.TAP); page = "intro" }, colors = ButtonDefaults.filledTonalButtonColors()) { Text("?") }
                            CompactButton(onClick = { act.cue(Feedback.Cue.TAP); page = "settings" }, colors = ButtonDefaults.filledTonalButtonColors()) { Icon(painterResource(R.drawable.ic_settings), "Settings") }
                        }
                        Box(Modifier.weight(1f).fillMaxWidth(), contentAlignment = Alignment.Center) {
                            when (mode) {
                                AirMode.MOUSE -> Column(horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(4.dp)) {
                                    Button(
                                        onClick = { setAir(!airOn); act.cue(if (airOn) Feedback.Cue.TOGGLE_ON else Feedback.Cue.TOGGLE_OFF); say(if (airOn) "Air mouse ON" else "Air mouse OFF") },
                                        modifier = Modifier.fillMaxWidth().height(52.dp),
                                        colors = if (airOn) ButtonDefaults.buttonColors() else ButtonDefaults.filledTonalButtonColors(),
                                    ) { Text("Air mouse: " + if (airOn) "ON" else "OFF", maxLines = 1, fontWeight = FontWeight.SemiBold) }
                                    CompactButton(onClick = { toPhone = !toPhone; InputTarget.set(ctx, toPhone); act.cue(Feedback.Cue.TAP); say("Controls the " + InputTarget.label(toPhone).lowercase()) }, colors = ButtonDefaults.filledTonalButtonColors()) {
                                        Text(if (toPhone && phoneState == "a11y_off") "Turn on in Aura on phone" else "Control: " + InputTarget.label(toPhone), maxLines = 1)
                                    }
                                }
                                AirMode.SLIDES -> Row(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalAlignment = Alignment.CenterVertically) {
                                    FilledTonalIconButton(onClick = { fire(Act.Present(PresentCmd.PREV, "Previous slide")) }, modifier = Modifier.size(52.dp)) { Icon(painterResource(R.drawable.ic_prev), "Previous slide") }
                                    FilledIconButton(onClick = { fire(Act.Present(PresentCmd.NEXT, "Next slide")) }, modifier = Modifier.size(62.dp)) { Icon(painterResource(R.drawable.ic_next), "Next slide") }
                                }
                                AirMode.MEDIA -> Row(horizontalArrangement = Arrangement.spacedBy(5.dp), verticalAlignment = Alignment.CenterVertically) {
                                    FilledTonalIconButton(onClick = { fire(Act.Media(MediaCmd.PREV, "Previous track")) }, modifier = Modifier.size(46.dp)) { Icon(painterResource(R.drawable.ic_prev), "Previous") }
                                    FilledIconButton(onClick = { fire(Act.Media(MediaCmd.PLAY_PAUSE, "Play / pause")) }, modifier = Modifier.size(58.dp)) { Icon(painterResource(R.drawable.ic_play), "Play or pause") }
                                    FilledTonalIconButton(onClick = { fire(Act.Media(MediaCmd.NEXT, "Next track")) }, modifier = Modifier.size(46.dp)) { Icon(painterResource(R.drawable.ic_next), "Next") }
                                }
                                AirMode.WINDOWS -> Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                                    Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                                        Button(onClick = { fire(AirMap.ALT_TAB) }, modifier = Modifier.weight(1f).height(40.dp), colors = ButtonDefaults.filledTonalButtonColors()) { Text("Switch", maxLines = 1) }
                                        Button(onClick = { fire(AirMap.WIN_TAB) }, modifier = Modifier.weight(1f).height(40.dp), colors = ButtonDefaults.filledTonalButtonColors()) { Text("All", maxLines = 1) }
                                    }
                                    Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                                        Button(onClick = { fire(AirMap.WIN_D) }, modifier = Modifier.weight(1f).height(40.dp), colors = ButtonDefaults.filledTonalButtonColors()) { Text("Desktop", maxLines = 1) }
                                        Button(onClick = { fire(AirMap.WIN_DOWN) }, modifier = Modifier.weight(1f).height(40.dp), colors = ButtonDefaults.filledTonalButtonColors()) { Text("Minimise", maxLines = 1) }
                                    }
                                }
                            }
                        }
                        Text(mode.legend, style = MaterialTheme.typography.labelSmall, color = K.text2, textAlign = TextAlign.Center, maxLines = 3, overflow = TextOverflow.Ellipsis)
                    }
                    if (toast.isNotEmpty()) {
                        Box(Modifier.align(Alignment.TopCenter).padding(top = 12.dp).clip(RoundedCornerShape(50)).background(K.primary).padding(horizontal = 14.dp, vertical = 6.dp)) {
                            Text(toast, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onPrimary, maxLines = 1)
                        }
                    }
                }
            }
        }
    }
}

/** First-open guide (3 swipeable cards); the "?" button on the run page shows it again. */
@Composable
private fun AirIntro(onDone: () -> Unit) {
    val pager = rememberPagerState { 3 }
    val titles = listOf("Air control", "Move the cursor", "Wrist gestures")
    val bodies = listOf(
        "Use your watch as a mouse, slide clicker and media remote for your laptop. Swipe to learn how.",
        "Pick Mouse and switch Air mouse ON, then tilt your wrist: the cursor follows. Tap = click, hold = right click, bezel = scroll.",
        "In Slides, Media and Apps a quick flick, twist, shake or double-knock does something. Each one buzzes and names itself.",
    )
    Box(Modifier.fillMaxSize()) {
        HorizontalPager(pager, Modifier.fillMaxSize()) { i ->
            Column(Modifier.fillMaxSize().padding(horizontal = 30.dp).padding(top = 26.dp, bottom = 18.dp), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.Center) {
                Text(titles[i], style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold, color = K.primary, textAlign = TextAlign.Center)
                Text(bodies[i], style = MaterialTheme.typography.bodySmall, color = K.text, textAlign = TextAlign.Center, modifier = Modifier.padding(top = 4.dp))
                if (i == 2) Button(onClick = onDone, modifier = Modifier.fillMaxWidth().padding(top = 8.dp).height(44.dp)) { Text("Got it") }
                else Text("Swipe →", style = MaterialTheme.typography.labelSmall, color = K.text2, modifier = Modifier.padding(top = 6.dp))
            }
        }
        Row(Modifier.align(Alignment.TopCenter).padding(top = 12.dp), horizontalArrangement = Arrangement.spacedBy(5.dp)) {
            repeat(3) { Box(Modifier.size(6.dp).clip(CircleShape).background(if (pager.currentPage == it) K.primary else K.line)) }
        }
    }
}

private suspend fun androidx.compose.ui.input.pointer.AwaitPointerEventScope.awaitPointerEventFromInitial() { awaitPointerEvent(PointerEventPass.Initial) }

@Composable
private fun AirSettingsPage(act: MainActivity, onTest: () -> Unit, onChanged: () -> Unit) {
    val list = rememberTransformingLazyColumnState()
    val spec = rememberTransformationSpec()
    var rev by remember { mutableStateOf(0) }
    val on = remember(rev) { AirPrefs.enabled(act) }
    fun bump() { rev++; onChanged() }
    @Composable
    fun sw(label: String, sec: String, gs: List<Gesture>) = SwitchButton(
        checked = gs.all { it in on },
        onCheckedChange = { AirPrefs.setEnabled(act, gs, it); act.cue(if (it) Feedback.Cue.TOGGLE_ON else Feedback.Cue.TOGGLE_OFF); bump() },
        modifier = Modifier.fillMaxWidth(),
        label = { Text(label) }, secondaryLabel = { Text(sec, maxLines = 2) },
    )
    @Composable
    fun flag(label: String, key: String, def: Boolean) = SwitchButton(
        checked = remember(rev) { AirPrefs.bool(act, key, def) },
        onCheckedChange = { AirPrefs.setBool(act, key, it); act.cue(if (it) Feedback.Cue.TOGGLE_ON else Feedback.Cue.TOGGLE_OFF); bump() },
        modifier = Modifier.fillMaxWidth(), label = { Text(label) },
    )
    @Composable
    fun cycle(label: String, key: String, def: Int, n: Int, names: List<String>) {
        val v = remember(rev) { AirPrefs.int(act, key, def).coerceIn(0, n - 1) }
        Button(onClick = { AirPrefs.setInt(act, key, (v + 1) % n); act.cue(Feedback.Cue.TAP); bump() }, modifier = Modifier.fillMaxWidth(),
            colors = ButtonDefaults.filledTonalButtonColors(), label = { Text(label) }, secondaryLabel = { Text(names[v] + "  (tap to change)") })
    }
    ScreenScaffold(scrollState = list) { padding ->
        TransformingLazyColumn(state = list, contentPadding = padding) {
            item { ListHeader(Modifier.fillMaxWidth().transformedHeight(this, spec), transformation = SurfaceTransformation(spec)) { Text("Air control") } }
            item { Button(onClick = { act.cue(Feedback.Cue.TAP); onTest() }, modifier = Modifier.fillMaxWidth(), label = { Text("Gesture test") }, secondaryLabel = { Text("See what is detected, no PC action") }) }
            item { sw("Flick", "Hand left / right: next / back", listOf(Gesture.FLICK_LEFT, Gesture.FLICK_RIGHT)) }
            item { sw("Twist", "Forearm out / in: Alt+Tab", listOf(Gesture.TWIST_OUT, Gesture.TWIST_IN)) }
            item { sw("Shake", "Show desktop", listOf(Gesture.SHAKE)) }
            item { sw("Double knock", "On the table or watch: main action", listOf(Gesture.DOUBLE_KNOCK)) }
            item { sw("Double pinch", "If the watch passes it on (best effort)", listOf(Gesture.PINCH)) }
            item { sw("Palm cover", "Cover the screen: mute / pause (best effort)", listOf(Gesture.COVER)) }
            item { sw("Fist clench", "Experimental, may misfire", listOf(Gesture.CLENCH)) }
            item { cycle("Air mouse speed", "sens", 2, AirPrefs.SENS.size, AirPrefs.SENS.map { "x$it" }) }
            item { cycle("Flick strength", "flick", 2, AirPrefs.FLICK.size, listOf("Very light", "Light", "Normal", "Firm", "Hard")) }
            item { cycle("Knock strength", "knock", 2, AirPrefs.KNOCK.size, listOf("Very light", "Light", "Normal", "Firm", "Hard")) }
            item { flag("Swap directions", "swap", false) }
            item { flag("Long-press = right click", "longright", true) }
            item { flag("Invert mouse X", "invx", false) }
            item { flag("Invert mouse Y", "invy", false) }
        }
    }
}
