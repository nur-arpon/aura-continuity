package com.spacez.unlock_stage1.wear

import android.graphics.BitmapFactory
import androidx.compose.foundation.Image
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.wear.compose.material3.ButtonGroup
import androidx.wear.compose.material3.FilledIconButton
import androidx.wear.compose.material3.FilledTonalIconButton
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.focusable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
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
import androidx.compose.runtime.produceState
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.input.rotary.onRotaryScrollEvent
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.wear.compose.foundation.lazy.TransformingLazyColumn
import androidx.wear.compose.foundation.lazy.rememberTransformingLazyColumnState
import androidx.wear.compose.material3.AlertDialog
import androidx.wear.compose.material3.AlertDialogDefaults
import androidx.wear.compose.material3.Button
import androidx.wear.compose.material3.ButtonDefaults
import androidx.wear.compose.material3.FailureConfirmationDialog
import androidx.wear.compose.material3.Icon
import androidx.wear.compose.material3.IconButton
import androidx.wear.compose.material3.IconButtonDefaults
import androidx.wear.compose.material3.ListHeader
import androidx.wear.compose.material3.MaterialTheme
import androidx.wear.compose.material3.ScreenScaffold
import androidx.wear.compose.material3.SuccessConfirmationDialog
import androidx.wear.compose.material3.Text
import androidx.wear.compose.material3.confirmationDialogCurvedText
import androidx.wear.compose.material3.lazy.rememberTransformationSpec
import androidx.wear.compose.material3.lazy.transformedHeight
import androidx.wear.compose.material3.SurfaceTransformation
import com.spacez.aura.link.MediaCmd
import com.spacez.aura.link.PresentCmd
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** Turns a stream of bezel / crown scroll pixels into whole steps (+1 next, -1 back). Pure and unit-tested. */
class RotaryStepper(private val threshold: Float = 48f) {
    private var acc = 0f
    /** @return the number of steps this movement completed; positive = clockwise. */
    fun feed(px: Float): Int {
        acc += px
        var steps = 0
        while (acc >= threshold) { acc -= threshold; steps++ }
        while (acc <= -threshold) { acc += threshold; steps-- }
        return steps
    }
    fun reset() { acc = 0f }
}

/**
 * One command per rotary gesture: after a detent fires, everything arriving in the next [cooldownMs] is dropped, so a
 * fast spin can never flood the PC (the old stepper sent ~130 commands in 7 s). Returns +1 / -1 / 0. Pure and unit-tested.
 */
class RotaryDetent(private val threshold: Float = 48f, private val cooldownMs: Long = 250) {
    private var acc = 0f
    private var lastFire = Long.MIN_VALUE / 2
    fun feed(px: Float, nowMs: Long): Int {
        if (nowMs - lastFire < cooldownMs) { acc = 0f; return 0 }
        acc += px
        if (kotlin.math.abs(acc) < threshold) return 0
        val dir = if (acc > 0) 1 else -1
        acc = 0f; lastFire = nowMs
        return dir
    }
}

/** Keeps the display on while a live-control screen (remote, music, slides, mouse) is showing. */
@Composable
fun KeepScreenOn() {
    val view = androidx.compose.ui.platform.LocalView.current
    DisposableEffect(view) {
        view.keepScreenOn = true
        onDispose { view.keepScreenOn = false }
    }
    // keep the PC link up while a remote screen is showing (the link idles out after 30 s)
    val ctx = androidx.compose.ui.platform.LocalContext.current
    LaunchedEffect(Unit) {
        while (true) {
            kotlinx.coroutines.withContext(kotlinx.coroutines.Dispatchers.IO) { WatchAura.keepWarm(ctx) }
            kotlinx.coroutines.delay(15_000)
        }
    }
}

// =============================================================================== Remote menu

@Composable
fun RemoteScreen(act: MainActivity, ui: UiState, onMusic: () -> Unit, onSlides: () -> Unit, onMouse: () -> Unit = {}) {
    KeepScreenOn()
    val list = rememberTransformingLazyColumnState()
    val spec = rememberTransformationSpec()
    val scope = rememberCoroutineScope()
    var ringing by remember { mutableStateOf(false) }
    var result by remember { mutableStateOf<String?>(null) }   // null = nothing, "" = ok, else the reason
    var ringOn by remember { mutableStateOf(false) }
    var waking by remember { mutableStateOf(false) }
    var confirmRestart by remember { mutableStateOf(false) }
    var done by remember { mutableStateOf("") }   // text of the success dialog

    fun wake() {
        if (waking) return
        act.cue(Feedback.Cue.SEND); waking = true
        scope.launch {
            val err = withContext(Dispatchers.IO) { WatchAura.wake(act) }
            waking = false
            if (err == null) { done = "Waking PC"; result = ""; act.cue(Feedback.Cue.APPROVED) } else { result = err; act.cue(Feedback.Cue.ERROR) }
        }
    }
    fun restart() {
        act.cue(Feedback.Cue.SEND)
        scope.launch {
            val err = withContext(Dispatchers.IO) { WatchAura.power(act, "restart") }
            if (err == null) { done = "Restarting PC"; result = ""; act.cue(Feedback.Cue.APPROVED) } else { result = err; act.cue(Feedback.Cue.ERROR) }
        }
    }
    val media by WatchAura.media.collectAsState()

    fun ring() {
        if (ringing) return
        done = ""
        act.cue(Feedback.Cue.SEND)
        ringing = true
        val want = !ringOn
        scope.launch {
            val err = withContext(Dispatchers.IO) { WatchAura.ring(act, want) }
            ringing = false
            if (err == null) { ringOn = want; result = ""; act.cue(Feedback.Cue.APPROVED) } else { result = err; act.cue(Feedback.Cue.ERROR) }
        }
    }

    ScreenScaffold(scrollState = list) { padding ->
        TransformingLazyColumn(state = list, contentPadding = padding) {
            item { ListHeader(Modifier.fillMaxWidth().transformedHeight(this, spec), transformation = SurfaceTransformation(spec)) { Text("Remote") } }
            item {
                Button(
                    onClick = { ring() },
                    modifier = Modifier.fillMaxWidth().height(60.dp).transformedHeight(this, spec), transformation = SurfaceTransformation(spec),
                    colors = if (ringOn) ButtonDefaults.filledTonalButtonColors() else ButtonDefaults.buttonColors(),
                    icon = { Icon(painterResource(R.drawable.ic_bell), null, modifier = Modifier.size(24.dp)) },
                    label = { Text(if (ringing) "Asking…" else if (ringOn) "Stop ringing" else "Ring laptop", style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold) },
                    secondaryLabel = { Text(if (ringOn) "Dismiss it on the PC" else "Loud chime + alert") },
                )
            }
            item {
                Button(
                    onClick = { wake() },
                    modifier = Modifier.fillMaxWidth().transformedHeight(this, spec), transformation = SurfaceTransformation(spec),
                    colors = ButtonDefaults.filledTonalButtonColors(),
                    icon = { Icon(painterResource(R.drawable.ic_laptop), null) },
                    label = { Text(if (waking) "Sending…" else "Wake PC") },
                    secondaryLabel = { Text("Turn it on from here") },
                )
            }
            item {
                Button(
                    onClick = { act.cue(Feedback.Cue.TAP); confirmRestart = true },
                    modifier = Modifier.fillMaxWidth().transformedHeight(this, spec), transformation = SurfaceTransformation(spec),
                    colors = ButtonDefaults.filledTonalButtonColors(),
                    icon = { Icon(painterResource(R.drawable.ic_warning), null) },
                    label = { Text("Restart PC") },
                    secondaryLabel = { Text("Asks first") },
                )
            }
            item {
                Button(
                    onClick = { act.cue(Feedback.Cue.TAP); onMusic() },
                    modifier = Modifier.fillMaxWidth().transformedHeight(this, spec), transformation = SurfaceTransformation(spec),
                    colors = ButtonDefaults.filledTonalButtonColors(),
                    icon = { Icon(painterResource(R.drawable.ic_play), null) },
                    label = { Text("Music") },
                    secondaryLabel = { Text(media.state?.title?.takeIf { it.isNotEmpty() } ?: "Play, volume", maxLines = 1, overflow = TextOverflow.Ellipsis) },
                )
            }
            item {
                Button(
                    onClick = { act.cue(Feedback.Cue.TAP); onSlides() },
                    modifier = Modifier.fillMaxWidth().transformedHeight(this, spec), transformation = SurfaceTransformation(spec),
                    colors = ButtonDefaults.filledTonalButtonColors(),
                    icon = { Icon(painterResource(R.drawable.ic_slides), null) },
                    label = { Text("Slides") },
                    secondaryLabel = { Text("Bezel = next / back") },
                )
            }
            item {
                Button(
                    onClick = { act.cue(Feedback.Cue.TAP); onMouse() },
                    modifier = Modifier.fillMaxWidth().transformedHeight(this, spec), transformation = SurfaceTransformation(spec),
                    colors = ButtonDefaults.filledTonalButtonColors(),
                    icon = { Icon(painterResource(R.drawable.ic_laptop), null) },
                    label = { Text("Mouse") },
                    secondaryLabel = { Text("Touchpad + keyboard") },
                )
            }
        }
    }
    val cts = androidx.wear.compose.material3.ConfirmationDialogDefaults.curvedTextStyle
    SuccessConfirmationDialog(visible = result == "", onDismissRequest = { result = null },
        curvedText = { confirmationDialogCurvedText(done.ifEmpty { if (ringOn) "Laptop ringing" else "Stopped" }, cts) })
    AlertDialog(
        visible = confirmRestart, onDismissRequest = { confirmRestart = false },
        icon = { Icon(painterResource(R.drawable.ic_warning), null) },
        title = { Text("Restart ${ui.pcName.ifBlank { "the PC" }}?") },
        text = { Text("Unsaved work on the PC is lost.") },
        confirmButton = { AlertDialogDefaults.ConfirmButton(onClick = { confirmRestart = false; restart() }) },
        dismissButton = { AlertDialogDefaults.DismissButton(onClick = { confirmRestart = false }) },
    )
    FailureConfirmationDialog(visible = !result.isNullOrEmpty(), onDismissRequest = { result = null },
        curvedText = { confirmationDialogCurvedText(WatchRemoteText.short(result ?: ""), cts) })
}

/** Plain short words for the failure dialog (round screen, one line). */
object WatchRemoteText {
    fun short(reason: String) = when {
        reason.contains("Pair", true) -> "Pair first"
        reason.contains("switched off", true) -> "Off on the PC"
        reason.contains("locked", true) -> "PC is locked"
        else -> "PC out of reach"
    }
}

// =============================================================================== Music

@Composable
fun MusicScreen(act: MainActivity) {
    KeepScreenOn()
    val scope = rememberCoroutineScope()
    val media by WatchAura.media.collectAsState()
    var err by remember { mutableStateOf<String?>(null) }
    val focus = remember { FocusRequester() }
    val detent = remember { RotaryDetent() }

    // ask the PC to keep us posted while this screen is open; re-asked every 20 s so the link never idles out
    // (idle sessions close after 30 s, and the PC's push stops with them)
    val pcName = remember { PcLinkPrefs(act).pcName.ifBlank { "your PC" } }
    var waited by remember { mutableStateOf(false) }   // asked at least once and still no answer
    LaunchedEffect(Unit) {
        while (true) {
            val e = withContext(Dispatchers.IO) { WatchAura.mediaCmd(act, MediaCmd.REFRESH) }
            if (e != null) android.util.Log.w("CUWatch", "music: refresh failed: $e")
            // a real refusal (switched off, locked) is worth showing; a timeout is not an error by itself
            if (e != null && !e.contains("PC not reachable")) err = e else if (e == null && err != null && media.at > 0) err = null
            withContext(Dispatchers.IO) { WatchAura.askPhoneList(act) }
            waited = true
            kotlinx.coroutines.delay(20_000)
        }
    }
    DisposableEffect(Unit) { onDispose { Thread { WatchAura.mediaCmd(act, MediaCmd.STOP_UPDATES) }.start() } }
    LaunchedEffect(Unit) { focus.requestFocus() }
    // Phone | Laptop switcher: null = automatic (whichever is playing)
    val phoneM by WatchAura.phone.collectAsState()
    var picked by remember { mutableStateOf<String?>(null) }
    val phoneS = phoneM.sessions.firstOrNull { it.playing } ?: phoneM.sessions.firstOrNull()
    val laptopTitle = media.state?.title?.takeIf { it.isNotEmpty() }
    val onPhone = when (picked) {
        "phone" -> true
        "laptop" -> false
        else -> if (media.state?.playing == true) false else if (phoneS?.playing == true) true else if (laptopTitle != null) false else phoneS != null
    }
    // what the button will show right after a tap, until the PC's real state arrives
    var guess by remember { mutableStateOf<Boolean?>(null) }
    LaunchedEffect(media.at, phoneM.at, onPhone) { guess = null }

    fun cmd(c: Int, cue: Boolean = true) {
        if (cue) act.cue(Feedback.Cue.TAP)
        if (c == MediaCmd.PLAY_PAUSE) guess = !(guess ?: (if (onPhone) phoneS?.playing else media.state?.playing) ?: false)
        scope.launch {
            val e = withContext(Dispatchers.IO) {
                val ps = phoneS
                if (onPhone) {
                    if (ps == null) "No phone session"
                    else when (c) {
                        MediaCmd.PLAY_PAUSE -> WatchAura.phoneCmd(act, ps.id, "toggle")
                        MediaCmd.NEXT -> WatchAura.phoneCmd(act, ps.id, "next")
                        MediaCmd.PREV -> WatchAura.phoneCmd(act, ps.id, "prev")
                        else -> null
                    }
                } else WatchAura.mediaCmd(act, c)
            }
            err = e   // a later success clears an earlier error
            if (e != null) { act.cue(Feedback.Cue.ERROR); guess = null }
        }
    }

    val st = media.state
    val heard = media.at > 0 || phoneM.at > 0
    val title = if (onPhone) phoneS?.title?.takeIf { it.isNotEmpty() } else st?.title?.takeIf { it.isNotEmpty() }
    val subtitle = if (onPhone) (phoneS?.artist?.takeIf { it.isNotEmpty() } ?: phoneS?.app) else st?.artist?.takeIf { it.isNotEmpty() }
    val playing = guess ?: (if (onPhone) phoneS?.playing == true else st?.playing == true)
    val artBmp = remember(media.art, onPhone) { if (onPhone) null else media.art?.let { runCatching { BitmapFactory.decodeByteArray(it, 0, it.size) }.getOrNull() } }
    val ringTrack = K.line; val ringFill = K.primary; val volFill = K.accent; val arcMuted = K.outline
    // progress: the PC pushes the position now and then; between pushes it is advanced here once a second while playing
    val tick by produceState(System.currentTimeMillis()) { while (true) { kotlinx.coroutines.delay(1000); value = System.currentTimeMillis() } }
    val dur = if (onPhone) phoneS?.durMs ?: 0L else st?.durationMs ?: 0L
    val basePos = if (onPhone) phoneS?.posMs ?: 0L else st?.positionMs ?: 0L
    val baseAt = if (onPhone) phoneM.at else media.at
    val posMs = if (!onPhone && st == null) 0L else (basePos + if (playing && baseAt > 0) (tick - baseAt).coerceAtLeast(0) else 0L).coerceIn(0L, if (dur > 0) dur else Long.MAX_VALUE)
    val progress = if (dur > 0) (posMs.toFloat() / dur).coerceIn(0f, 1f) else 0f
    fun mmss(ms: Long): String { val s = (ms / 1000).toInt(); return "%d:%02d".format(s / 60, s % 60) }
    ScreenScaffold { _ ->
        Box(
            Modifier.fillMaxSize()
                .onRotaryScrollEvent { e ->
                    val n = if (onPhone) 0 else detent.feed(e.verticalScrollPixels, System.currentTimeMillis())
                    if (n != 0) {
                        act.cue(Feedback.Cue.DETENT)   // one crisp click per volume step
                        cmd(if (n > 0) MediaCmd.VOL_UP else MediaCmd.VOL_DOWN, cue = false)
                    }
                    n != 0
                }
                .focusRequester(focus).focusable(),
        ) {
            val vol = if (onPhone) null else st?.volume
            Canvas(Modifier.fillMaxSize()) {
                // progress ring: the whole rim, clockwise from the top
                val inset = 4.dp.toPx()
                val tl = Offset(inset, inset)
                val sz = androidx.compose.ui.geometry.Size(size.width - 2 * inset, size.height - 2 * inset)
                drawArc(ringTrack, -90f, 360f, false, tl, sz, style = Stroke(4.dp.toPx(), cap = StrokeCap.Round))
                if (progress > 0f) drawArc(ringFill, -90f, 360f * progress, false, tl, sz, style = Stroke(4.dp.toPx(), cap = StrokeCap.Round))
                // volume: a slim arc inside the ring on the right edge (the crown / bezel moves it)
                if (vol != null && st != null) {
                    val vi = 13.dp.toPx()
                    val vtl = Offset(vi, vi)
                    val vsz = androidx.compose.ui.geometry.Size(size.width - 2 * vi, size.height - 2 * vi)
                    val sweep = 64f
                    drawArc(ringTrack, -sweep / 2, sweep, false, vtl, vsz, style = Stroke(3.dp.toPx(), cap = StrokeCap.Round))
                    val f = (vol.coerceIn(0, 100)) / 100f
                    if (f > 0f) drawArc(if (st.muted) arcMuted else volFill, sweep / 2 - sweep * f, sweep * f, false, vtl, vsz, style = Stroke(3.dp.toPx(), cap = StrokeCap.Round))
                }
            }
            Column(
                Modifier.fillMaxSize().padding(start = 22.dp, end = 22.dp, top = 30.dp, bottom = 24.dp),
                horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.SpaceBetween,
            ) {
                Column(horizontalAlignment = Alignment.CenterHorizontally) {
                    Row(horizontalArrangement = Arrangement.spacedBy(4.dp), verticalAlignment = Alignment.CenterVertically) {
                        for ((label, isPhone) in listOf("Phone" to true, "Laptop" to false)) {
                            val on = onPhone == isPhone
                            Text(label, Modifier.clip(CircleShape).background(if (on) K.primary else K.line).clickable { act.cue(Feedback.Cue.TAP); picked = if (isPhone) "phone" else "laptop" }
                                .padding(horizontal = 9.dp, vertical = 2.dp),
                                style = MaterialTheme.typography.labelSmall, color = if (on) K.bg else K.text2)
                        }
                    }
                    if (artBmp != null) Image(artBmp.asImageBitmap(), null, Modifier.padding(top = 2.dp).size(26.dp).clip(CircleShape), contentScale = ContentScale.Crop)
                    Text(title ?: if (!heard) (if (waited) "No answer from $pcName" else "Connecting...") else if (onPhone) "Nothing playing on the phone" else "Nothing playing",
                        Modifier.fillMaxWidth(), maxLines = if (artBmp != null) 1 else 2, overflow = TextOverflow.Ellipsis, textAlign = TextAlign.Center,
                        style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.Bold)
                    Text(err ?: if (title != null) (subtitle ?: "on $pcName") else if (onPhone) "via $pcName" else "on $pcName",
                        Modifier.fillMaxWidth(), maxLines = 1, overflow = TextOverflow.Ellipsis, textAlign = TextAlign.Center,
                        style = MaterialTheme.typography.bodyMedium, color = if (err != null) K.accent else K.text2)
                }
                val iPrev = remember { MutableInteractionSource() }
                val iPlay = remember { MutableInteractionSource() }
                val iNext = remember { MutableInteractionSource() }
                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceEvenly, verticalAlignment = Alignment.CenterVertically) {
                    FilledTonalIconButton(onClick = { cmd(MediaCmd.PREV) }, interactionSource = iPrev, shapes = iconShapes(), modifier = Modifier.size(46.dp)) {
                        Icon(painterResource(R.drawable.ic_prev), "Previous", modifier = Modifier.size(24.dp))
                    }
                    FilledIconButton(onClick = { cmd(MediaCmd.PLAY_PAUSE) }, interactionSource = iPlay, shapes = iconShapes(), modifier = Modifier.size(72.dp)) {
                        Icon(painterResource(if (playing) R.drawable.ic_pause else R.drawable.ic_play), if (playing) "Pause" else "Play", modifier = Modifier.size(38.dp))
                    }
                    FilledTonalIconButton(onClick = { cmd(MediaCmd.NEXT) }, interactionSource = iNext, shapes = iconShapes(), modifier = Modifier.size(46.dp)) {
                        Icon(painterResource(R.drawable.ic_next), "Next", modifier = Modifier.size(24.dp))
                    }
                }
                val volText = if (onPhone) "Phone" else if (st == null || st.volume == null) "Crown = volume" else if (st.muted) "Muted · tap" else "Vol ${st.volume}% · tap"
                Column(horizontalAlignment = Alignment.CenterHorizontally) {
                    if (dur > 0) Text("${mmss(posMs)} / ${mmss(dur)}", textAlign = TextAlign.Center, maxLines = 1, style = MaterialTheme.typography.labelMedium, color = K.text)
                    Text(volText, Modifier.clickable { if (!onPhone) cmd(MediaCmd.MUTE) }.padding(vertical = 2.dp),
                        textAlign = TextAlign.Center, maxLines = 1, style = MaterialTheme.typography.labelSmall, color = K.text2)
                }
            }
        }
    }
}

// =============================================================================== Slides

@Composable
fun SlidesScreen(act: MainActivity) {
    KeepScreenOn()
    val scope = rememberCoroutineScope()
    val focus = remember { FocusRequester() }
    val detent = remember { RotaryDetent() }
    var count by remember { mutableIntStateOf(0) }
    var err by remember { mutableStateOf<String?>(null) }
    var flash by remember { mutableStateOf("") }

    fun go(c: Int) {
        act.cue(Feedback.Cue.SLIDE)
        flash = when (c) { PresentCmd.NEXT -> "Next"; PresentCmd.PREV -> "Back"; PresentCmd.BLACK -> "Blank"; else -> "" }
        scope.launch {
            val e = withContext(Dispatchers.IO) { WatchAura.present(act, c) }
            err = e
            if (e != null) act.cue(Feedback.Cue.ERROR) else if (c == PresentCmd.NEXT) count++ else if (c == PresentCmd.PREV && count > 0) count--
        }
    }
    LaunchedEffect(Unit) { focus.requestFocus() }

    ScreenScaffold { _ ->
        Column(
            Modifier.fillMaxSize().padding(start = 12.dp, end = 12.dp, top = 14.dp)
                .onRotaryScrollEvent { e ->
                    val n = detent.feed(e.verticalScrollPixels, System.currentTimeMillis())
                    if (n != 0) go(if (n > 0) PresentCmd.NEXT else PresentCmd.PREV)
                    n != 0
                }
                .focusRequester(focus).focusable(),
            horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.Center,
        ) {
            Icon(painterResource(R.drawable.ic_slides), null, tint = K.primary, modifier = Modifier.size(30.dp))
            Text(if (count == 0) "Slides" else "Slide +$count", style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold)
            Text(err ?: "Turn the bezel: clockwise = next", textAlign = TextAlign.Center, maxLines = 2, style = MaterialTheme.typography.bodySmall,
                color = if (err != null) K.accent else K.text2)
            Spacer(Modifier.height(8.dp))
            val iPrev = remember { MutableInteractionSource() }
            val iBlank = remember { MutableInteractionSource() }
            val iNext = remember { MutableInteractionSource() }
            ButtonGroup(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                FilledTonalIconButton(onClick = { go(PresentCmd.PREV) }, interactionSource = iPrev, shapes = iconShapes(),
                    modifier = Modifier.weight(1f).height(56.dp).animateWidth(iPrev)) { Icon(painterResource(R.drawable.ic_prev), "Previous slide") }
                FilledTonalIconButton(onClick = { go(PresentCmd.BLACK) }, interactionSource = iBlank, shapes = iconShapes(),
                    modifier = Modifier.weight(0.8f).height(46.dp).animateWidth(iBlank)) { Icon(painterResource(R.drawable.ic_blank), "Blank screen") }
                FilledIconButton(onClick = { go(PresentCmd.NEXT) }, interactionSource = iNext, shapes = iconShapes(),
                    modifier = Modifier.weight(1.3f).height(64.dp).animateWidth(iNext)) { Icon(painterResource(R.drawable.ic_next), "Next slide") }
            }
        }
    }
}
