package com.spacez.unlock_stage1.handoff

import android.content.Context
import android.os.Handler
import android.os.Looper
import com.spacez.aura.link.Message
import com.spacez.unlock_stage1.AuraPhone
import com.spacez.unlock_stage1.UnlockState

/**
 * Item 26: pushes the phone's recent list to the PC over Aura Link ("More from your phone" in the PC tray + bubble).
 * Event-driven only: [note] is called by ZeroTap.pushState when the phone reports something (media session / app
 * content); a changed list settles for [SETTLE_MS], then goes out once (min [GAP_MS] between pushes). Never polled,
 * never dials: with the link down the list simply waits (and goes out with the PC's next "aura:state?" ask).
 */
object RecentPush {
    private const val SETTLE_MS = 1500L
    private const val GAP_MS = 4000L
    private val main by lazy { Handler(Looper.getMainLooper()) }
    private val ring by lazy { RecentLogic.Ring() }
    @Volatile private var loaded = false
    @Volatile private var ctx: Context? = null
    private var lastSentSig: String? = null
    private var lastSentAt = 0L
    private val flush = Runnable { send(false) }

    private fun p(c: Context) = c.getSharedPreferences("recent_push", 0)
    private fun now() = System.currentTimeMillis()

    private fun load(c: Context) {
        if (loaded) return
        synchronized(ring) {
            if (loaded) return
            RecentLogic.deserialize(p(c).getString("list", null)).reversed().forEach { ring.note(it) }
            loaded = true
        }
    }

    /** A state report (the map ZeroTap pushes to the PC) happened. */
    fun note(c: Context, m: Map<String, Any?>) {
        ctx = c.applicationContext
        val e = RecentLogic.fromState(m, now()) ?: return
        load(c)
        val changed = synchronized(ring) { ring.note(e) }
        runCatching { p(c).edit().putString("list", RecentLogic.serialize(synchronized(ring) { ring.list() })).apply() }
        if (!changed) return
        main.removeCallbacks(flush)
        main.postDelayed(flush, maxOf(SETTLE_MS, lastSentAt + GAP_MS - now()))
    }

    /** The PC asked what the phone has ("aura:state?"): send the list now, changed or not. */
    fun pushNow(c: Context) {
        ctx = c.applicationContext
        load(c)
        main.post { send(true) }
    }

    private fun send(force: Boolean) {
        val c = ctx ?: return
        if (!com.spacez.unlock_stage1.zerotap.ZeroTap.phoneToPc(c)) return
        val l = AuraPhone.link ?: return
        if (!l.connected) return // never dial for this
        runCatching { AppsPush.send(c, force) }
        val list = synchronized(ring) { ring.list() }
        if (list.isEmpty()) return
        val sig = RecentLogic.signature(list)
        if (!force && sig == lastSentSig) return
        lastSentSig = sig; lastSentAt = now()
        l.send(Message.activity(RecentLogic.toJson(list, now())))
        UnlockState.log("recent list -> PC: ${list.size} items")
    }
}
