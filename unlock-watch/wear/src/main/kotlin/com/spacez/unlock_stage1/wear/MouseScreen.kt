package com.spacez.unlock_stage1.wear

import android.app.Activity
import android.app.RemoteInput
import android.content.Intent
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.focusable
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.rotary.onRotaryScrollEvent
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.wear.compose.material3.ButtonDefaults
import androidx.wear.compose.material3.CompactButton
import androidx.wear.compose.material3.MaterialTheme
import androidx.wear.compose.material3.ScreenScaffold
import androidx.wear.compose.material3.Text
import androidx.wear.input.RemoteInputIntentHelper
import org.json.JSONObject

/** Turns many small touch deltas into at most one INPUT move per [minGapMs] (~60 Hz), keeping the sub-pixel remainder. */
class MoveCoalescer(private val gain: Float = 2.2f, private val minGapMs: Long = 16) {
    private var ax = 0f; private var ay = 0f
    private var last = 0L
    fun reset() { ax = 0f; ay = 0f }
    /** @return (dx, dy) to send now, or null while it is still too soon / nothing to send. */
    fun feed(dx: Float, dy: Float, nowMs: Long, force: Boolean = false): Pair<Int, Int>? {
        ax += dx * gain; ay += dy * gain
        if (!force && nowMs - last < minGapMs) return null
        val ix = ax.toInt(); val iy = ay.toInt()
        if (ix == 0 && iy == 0) return null
        ax -= ix; ay -= iy; last = nowMs
        return ix to iy
    }
}

/**
 * Remote mouse / keyboard for the PC (or the phone: the Laptop | Phone chip at the top, item 7). Touchpad: drag = move,
 * tap = left click, long-press or two-finger tap = right click (Back on the phone), bezel = scroll. "Type" opens the Wear keyboard / voice input and sends the text; Del / Enter are single keys.
 * Every event is one small INPUT message (no ack, 2 s lifetime, see aura-link msg.rs).
 */
@Composable
fun MouseScreen(act: MainActivity) {
    KeepScreenOn()
    val ctx = LocalContext.current
    val focus = remember { FocusRequester() }
    val mover = remember { MoveCoalescer() }
    var phone by remember { mutableStateOf(InputTarget.phone(ctx)) }
    val phoneState by PhoneInput.state.collectAsState()
    val twoTap = remember { TwoFingerTap() }
    var lastTwoMs by remember { mutableStateOf(0L) }
    fun send(o: JSONObject) = WatchAura.input(ctx, o.toString())
    fun rightClick() { act.cue(Feedback.Cue.SEND); send(JSONObject().put("t", "click").put("b", "right")) }
    fun key(k: String) = send(JSONObject().put("t", "key").put("k", k))

    val typer = rememberLauncherForActivityResult(ActivityResultContracts.StartActivityForResult()) { r ->
        if (r.resultCode == Activity.RESULT_OK) {
            val t = r.data?.let { RemoteInput.getResultsFromIntent(it) }?.getCharSequence("aura_text")?.toString()
            if (!t.isNullOrEmpty()) send(JSONObject().put("t", "text").put("s", t))
        }
    }
    fun type() {
        val i: Intent = RemoteInputIntentHelper.createActionRemoteInputIntent()
        RemoteInputIntentHelper.putRemoteInputsExtra(i, listOf(RemoteInput.Builder("aura_text").setLabel("Type on PC").build()))
        typer.launch(i)
    }
    LaunchedEffect(Unit) { focus.requestFocus(); if (phone) PhoneInput.warm(ctx) }

    ScreenScaffold { _ ->
        Box(Modifier.fillMaxSize()
            .onRotaryScrollEvent { e ->
                // clockwise = down; one wheel notch is 120
                val dy = (-e.verticalScrollPixels * 2f).toInt()
                if (dy != 0) send(JSONObject().put("t", "scroll").put("dy", dy))
                true
            }
            .focusRequester(focus).focusable()) {
            // the pad: a dark disc with a faint edge
            Box(Modifier.fillMaxSize().padding(6.dp).clip(CircleShape).background(K.high)
                .pointerInput(Unit) {
                    // watches every pointer event (never consumes): two fingers down and up quickly = right click
                    awaitEachGesture {
                        var go = true
                        while (go) {
                            val ev = awaitPointerEvent(PointerEventPass.Initial)
                            val now = ev.changes.first().uptimeMillis
                            val down = ev.changes.filter { it.pressed }.associate { it.id.value to (it.position.x to it.position.y) }
                            if (down.size >= 2) lastTwoMs = now
                            if (twoTap.update(down, now)) { lastTwoMs = now; rightClick() }
                            go = ev.changes.any { it.pressed } || twoTap.active
                        }
                    }
                }
                .pointerInput(Unit) {
                    detectTapGestures(
                        onTap = { if (System.currentTimeMillis() - lastTwoMs > 400 && !twoTap.active) { act.cue(Feedback.Cue.TAP); send(JSONObject().put("t", "click").put("b", "left")) } },
                        onLongPress = { if (!twoTap.active) rightClick() },
                    )
                }
                .pointerInput(Unit) {
                    detectDragGestures(
                        onDragEnd = { mover.feed(0f, 0f, System.currentTimeMillis(), force = true)?.let { (x, y) -> send(JSONObject().put("t", "move").put("dx", x).put("dy", y)) } },
                    ) { change, d ->
                        change.consume()
                        if (twoTap.active) return@detectDragGestures
                        mover.feed(d.x, d.y, System.currentTimeMillis())?.let { (x, y) -> send(JSONObject().put("t", "move").put("dx", x).put("dy", y)) }
                    }
                })
            if (phone && phoneState == "a11y_off") {
                Text("Turn on in Aura\non your phone", Modifier.align(Alignment.Center), style = MaterialTheme.typography.labelMedium, color = K.accent, textAlign = TextAlign.Center)
            } else if (phone) {
                Row(Modifier.align(Alignment.Center), horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                    CompactButton(onClick = { act.cue(Feedback.Cue.TAP); key("back") }, colors = ButtonDefaults.filledTonalButtonColors()) { Text("Back", maxLines = 1) }
                    CompactButton(onClick = { act.cue(Feedback.Cue.TAP); key("home") }, colors = ButtonDefaults.filledTonalButtonColors()) { Text("Home", maxLines = 1) }
                    CompactButton(onClick = { act.cue(Feedback.Cue.TAP); key("recents") }, colors = ButtonDefaults.filledTonalButtonColors()) { Text("Apps", maxLines = 1) }
                }
            } else Text("Touchpad", Modifier.align(Alignment.Center), style = MaterialTheme.typography.labelSmall, color = K.text2)
            CompactButton(onClick = { phone = !phone; InputTarget.set(ctx, phone); act.cue(Feedback.Cue.TOGGLE_ON) },
                modifier = Modifier.align(Alignment.TopCenter).padding(top = 20.dp),
                colors = if (phone) ButtonDefaults.buttonColors() else ButtonDefaults.filledTonalButtonColors()) { Text(if (phone) "Phone" else "Laptop", maxLines = 1) }
            Row(Modifier.align(Alignment.BottomCenter).padding(bottom = 24.dp), horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                CompactButton(onClick = { act.cue(Feedback.Cue.TAP); type() }) { Text("Type") }
                CompactButton(onClick = { act.cue(Feedback.Cue.TAP); key("backspace") }, colors = ButtonDefaults.filledTonalButtonColors()) { Text("Del") }
                CompactButton(onClick = { act.cue(Feedback.Cue.TAP); key("enter") }, colors = ButtonDefaults.filledTonalButtonColors()) { Text("Go") }
            }
        }
    }
}
