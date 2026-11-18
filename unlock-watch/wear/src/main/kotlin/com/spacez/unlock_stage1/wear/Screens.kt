package com.spacez.unlock_stage1.wear

import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.Spring
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.animateDpAsState
import androidx.compose.animation.core.spring
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.Canvas
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
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.ui.draw.clip
import androidx.wear.compose.material3.FilledTonalIconButton
import androidx.wear.compose.material3.IconButtonDefaults
import androidx.wear.compose.material3.IconButtonShapes
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.produceState
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.scale
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.wear.compose.foundation.lazy.TransformingLazyColumn
import androidx.wear.compose.foundation.lazy.TransformingLazyColumnState
import androidx.wear.compose.foundation.lazy.rememberTransformingLazyColumnState
import androidx.wear.compose.material3.AlertDialog
import androidx.wear.compose.material3.AlertDialogDefaults
import androidx.wear.compose.material3.Button
import androidx.wear.compose.material3.ButtonDefaults
import androidx.wear.compose.material3.CircularProgressIndicator
import androidx.wear.compose.material3.EdgeButton
import androidx.wear.compose.material3.EdgeButtonSize
import androidx.wear.compose.material3.FailureConfirmationDialog
import androidx.wear.compose.material3.Icon
import androidx.wear.compose.material3.ListHeader
import androidx.wear.compose.material3.MaterialTheme
import androidx.wear.compose.material3.ProgressIndicatorDefaults
import androidx.wear.compose.material3.ScreenScaffold
import androidx.wear.compose.material3.SuccessConfirmationDialog
import androidx.wear.compose.material3.SwitchButton
import androidx.wear.compose.material3.Text
import androidx.wear.compose.material3.confirmationDialogCurvedText
import androidx.wear.compose.material3.lazy.rememberTransformationSpec
import androidx.wear.compose.material3.lazy.transformedHeight
import androidx.wear.compose.material3.SurfaceTransformation
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

@Composable
private fun lookColor(l: PcLook): Color = when (l) {
    PcLook.UNLOCKED -> K.accent
    PcLook.LOCKED -> K.primary
    PcLook.IDLE -> K.outline
    PcLook.UNREACHABLE -> K.outline
    PcLook.NOT_PAIRED -> K.outline
}

/** One light tick per item the bezel / crown scrolls past (on top of the list's own rotary feel). */
@Composable
private fun RotaryTicks(act: MainActivity, state: TransformingLazyColumnState) {
    LaunchedEffect(state) {
        var first = true
        snapshotFlow { state.anchorItemIndex }.distinctUntilChanged().collect {
            if (first) first = false else if (state.isScrollInProgress) act.cue(Feedback.Cue.TICK)
        }
    }
}

/** Edge button colours from the app's own (Expressive) scheme: filled primary to unlock / pair, tonal primaryContainer to lock. */
@Composable
fun edgeColors(tonal: Boolean) = ButtonDefaults.buttonColors(
    containerColor = if (tonal) MaterialTheme.colorScheme.primaryContainer else MaterialTheme.colorScheme.primary,
    contentColor = if (tonal) MaterialTheme.colorScheme.onPrimaryContainer else MaterialTheme.colorScheme.onPrimary,
)

/** Icon-button shapes: morph on press in Expressive, plain in Classic. */
@Composable
fun iconShapes(): IconButtonShapes = if (LocalExpressive.current) IconButtonDefaults.animatedShapes() else IconButtonDefaults.shapes()

// =============================================================================== Home

@Composable
fun HomeScreen(act: MainActivity, ui: UiState, onPair: () -> Unit, onSettings: () -> Unit, onRemote: () -> Unit = {},
               onMusic: () -> Unit = {}, onSlides: () -> Unit = {}, onMouse: () -> Unit = {}, onAir: () -> Unit = {}) {
    val list = rememberTransformingLazyColumnState()
    val spec = rememberTransformationSpec()
    val scope = rememberCoroutineScope()
    var locking by remember { mutableStateOf(false) }
    var result by remember { mutableStateOf<LockResult?>(null) }
    var unlocking by remember { mutableStateOf(false) }
    var unlockResult by remember { mutableStateOf<UnlockResult?>(null) }
    RotaryTicks(act, list)

    fun lock() {
        if (locking) return
        act.cue(Feedback.Cue.LOCK)
        locking = true
        scope.launch {
            val r = LockAction.run(act)
            locking = false
            result = r
            act.cue(if (r == LockResult.DIRECT || r == LockResult.VIA_PHONE) Feedback.Cue.SUCCESS else Feedback.Cue.ERROR)
        }
    }

    fun unlock() {
        if (unlocking) return
        act.cue(Feedback.Cue.TAP)
        unlocking = true
        scope.launch {
            val r = UnlockAction.run(act)
            unlocking = false
            unlockResult = r
            act.cue(if (r == UnlockResult.SENT) Feedback.Cue.APPROVED else Feedback.Cue.ERROR)
        }
    }

    // One primary action on the edge: Unlock when the PC is at its lock screen, otherwise Lock.
    val primaryUnlock = ui.pcPaired && ui.pc.look == PcLook.LOCKED

    @Composable
    fun ActionContent(unlock: Boolean) {
        val busy = if (unlock) unlocking else locking
        if (busy) CircularProgressIndicator(modifier = Modifier.size(20.dp), strokeWidth = 3.dp,
            colors = ProgressIndicatorDefaults.colors(indicatorColor = MaterialTheme.colorScheme.onPrimary, trackColor = MaterialTheme.colorScheme.onPrimary.copy(alpha = 0.2f)))
        else Text(if (unlock) "Unlock PC" else "Lock PC", fontWeight = FontWeight.SemiBold, maxLines = 1, softWrap = false)
    }

    // The edge button sits in an overlay rather than ScreenScaffold's edgeButton slot: that slot only reveals the button
    // once the list is scrolled to its end, and the primary action must be there the moment the app opens.
    ScreenScaffold { padding ->
        Box(Modifier.fillMaxSize()) {
        TransformingLazyColumn(state = list, contentPadding = PaddingValues(top = padding.calculateTopPadding() + 2.dp, bottom = 70.dp)) {
            item { Hero(ui, Modifier.fillMaxWidth().transformedHeight(this, spec)) }
            if (ui.pcPaired) item {
                // the PC apps: expressive icon buttons that morph when pressed
                Row(Modifier.fillMaxWidth().transformedHeight(this, spec).padding(vertical = 2.dp), horizontalArrangement = Arrangement.SpaceEvenly, verticalAlignment = Alignment.Top) {
                    QuickApp(R.drawable.ic_play, "Music") { act.cue(Feedback.Cue.TAP); onMusic() }
                    QuickApp(R.drawable.ic_slides, "Slides") { act.cue(Feedback.Cue.TAP); onSlides() }
                    QuickApp(R.drawable.ic_laptop, "Mouse") { act.cue(Feedback.Cue.TAP); onMouse() }
                }
            }
            if (ui.pcPaired) item {
                Button(
                    onClick = { act.cue(Feedback.Cue.TAP); onRemote() },
                    modifier = Modifier.fillMaxWidth().transformedHeight(this, spec), transformation = SurfaceTransformation(spec),
                    colors = ButtonDefaults.filledTonalButtonColors(),
                    icon = { Icon(painterResource(R.drawable.ic_bell), null) },
                    label = { Text("Remote") },
                    secondaryLabel = { Text("Ring · wake · restart") },
                )
            }
            if (ui.pcPaired) item {
                Button(
                    onClick = { act.cue(Feedback.Cue.TAP); onAir() },
                    modifier = Modifier.fillMaxWidth().transformedHeight(this, spec), transformation = SurfaceTransformation(spec),
                    colors = ButtonDefaults.filledTonalButtonColors(),
                    icon = { Icon(painterResource(R.drawable.ic_hand), null) },
                    label = { Text("Air control") },
                    secondaryLabel = { Text("Wrist mouse + gestures") },
                )
            }
            if (ui.pcPaired) item {
                // the other half of Lock / Unlock, for when the PC state is not what the hero shows
                Button(
                    onClick = { if (primaryUnlock) lock() else unlock() },
                    modifier = Modifier.fillMaxWidth().transformedHeight(this, spec), transformation = SurfaceTransformation(spec),
                    colors = ButtonDefaults.filledTonalButtonColors(),
                    icon = { Icon(painterResource(if (primaryUnlock) R.drawable.ic_lock else R.drawable.ic_lock_open), null) },
                    label = { Text(if (primaryUnlock) "Lock PC" else "Unlock PC") },
                    secondaryLabel = { Text(if (primaryUnlock) "Lock it instead" else "At its lock screen") },
                )
            }
            if (ui.testActive) item {
                Button(
                    onClick = {}, modifier = Modifier.fillMaxWidth().transformedHeight(this, spec), transformation = SurfaceTransformation(spec),
                    icon = { Icon(painterResource(R.drawable.ic_science), null) },
                    label = { Text("Test mode on") },
                    secondaryLabel = { Text("Until ${StatusText.clock(act, ui.testUntil)}") },
                    colors = ButtonDefaults.filledVariantButtonColors(),
                )
            }
            if (!ui.permissionsOk) item {
                Button(
                    onClick = { act.cue(Feedback.Cue.TAP); act.askPermissions() },
                    modifier = Modifier.fillMaxWidth().transformedHeight(this, spec), transformation = SurfaceTransformation(spec),
                    icon = { Icon(painterResource(R.drawable.ic_bluetooth), null) },
                    label = { Text("Allow nearby devices") },
                    secondaryLabel = { Text("Needed to reach your PC") },
                    colors = ButtonDefaults.filledTonalButtonColors(),
                )
            }
            item {
                StatusRow(act, spec, this, R.drawable.ic_watch, "This watch",
                    when {
                        !ui.wristRead -> "Checking…"
                        ui.testActive -> "Test mode · answers as worn"
                        !ui.wristKnown -> if (ui.watchLocked) "Locked" else "Unlocked"
                        else -> (if (ui.onWrist) "On wrist" else "Off wrist") + " · " + (if (ui.watchLocked) "Locked" else "Unlocked")
                    }, onSettings)
            }
            item {
                StatusRow(act, spec, this, R.drawable.ic_hand, stringResource(R.string.feature_unlock),
                    when {
                        !ui.pcPaired -> "Pair with PC first"
                        !ui.handsFree -> "Off"
                        ui.listening && ui.ready -> "Ready · listening"
                        ui.listening -> "On · needs wrist + unlock"
                        else -> "On · starting…"
                    }, onSettings)
            }
            item {
                StatusRow(act, spec, this, R.drawable.ic_phone, "Phone",
                    when {
                        ui.phoneReachable == false -> "Not reachable"
                        !ui.phonePaired -> if (ui.phoneReachable == true) "Connected · not paired" else "Not paired"
                        ui.phoneReachable == true -> "Connected · paired"
                        else -> "Paired"
                    }, onSettings)
            }
            item {
                Button(
                    onClick = { act.cue(Feedback.Cue.TAP); onSettings() },
                    modifier = Modifier.fillMaxWidth().transformedHeight(this, spec), transformation = SurfaceTransformation(spec),
                    colors = ButtonDefaults.filledTonalButtonColors(),
                    icon = { Icon(painterResource(R.drawable.ic_settings), null) },
                    label = { Text("Settings") },
                )
            }
            item { LeanChip(Modifier.fillMaxWidth().transformedHeight(this, spec)) }
        }
        // a soft scrim so list items fade out under the edge button instead of peeking around it
        Box(Modifier.align(Alignment.BottomCenter).fillMaxWidth().height(40.dp)
            .background(Brush.verticalGradient(listOf(Color.Transparent, MaterialTheme.colorScheme.background))))
        Box(Modifier.align(Alignment.BottomCenter)) {
            when {
                !ui.anyPaired -> EdgeButton(onClick = { act.cue(Feedback.Cue.TAP); onPair() }, buttonSize = EdgeButtonSize.ExtraSmall, colors = edgeColors(false)) {
                    Text("Pair with PC", fontWeight = FontWeight.SemiBold, maxLines = 1, softWrap = false)
                }
                else -> EdgeButton(onClick = { if (primaryUnlock) unlock() else lock() }, buttonSize = EdgeButtonSize.ExtraSmall, colors = edgeColors(!primaryUnlock)) { ActionContent(primaryUnlock) }
            }
        }
        }
    }

    val cts = androidx.wear.compose.material3.ConfirmationDialogDefaults.curvedTextStyle
    SuccessConfirmationDialog(
        visible = result == LockResult.DIRECT || result == LockResult.VIA_PHONE,
        onDismissRequest = { result = null },
        curvedText = { confirmationDialogCurvedText(if (result == LockResult.VIA_PHONE) "Sent via phone" else "PC locked", cts) },
    )
    SuccessConfirmationDialog(
        visible = unlockResult == UnlockResult.SENT,
        onDismissRequest = { unlockResult = null },
        curvedText = { confirmationDialogCurvedText("Approved", cts) },
    )
    FailureConfirmationDialog(
        visible = unlockResult == UnlockResult.NOT_READY || unlockResult == UnlockResult.FAILED || unlockResult == UnlockResult.NOT_PAIRED,
        onDismissRequest = { unlockResult = null },
        curvedText = {
            confirmationDialogCurvedText(when (unlockResult) {
                UnlockResult.NOT_READY -> "Wear + unlock watch"
                UnlockResult.NOT_PAIRED -> "Pair first"
                else -> "PC out of reach"
            }, cts)
        },
    )
    FailureConfirmationDialog(
        visible = result == LockResult.FAILED || result == LockResult.NOT_PAIRED,
        onDismissRequest = { result = null },
        curvedText = { confirmationDialogCurvedText(if (result == LockResult.NOT_PAIRED) "Pair first" else "PC out of reach", cts) },
    )
}

/** A labelled expressive icon button (morphs from circle to a rounded square while pressed). */
@Composable
private fun QuickApp(icon: Int, label: String, onClick: () -> Unit) {
    Column(horizontalAlignment = Alignment.CenterHorizontally) {
        FilledTonalIconButton(onClick = onClick, modifier = Modifier.size(52.dp), shapes = iconShapes()) {
            Icon(painterResource(icon), contentDescription = label, modifier = Modifier.size(26.dp))
        }
        Text(label, style = MaterialTheme.typography.labelSmall, color = K.text2, maxLines = 1)
    }
}

@Composable
private fun StatusRow(act: MainActivity, spec: androidx.wear.compose.material3.lazy.TransformationSpec,
                      scope: androidx.wear.compose.foundation.lazy.TransformingLazyColumnItemScope,
                      icon: Int, title: String, value: String, onClick: () -> Unit) = with(scope) {
    Button(
        onClick = { act.cue(Feedback.Cue.TAP); onClick() },
        modifier = Modifier.fillMaxWidth().transformedHeight(this, spec), transformation = SurfaceTransformation(spec),
        contentPadding = PaddingValues(horizontal = 14.dp, vertical = 6.dp),
        icon = { Icon(painterResource(icon), null, tint = K.primary, modifier = Modifier.size(18.dp)) },
        label = { Text(title, style = MaterialTheme.typography.labelMedium, color = K.text2, maxLines = 1) },
        secondaryLabel = { Text(value, style = MaterialTheme.typography.bodyMedium, color = K.text, maxLines = 2, overflow = TextOverflow.Ellipsis) },
        colors = ButtonDefaults.filledTonalButtonColors(containerColor = K.low),
    )
}

/** The hero: a big round-friendly tile in the PC state colour (circle when open, squircle when locked). */
@Composable
private fun Hero(ui: UiState, modifier: Modifier) {
    val motion = MaterialTheme.motionScheme
    val expressive = LocalExpressive.current
    val color by animateColorAsState(lookColor(ui.pc.look), motion.defaultEffectsSpec(), label = "orb")
    val pct by animateFloatAsState(if (expressive && ui.pc.look == PcLook.LOCKED) 0.34f else 0.5f, motion.defaultSpatialSpec(), label = "corner")
    val shape = RoundedCornerShape(percent = (pct * 100).toInt())
    val appear = remember { Animatable(0.85f) }
    LaunchedEffect(Unit) { appear.animateTo(1f, spring(dampingRatio = Spring.DampingRatioMediumBouncy, stiffness = Spring.StiffnessLow)) }
    Column(modifier.padding(top = 0.dp, bottom = 2.dp), horizontalAlignment = Alignment.CenterHorizontally) {
        Box(Modifier.size(44.dp).scale(appear.value).clip(shape).background(color.copy(alpha = 0.20f)).border(2.dp, color, shape), contentAlignment = Alignment.Center) {
            Icon(
                painterResource(when (ui.pc.look) { PcLook.UNLOCKED -> R.drawable.ic_lock_open; PcLook.LOCKED -> R.drawable.ic_lock; else -> R.drawable.ic_laptop }),
                contentDescription = null, tint = if (ui.pc.look == PcLook.UNLOCKED || ui.pc.look == PcLook.LOCKED) color else K.text2, modifier = Modifier.size(22.dp),
            )
        }
        Spacer(Modifier.height(2.dp))
        // the PC name is dropped once paired: the row of PC apps must clear the edge button on a small round screen
        if (!ui.pcPaired) Text(stringResource(R.string.app_name), style = MaterialTheme.typography.labelMedium, color = K.text2, maxLines = 1)
        AnimatedContent(ui.pc.title, transitionSpec = { (fadeIn(tween(220)) + scaleIn(initialScale = 0.92f)) togetherWith fadeOut(tween(120)) }, label = "title") { t ->
            Text(t, style = MaterialTheme.typography.titleMedium, color = K.text, textAlign = TextAlign.Center, maxLines = 1)
        }
        // the "2 h ago" part is worked out on every tick, so it never goes stale on screen
        val now by produceState(System.currentTimeMillis()) { while (true) { delay(30_000); value = System.currentTimeMillis() } }
        val detail = ui.pc.detailAt(now)
        if (detail.isNotEmpty()) Text(detail, style = MaterialTheme.typography.bodySmall, color = K.text2, textAlign = TextAlign.Center, maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}

@Composable
private fun LeanChip(modifier: Modifier) {
    Row(modifier.padding(top = 6.dp, bottom = 4.dp), horizontalArrangement = Arrangement.Center, verticalAlignment = Alignment.CenterVertically) {
        Icon(painterResource(R.drawable.ic_eco), null, tint = K.outline, modifier = Modifier.size(14.dp))
        Spacer(Modifier.width(4.dp))
        Text("Battery-lean · event-driven", style = MaterialTheme.typography.labelSmall, color = K.outline)
    }
}

// =============================================================================== Pair with PC

@Composable
fun PairScreen(act: MainActivity, ui: UiState, onDone: () -> Unit) {
    val total = PcLinkProtocol.PAIR_WINDOW_MS
    var opened by remember { mutableLongStateOf(0L) }
    var attempt by remember { mutableIntStateOf(0) }
    var frac by remember { mutableStateOf(1f) }
    var secs by remember { mutableIntStateOf((total / 1000).toInt()) }

    // open the window (and start the listener) every attempt; close it when leaving the screen.
    // The ring is driven from the wall clock (one update per frame-ish), only during these 60 s.
    LaunchedEffect(attempt) {
        val lp = PcLinkPrefs(act)
        opened = System.currentTimeMillis()
        lp.pairUntil = opened + total
        PcLinkService.sync(act)
        while (true) {
            val leftMs = (opened + total - System.currentTimeMillis()).coerceAtLeast(0)
            frac = leftMs.toFloat() / total
            val left = ((leftMs + 999) / 1000).toInt()
            if (left != secs) { secs = left; if (left in 1..5) act.cue(Feedback.Cue.COUNTDOWN) }
            if (leftMs == 0L) break
            delay(50)
        }
    }
    androidx.compose.runtime.DisposableEffect(Unit) {
        onDispose { if (PcLinkPrefs(act).pairOpen) { PcLinkPrefs(act).pairUntil = 0; PcLinkService.sync(act) } }
    }

    val success = ui.lastEvent == PcEvent.PAIRED && ui.lastEventMs >= opened && opened > 0
    val expired = !success && secs == 0
    LaunchedEffect(success) {
        if (success) { act.cue(Feedback.Cue.SUCCESS); delay(3000); onDone() }
    }
    val ringColor by animateColorAsState(if (success) K.accent else if (expired) K.line else K.primary, tween(400), label = "ring")

    ScreenScaffold(timeText = {}) {
        Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
            CircularProgressIndicator(
                progress = { if (success) 1f else frac },
                modifier = Modifier.fillMaxSize().padding(3.dp),
                strokeWidth = 6.dp,
                colors = ProgressIndicatorDefaults.colors(indicatorColor = ringColor, trackColor = K.mid),
            )
            AnimatedContent(
                when { success -> 2; expired -> 1; else -> 0 },
                transitionSpec = { (fadeIn(tween(250)) + scaleIn(initialScale = 0.9f)) togetherWith fadeOut(tween(150)) },
                label = "pair",
            ) { phase ->
                Column(Modifier.padding(horizontal = 30.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                    when (phase) {
                        2 -> {
                            Icon(painterResource(R.drawable.ic_check), null, tint = K.accent, modifier = Modifier.size(40.dp))
                            Text("Paired", style = MaterialTheme.typography.titleLarge, color = K.text)
                            Text(ui.pcName.ifBlank { "Your PC" }, style = MaterialTheme.typography.bodyMedium, color = K.text2, maxLines = 1)
                            Spacer(Modifier.height(4.dp))
                            Text(stringResource(R.string.feature_unlock) + " is on", style = MaterialTheme.typography.labelSmall, color = K.accent, textAlign = TextAlign.Center)
                        }
                        1 -> {
                            Text("Window closed", style = MaterialTheme.typography.titleMedium, color = K.text)
                            Text("The PC didn't ask in time", style = MaterialTheme.typography.bodySmall, color = K.text2, textAlign = TextAlign.Center)
                            Spacer(Modifier.height(8.dp))
                            Button(onClick = { act.cue(Feedback.Cue.TAP); attempt++ }, label = { Text("Try again") })
                        }
                        else -> {
                            Text("PAIR WITH PC", style = MaterialTheme.typography.labelSmall, color = K.primary, letterSpacing = 1.5.sp)
                            Text("$secs", style = MaterialTheme.typography.displayMedium, color = K.text, fontWeight = FontWeight.Medium)
                            if (!ui.listening) Text(if (!ui.permissionsOk) "Allow nearby devices" else "Starting Bluetooth…", style = MaterialTheme.typography.labelSmall, color = K.accent)
                            else Text("On your PC run", style = MaterialTheme.typography.labelSmall, color = K.text2)
                            Text("unlock-pc pair-watch", style = MaterialTheme.typography.labelMedium, color = K.text, fontWeight = FontWeight.SemiBold, maxLines = 1)
                            Spacer(Modifier.height(6.dp))
                            Text("CODE", style = MaterialTheme.typography.labelSmall, color = K.outline, letterSpacing = 1.5.sp)
                            Text(StatusText.code(ui.fingerprint), style = MaterialTheme.typography.titleSmall, color = K.accent, fontWeight = FontWeight.SemiBold, letterSpacing = 0.5.sp, maxLines = 1)
                        }
                    }
                }
            }
        }
    }
}

// =============================================================================== Settings

@Composable
fun SettingsScreen(act: MainActivity, ui: UiState, onPair: () -> Unit, onAbout: () -> Unit) {
    val list = rememberTransformingLazyColumnState()
    val spec = rememberTransformationSpec()
    val scope = rememberCoroutineScope()
    var forget by remember { mutableStateOf(false) }
    var phoneCode by remember { mutableStateOf<String?>(null) }
    RotaryTicks(act, list)

    fun toggleCue(on: Boolean) = act.cue(if (on) Feedback.Cue.TOGGLE_ON else Feedback.Cue.TOGGLE_OFF)

    ScreenScaffold(scrollState = list) { padding ->
        TransformingLazyColumn(state = list, contentPadding = padding) {
            item { ListHeader(Modifier.fillMaxWidth().transformedHeight(this, spec), transformation = SurfaceTransformation(spec)) { Text("Settings") } }
            item {
                SwitchButton(
                    checked = ui.handsFree && ui.pcPaired, enabled = ui.pcPaired,
                    onCheckedChange = { PcLinkPrefs(act).enabled = it; PcLinkService.sync(act); toggleCue(it) },
                    modifier = Modifier.fillMaxWidth().transformedHeight(this, spec), transformation = SurfaceTransformation(spec),
                    label = { Text(stringResource(R.string.feature_unlock)) },
                    secondaryLabel = { Text(if (ui.pcPaired) "When worn + unlocked" else "Pair with PC first") },
                )
            }
            item {
                SwitchButton(
                    checked = ui.lockWhenOff,
                    onCheckedChange = { WatchPrefs(act).liveLog = it; PcLinkService.sync(act); toggleCue(it) },
                    modifier = Modifier.fillMaxWidth().transformedHeight(this, spec), transformation = SurfaceTransformation(spec),
                    label = { Text("Lock when removed") },
                    secondaryLabel = { Text("Lock PC when the watch comes off") },
                )
            }
            item {
                var extras by remember { mutableStateOf(WatchAura.extrasOn(act)) }
                SwitchButton(
                    checked = extras && ui.pcPaired, enabled = ui.pcPaired,
                    onCheckedChange = { extras = it; WatchAura.setExtras(act, it); toggleCue(it) },
                    modifier = Modifier.fillMaxWidth().transformedHeight(this, spec), transformation = SurfaceTransformation(spec),
                    label = { Text("Remote extras") },
                    secondaryLabel = { Text(if (ui.pcPaired) "Ring laptop, music, slides" else "Pair with PC first") },
                )
            }
            item { ListHeader(Modifier.fillMaxWidth().transformedHeight(this, spec), transformation = SurfaceTransformation(spec)) { Text("Look") } }
            item {
                SwitchButton(
                    checked = ui.expressive,
                    onCheckedChange = { WatchPrefs(act).expressive = it; act.reload(); toggleCue(it) },
                    modifier = Modifier.fillMaxWidth().transformedHeight(this, spec), transformation = SurfaceTransformation(spec),
                    label = { Text("Expressive look") },
                    secondaryLabel = { Text(if (ui.expressive) "Material 3 Expressive" else "Classic navy") },
                )
            }
            if (ui.expressive) item {
                SwitchButton(
                    checked = ui.colour == COLOUR_WATCH,
                    onCheckedChange = { WatchPrefs(act).colour = if (it) COLOUR_WATCH else Seed.BLUE.key; act.reload(); toggleCue(it) },
                    modifier = Modifier.fillMaxWidth().transformedHeight(this, spec), transformation = SurfaceTransformation(spec),
                    label = { Text("Watch colours") },
                    secondaryLabel = { Text(if (ui.colour == COLOUR_WATCH) "From your watch face" else "Pick a colour below") },
                )
            }
            if (ui.expressive) item {
                Row(Modifier.fillMaxWidth().transformedHeight(this, spec).padding(vertical = 4.dp), horizontalArrangement = Arrangement.SpaceEvenly, verticalAlignment = Alignment.CenterVertically) {
                    Seed.entries.forEach { sd ->
                        val on = ui.colour == sd.key
                        val c = seedScheme(sd.hue).primary
                        val size by animateDpAsState(if (on) 40.dp else 34.dp, MaterialTheme.motionScheme.fastSpatialSpec(), label = "swatch")
                        Box(
                            Modifier.size(size).clip(CircleShape).background(c).border(if (on) 3.dp else 0.dp, K.text, CircleShape)
                                .clickable(onClickLabel = sd.label) { WatchPrefs(act).colour = sd.key; act.reload(); act.cue(Feedback.Cue.TOGGLE_ON) },
                            contentAlignment = Alignment.Center,
                        ) { if (on) Icon(painterResource(R.drawable.ic_check), sd.label, tint = Color.Black.copy(alpha = 0.7f), modifier = Modifier.size(18.dp)) }
                    }
                }
            }
            item { ListHeader(Modifier.fillMaxWidth().transformedHeight(this, spec), transformation = SurfaceTransformation(spec)) { Text("Feel") } }
            item {
                SwitchButton(
                    checked = ui.sound,
                    onCheckedChange = { Feedback.setSound(act, it); act.reload(); toggleCue(it) },
                    modifier = Modifier.fillMaxWidth().transformedHeight(this, spec), transformation = SurfaceTransformation(spec),
                    label = { Text("Sounds") },
                    secondaryLabel = { Text("Silent in DND, Theater, vibrate") },
                )
            }
            item {
                Button(
                    onClick = {
                        val next = (ui.haptics + 3) % 4 // Max -> Medium -> Light -> Off -> Max
                        Feedback.setIntensity(act, next); act.reload(); act.cue(Feedback.Cue.LOCK)
                    },
                    modifier = Modifier.fillMaxWidth().transformedHeight(this, spec), transformation = SurfaceTransformation(spec),
                    label = { Text("Haptics") },
                    secondaryLabel = { Text(listOf("Off", "Light", "Medium", "Max")[ui.haptics.coerceIn(0, 3)]) },
                    icon = { Icon(painterResource(R.drawable.ic_watch), null, tint = K.primary) },
                    colors = ButtonDefaults.filledTonalButtonColors(),
                )
            }
            item { ListHeader(Modifier.fillMaxWidth().transformedHeight(this, spec), transformation = SurfaceTransformation(spec)) { Text("Devices") } }
            item {
                Button(
                    onClick = { act.cue(Feedback.Cue.TAP); onPair() },
                    modifier = Modifier.fillMaxWidth().transformedHeight(this, spec), transformation = SurfaceTransformation(spec),
                    icon = { Icon(painterResource(R.drawable.ic_laptop), null, tint = K.primary) },
                    label = { Text(if (ui.pcPaired) "Pair PC again" else "Pair with PC") },
                    secondaryLabel = { Text(if (ui.pcPaired) ui.pcName.ifBlank { "Paired" } else "Not paired") },
                    colors = ButtonDefaults.filledTonalButtonColors(),
                )
            }
            item {
                Button(
                    onClick = {
                        act.cue(Feedback.Cue.TAP)
                        scope.launch {
                            val n = PhoneLink.pair(act)
                            if (n > 0) phoneCode = ui.fingerprint else act.cue(Feedback.Cue.ERROR).also { phoneCode = "" }
                        }
                    },
                    modifier = Modifier.fillMaxWidth().transformedHeight(this, spec), transformation = SurfaceTransformation(spec),
                    icon = { Icon(painterResource(R.drawable.ic_phone), null, tint = K.primary) },
                    label = { Text(if (ui.phonePaired) "Phone paired" else "Pair with phone") },
                    secondaryLabel = { Text(if (ui.phonePaired) "Tap to send the key again" else "Relay + approvals") },
                    colors = ButtonDefaults.filledTonalButtonColors(),
                )
            }
            if (ui.pcPaired) item {
                Button(
                    onClick = { act.cue(Feedback.Cue.TAP); forget = true },
                    modifier = Modifier.fillMaxWidth().transformedHeight(this, spec), transformation = SurfaceTransformation(spec),
                    label = { Text("Forget this PC") },
                    colors = ButtonDefaults.outlinedButtonColors(), border = ButtonDefaults.outlinedButtonBorder(enabled = true),
                )
            }
            item {
                Button(
                    onClick = { act.cue(Feedback.Cue.TAP); onAbout() },
                    modifier = Modifier.fillMaxWidth().transformedHeight(this, spec), transformation = SurfaceTransformation(spec),
                    icon = { Icon(painterResource(R.drawable.ic_info), null, tint = K.primary) },
                    label = { Text("About") },
                    colors = ButtonDefaults.filledTonalButtonColors(),
                )
            }
        }
    }

    AlertDialog(
        visible = forget, onDismissRequest = { forget = false },
        icon = { Icon(painterResource(R.drawable.ic_laptop), null) },
        title = { Text("Forget ${ui.pcName.ifBlank { "this PC" }}?") },
        text = { Text(stringResource(R.string.feature_unlock) + " stops until you pair again.") },
        confirmButton = {
            AlertDialogDefaults.ConfirmButton(onClick = {
                PcLinkPrefs(act).forget(); PcLinkService.sync(act); Surfaces.refresh(act); act.cue(Feedback.Cue.TOGGLE_OFF); forget = false
            })
        },
        dismissButton = { AlertDialogDefaults.DismissButton(onClick = { forget = false }) },
    )
    AlertDialog(
        visible = phoneCode != null, onDismissRequest = { phoneCode = null },
        icon = { Icon(painterResource(R.drawable.ic_phone), null) },
        title = { Text(if (phoneCode.isNullOrEmpty()) "Phone not reachable" else "Check your phone") },
        text = {
            Text(if (phoneCode.isNullOrEmpty()) "Open Aura on the phone and try again." else "Tap Same code if it shows\n${StatusText.code(phoneCode!!)}",
                textAlign = TextAlign.Center)
        },
        edgeButton = { AlertDialogDefaults.EdgeButton(onClick = { phoneCode = null }) },
    )
}

// =============================================================================== About

@Composable
fun AboutScreen(ui: UiState) {
    val list = rememberTransformingLazyColumnState()
    val spec = rememberTransformationSpec()
    ScreenScaffold(scrollState = list) { padding ->
        TransformingLazyColumn(state = list, contentPadding = padding) {
            item { ListHeader(Modifier.fillMaxWidth().transformedHeight(this, spec), transformation = SurfaceTransformation(spec)) { Text(stringResource(R.string.app_name)) } }
            item { Info(spec, this, "What it is", stringResource(R.string.app_descriptor)) }
            item { Info(spec, this, "Version", BuildInfo.VERSION) }
            item { Info(spec, this, "Watch key", StatusText.code(ui.fingerprint)) }
            item { Info(spec, this, "PC", if (ui.pcPaired) "${ui.pcName} · ${ui.pcAddress}" else "Not paired") }
            item { Info(spec, this, "Battery", "Event-driven. No polling, no wakelocks; the listener sleeps until your PC asks.") }
            item { Info(spec, this, "Safety", "The watch unlocks your PC only when you tap Unlock PC or approve a sign-in, while it is worn and unlocked. The PC checks a signed, fresh answer; never on its own.") }
            item { Info(spec, this, "Bluetooth LE", ui.ble) }
        }
    }
}

@Composable
private fun Info(spec: androidx.wear.compose.material3.lazy.TransformationSpec, scope: androidx.wear.compose.foundation.lazy.TransformingLazyColumnItemScope,
                 title: String, body: String) = with(scope) {
    Column(Modifier.fillMaxWidth().transformedHeight(this, spec).padding(horizontal = 14.dp, vertical = 6.dp)) {
        Text(title.uppercase(), style = MaterialTheme.typography.labelSmall, color = K.primary, letterSpacing = 1.2.sp)
        Text(body, style = MaterialTheme.typography.bodyMedium, color = K.text)
    }
}
