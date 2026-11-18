package com.spacez.unlock_stage1.zerotap

import android.app.AppOpsManager
import android.app.usage.UsageEvents
import android.app.usage.UsageStatsManager
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.os.Handler
import android.os.Looper
import android.os.Process

/**
 * Zero-tap continue without the accessibility service (item 19): Usage access tells which app
 * came to the front. Android has no push callback for that, so this is a SHORT, bounded watch
 * that only starts on an OS event (screen unlocked / screen on): one look per second for 20 s.
 * With the accessibility service on, this is not used at all (its window events are the trigger).
 */
object Foreground {
    fun granted(c: Context): Boolean {
        val ops = c.getSystemService(AppOpsManager::class.java)
        return ops.unsafeCheckOpNoThrow(AppOpsManager.OPSTR_GET_USAGE_STATS, Process.myUid(), c.packageName) == AppOpsManager.MODE_ALLOWED
    }

    private val main = Handler(Looper.getMainLooper())
    private var receiver: BroadcastReceiver? = null
    private var until = 0L
    private var since = 0L
    private var lastPkg: String? = null
    private var ctx: Context? = null

    /** Called from the Aura service start: listens for unlock / screen-on (OS broadcasts). */
    fun start(c: Context) {
        if (receiver != null) return
        val app = c.applicationContext
        val r = object : BroadcastReceiver() {
            override fun onReceive(ctx: Context, i: Intent) = window(app)
        }
        receiver = r
        app.registerReceiver(r, IntentFilter().apply { addAction(Intent.ACTION_USER_PRESENT); addAction(Intent.ACTION_SCREEN_ON) })
    }

    private val tick = object : Runnable {
        override fun run() {
            val c = ctx ?: return
            look(c)
            if (System.currentTimeMillis() < until) main.postDelayed(this, 1_000)
        }
    }

    private fun window(c: Context) {
        if (!ZeroTap.pcToPhone(c) || AuraAccessibility.on(c) || !granted(c)) return
        ctx = c
        val now = System.currentTimeMillis()
        since = now - 2_000
        until = now + 20_000
        main.removeCallbacks(tick)
        main.post(tick)
    }

    private fun look(c: Context) {
        val usm = c.getSystemService(UsageStatsManager::class.java) ?: return
        val now = System.currentTimeMillis()
        val ev = usm.queryEvents(since, now)
        val e = UsageEvents.Event()
        var top: String? = null
        while (ev.hasNextEvent()) {
            ev.getNextEvent(e)
            if (e.eventType == UsageEvents.Event.ACTIVITY_RESUMED) top = e.packageName
        }
        since = now
        if (top != null && top != lastPkg) {
            lastPkg = top
            if (top in ContinueRules.WATCHED) ZeroTap.onForeground(c, top, "usage")
        }
    }
}
