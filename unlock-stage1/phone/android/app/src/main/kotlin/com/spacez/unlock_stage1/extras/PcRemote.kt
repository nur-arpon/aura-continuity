package com.spacez.unlock_stage1.extras

import android.content.Context
import com.spacez.aura.link.AuraLink
import com.spacez.aura.link.Kind
import com.spacez.aura.link.MediaCmd
import com.spacez.aura.link.MediaState
import com.spacez.aura.link.Message
import com.spacez.aura.link.PresentCmd
import com.spacez.unlock_stage1.UnlockState
import com.spacez.unlock_stage1.away.AwayPhone

/**
 * Item 15, phone end: ring the laptop, PC media controls and the presentation remote, all over the
 * existing Aura Link session (Bluetooth first, the tailnet right after a tap). Nothing here polls:
 * the media screen asks the PC for updates (REFRESH, valid 2 minutes) while it is open and the PC
 * pushes only changes.
 */
object PcRemote {
    @Volatile var media: MediaState? = null; private set
    @Volatile var art: ByteArray? = null; private set
    @Volatile var mediaAt = 0L; private set
    @Volatile var laptopRinging = false; private set
    @Volatile private var presenterKeys = false

    /** Test hook / unit tests: what the next send would be is observable through [sender]. */
    @Volatile var sender: ((Context, Message, Long) -> String?)? = null

    private fun now() = System.currentTimeMillis()

    /** null = delivered and acked by the PC; otherwise the reason in plain words. */
    fun send(c: Context, m: Message, waitMs: Long = 6_000): String? {
        sender?.let { return it(c, m, waitMs) }
        val l: AuraLink = AwayPhone.link(c) ?: return "Not paired with a PC"
        AwayPhone.userAction()
        val err = l.waitAcked(l.send(m), waitMs) ?: return null
        return friendly(err)
    }

    fun friendly(err: String): String = when {
        err.contains("timed out", true) || err.contains("timeout", true) -> "Your PC did not answer. Check Bluetooth, or Tailscale when away"
        err.isBlank() -> "Your PC did not answer"
        else -> err
    }

    fun ringPc(c: Context, on: Boolean): String {
        val err = send(c, Message.ring(on))
        UnlockState.log("ring laptop ${if (on) "on" else "off"}: ${err ?: "ok"}")
        if (err == null) laptopRinging = on
        return err ?: if (on) "Ringing your laptop" else "Stopped"
    }

    fun mediaCmd(c: Context, cmd: Int): String? {
        val err = send(c, Message.mediaCmd(cmd), if (cmd == MediaCmd.REFRESH) 6_000 else 3_000)
        if (err != null) UnlockState.log("media cmd $cmd: $err")
        return err
    }

    private val inputQueue = java.util.concurrent.Executors.newSingleThreadExecutor { r -> Thread(r, "aura-input").apply { isDaemon = true } }

    /** Remote mouse / keyboard (INPUT 0x45): one JSON event, no ack, 2 s lifetime; ordered on its own thread. */
    fun input(c: Context, json: String) {
        val app = c.applicationContext
        inputQueue.execute { runCatching { AwayPhone.link(app)?.send(Message.input(json)) } }
    }

    fun present(c: Context, cmd: Int): String? {
        val err = send(c, Message.present(cmd), 3_000)
        if (err != null) UnlockState.log("present $cmd: $err")
        return err
    }

    /** The media or presenter screen opened (or is still open): ask the PC to keep us posted. */
    fun watch(c: Context): String? = mediaCmd(c, MediaCmd.REFRESH)

    fun unwatch(c: Context) { Thread { send(c, Message.mediaCmd(MediaCmd.STOP_UPDATES), 2_000) }.start() }

    fun setPresenterKeys(on: Boolean) { presenterKeys = on }
    fun presenterKeysOn() = presenterKeys

    // ---------------------------------------------------------------- from the PC

    fun onMediaState(body: ByteArray) {
        runCatching { MediaState.decode(body) }.onSuccess {
            val old = media
            if (old == null || old.title != it.title || old.artist != it.artist) art = null
            media = it; mediaAt = now()
        }
    }

    fun onMediaArt(body: ByteArray) { art = body.takeIf { it.isNotEmpty() } }

    /** Laptop dismissed its alert: the phone's "Ringing your laptop" state ends. */
    fun onRingFromPc(c: Context, on: Boolean) {
        if (on) { PhoneRinger.start(c, "your PC"); return }
        // RING 0 means "stop" for a ringing phone, and "the laptop was found" for a ring this phone started
        if (PhoneRinger.ringing) PhoneRinger.stop(c, "PC stopped the ring")
        laptopRinging = false
    }

    /** Position now, interpolated from the last state while it plays. */
    fun positionNow(): Long {
        val m = media ?: return 0
        val p = if (m.playing) m.positionMs + (now() - mediaAt) else m.positionMs
        return if (m.durationMs > 0) p.coerceIn(0, m.durationMs) else p.coerceAtLeast(0)
    }

    fun status(): Map<String, Any?> {
        val m = media
        return mapOf(
            "title" to (m?.title ?: ""), "artist" to (m?.artist ?: ""), "playing" to (m?.playing == true),
            "pos" to positionNow(), "dur" to (m?.durationMs ?: 0L),
            "volume" to (m?.volume ?: -1), "muted" to (m?.muted == true),
            "art" to art, "at" to mediaAt,
            "laptopRinging" to laptopRinging, "phoneRinging" to PhoneRinger.ringing,
            "presenterKeys" to presenterKeys,
        )
    }
}
