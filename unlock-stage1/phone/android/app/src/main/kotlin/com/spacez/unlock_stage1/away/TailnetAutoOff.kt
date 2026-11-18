package com.spacez.unlock_stage1.away

import android.app.usage.UsageEvents
import android.app.usage.UsageStatsManager
import android.content.Context
import android.content.Intent
import android.os.Handler
import android.os.Looper
import com.spacez.unlock_stage1.UnlockState
import com.spacez.unlock_stage1.zerotap.Foreground

/**
 * "Turn Tailscale off after streaming". When Aura itself switched Tailscale on to open the laptop stream
 * (Tailnet.ensureUp), switch it off again once Artemis / Moonlight has been out of the foreground for 2 minutes and no other
 * Aura away-action is running. If Tailscale was already on (the owner turned it on himself) Aura never touches it.
 *
 * Battery-light: nothing runs unless Aura turned Tailscale on; then one cheap Usage-access look every 30 s until it is
 * turned off (or the owner turned it off). No wake locks, no tight polling.
 */
object TailnetAutoOff {
    const val PREFS = "away"
    private const val K_ON = "ts_auto_off"          // setting, default ON
    private const val K_BY_AURA = "ts_by_aura_at"   // when Aura turned it on (0 = it did not)
    const val GRACE_MS = 120_000L
    const val TICK_MS = 30_000L
    val STREAM_PKGS = setOf("com.limelight.noir", "com.limelight")

    // ------------------------------------------------------------------ pure part (JVM-tested)

    /** ActivityEvent types used here (UsageEvents.Event): RESUMED = 1, PAUSED = 2, STOPPED = 23. */
    data class Ev(val pkg: String, val type: Int, val at: Long)

    /** (in the stream app right now, last time it was seen in front or leaving it) from usage events; null = never seen. */
    fun streamState(events: List<Ev>): Pair<Boolean, Long?> {
        var inFront = false
        var last: Long? = null
        for (e in events.sortedBy { it.at }) {
            if (e.pkg !in STREAM_PKGS) continue
            when (e.type) {
                1 -> { inFront = true; last = e.at }
                2, 23 -> { inFront = false; last = e.at }
            }
        }
        return inFront to last
    }

    enum class Verdict { WAIT, OFF }

    /**
     * [armedAt] = when Aura turned Tailscale on, [lastStream] = last time the stream app was in front / left (null = not seen since).
     * Off only after GRACE_MS away from the stream app (counted from arming when it never opened) with nothing else running.
     */
    fun decide(now: Long, armedAt: Long, inStream: Boolean, lastStream: Long?, busy: Boolean): Verdict {
        if (inStream || busy) return Verdict.WAIT
        val since = maxOf(armedAt, lastStream ?: 0L)
        return if (now - since >= GRACE_MS) Verdict.OFF else Verdict.WAIT
    }

    // ------------------------------------------------------------------ Android part

    private val main = Handler(Looper.getMainLooper())
    @Volatile private var app: Context? = null

    private fun p(c: Context) = c.getSharedPreferences(PREFS, 0)
    fun enabled(c: Context) = p(c).getBoolean(K_ON, true)
    fun setEnabled(c: Context, on: Boolean) {
        p(c).edit().putBoolean(K_ON, on).apply()
        if (!on) main.removeCallbacks(tick) else if (armedAt(c) > 0) arm(c, armedAt(c))
    }
    private fun armedAt(c: Context) = p(c).getLong(K_BY_AURA, 0)

    /** Tailnet.ensureUp just switched Tailscale on by itself: remember it and start the (bounded) watch. */
    fun armed(c: Context) { arm(c, System.currentTimeMillis()) }

    /** Called when the process starts: a watch that was running when the process died continues. */
    fun resume(c: Context) { if (armedAt(c) > 0 && enabled(c)) arm(c, armedAt(c)) }

    private fun arm(c: Context, at: Long) {
        app = c.applicationContext
        p(c).edit().putLong(K_BY_AURA, at).apply()
        if (!enabled(c)) return
        main.removeCallbacks(tick); main.postDelayed(tick, TICK_MS)
        UnlockState.log("tailscale auto-off: armed (Aura turned Tailscale on), checking every ${TICK_MS / 1000} s")
    }

    /** The owner turned Tailscale off (or it dropped): nothing of ours to undo any more. */
    fun disarm(c: Context, why: String) {
        main.removeCallbacks(tick)
        if (armedAt(c) > 0) { p(c).edit().putLong(K_BY_AURA, 0).apply(); UnlockState.log("tailscale auto-off: disarmed ($why)") }
    }

    private val tick = object : Runnable {
        override fun run() {
            val c = app ?: return
            if (armedAt(c) == 0L || !enabled(c)) return
            if (Tailnet.ownIp() == null) { disarm(c, "Tailscale is already off"); return }
            // the look (a usage-stats query) is off the main thread
            Thread {
                val st = runCatching { look(c, armedAt(c)) }.getOrNull()
                main.post { step(c, st) }
            }.start()
        }
    }

    private fun step(c: Context, st: Pair<Boolean, Long?>?) {
        if (armedAt(c) == 0L) return
        val now = System.currentTimeMillis()
        if (st == null) {
            // no way to see the foreground (no Usage access): never guess, a wrong guess would cut a live stream
            UnlockState.log("tailscale auto-off: cannot see the foreground (Usage access off): leaving Tailscale on")
            disarm(c, "no foreground signal"); return
        }
        val busy = AwayPhone.gate.isOpen(now)
        if (decide(now, armedAt(c), st.first, st.second, busy) == Verdict.OFF) {
            runCatching { c.sendBroadcast(Intent("com.tailscale.ipn.DISCONNECT_VPN").setClassName(AwayLogic.TAILSCALE_PKG, "com.tailscale.ipn.IPNReceiver")) }
            UnlockState.log("tailscale auto-off: the stream app has been closed for 2 minutes: Tailscale turned off")
            disarm(c, "turned off")
        } else main.postDelayed(tick, TICK_MS)
    }

    /** Usage events since just before Aura turned Tailscale on (a long stream has no recent event: its RESUMED is old) -> (stream app in front now, last time seen); null when Usage access is not granted. */
    private fun look(c: Context, armedAt: Long): Pair<Boolean, Long?>? {
        if (!Foreground.granted(c)) return null
        val usm = c.getSystemService(UsageStatsManager::class.java) ?: return null
        val now = System.currentTimeMillis()
        val ev = usm.queryEvents(maxOf(armedAt - 60_000L, now - 6 * 3_600_000L), now)
        val e = UsageEvents.Event()
        val list = ArrayList<Ev>()
        while (ev.hasNextEvent()) {
            ev.getNextEvent(e)
            if (e.packageName in STREAM_PKGS) list.add(Ev(e.packageName, e.eventType, e.timeStamp))
        }
        return streamState(list)
    }
}
