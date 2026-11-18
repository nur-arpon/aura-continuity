package com.spacez.unlock_stage1.handoff

import kotlin.math.PI
import kotlin.math.atan2
import kotlin.math.cos
import kotlin.math.exp
import kotlin.math.sin

/** Chat-head tree, pure part (no Android types, JVM-tested in ChatHeadLogicTest). */
object ChatHeadLogic {
    /** Laptop apps shown (two rings: 5 + 8). */
    const val MAX_APPS = 13
    /** Tabs shown for a browser. */
    const val MAX_TABS = 8
    /** The head stays through a dropped link (Bluetooth / tailnet come and go) this long. */
    const val LINK_GRACE_MS = 15 * 60_000L
    /** A dismissed head stays away this long. */
    const val SNOOZE_MS = 30 * 60_000L

    data class Tab(val title: String, val url: String?, val active: Boolean)
    data class LApp(val id: String, val name: String, val browser: Boolean, val title: String, val url: String?, val posMs: Long?, val tabs: List<Tab>)

    /** What a tap on an app bubble does. */
    enum class AppAction { TABS, OPEN_URL, OPEN_LAPTOP }

    fun appAction(a: LApp): AppAction = when {
        a.browser && a.tabs.any { it.url != null } -> AppAction.TABS
        a.browser && a.url != null -> AppAction.OPEN_URL
        else -> AppAction.OPEN_LAPTOP
    }

    /** The tabs a level shows: only openable ones (they have a link), active first (the PC already ordered them). */
    fun visibleTabs(a: LApp): List<Tab> = a.tabs.filter { it.url != null }.take(MAX_TABS)

    /** The position a tab opens at: only the active YouTube tab knows one. */
    fun posFor(a: LApp, t: Tab): Long? = if (t.active && t.url != null && HandoffLogic.isYouTube(t.url)) a.posMs else null

    /**
     * Offsets (dx, dy) of [n] bubbles from the origin centre, on an arc of radius [r] pointing to the middle of the
     * screen: left when the origin is on the right half. [step] radians apart.
     */
    fun arc(n: Int, r: Float, towardsLeft: Boolean, step: Float = 0.74f): List<Pair<Float, Float>> {
        val base = if (towardsLeft) PI else 0.0
        val s = minOf(step, 2.4f / maxOf(1, n))
        return List(n) { i ->
            val a = base + (i - (n - 1) / 2.0) * s
            (r * cos(a)).toFloat() to (r * sin(a)).toFloat()
        }
    }

    /**
     * Offsets (dx, dy) of [n] bubbles on rings around the origin: ring 1 (radius [r0]) holds as many as fit [gap] apart
     * on an arc of [span] radians, the rest go on bigger rings ([dr] further out). The fan points to the middle of the
     * screen: sideways, tilted up or down by [tilt] (-1..1, see [tilt]) when the origin is near the top or bottom edge.
     */
    fun rings(n: Int, r0: Float, dr: Float, gap: Float, towardsLeft: Boolean, tilt: Float = 0f, span: Float = 2.8f): List<Pair<Float, Float>> {
        // worked out facing right, mirrored for the right edge; each ring's arc is kept inside the half facing away from the
        // edge (a tilted arc used to swing past straight up/down, and those bubbles piled up on the screen edge)
        val base = atan2(tilt.toDouble() * 0.9, 1.0)
        val out = ArrayList<Pair<Float, Float>>(n)
        var left = n
        var r = r0
        while (left > 0) {
            val cap = maxOf(1, (span / (gap / r)).toInt() + 1)
            val c = minOf(left, cap)
            val step = if (c > 1) minOf(1.15f * gap / r, span / (c - 1)) else 0f
            val half = (c - 1) / 2.0 * step
            val lim = (Math.PI / 2 - 0.12 - half).coerceAtLeast(0.0)
            val b = base.coerceIn(-lim, lim)
            for (i in 0 until c) {
                val a = b + (i - (c - 1) / 2.0) * step
                val x = (r * cos(a)).toFloat()
                out.add((if (towardsLeft) -x else x) to (r * sin(a)).toFloat())
            }
            left -= c
            r += dr
        }
        return out
    }

    /** -1..1: how far the fan should tilt to the screen's middle for an origin at height [cy] of [h]. */
    fun tilt(cy: Float, h: Float): Float = (((h / 2f - cy) / h) * 2.4f).coerceIn(-1f, 1f)

    /** Keep a bubble centre on screen, with room for its label below. */
    fun clampCentre(x: Float, y: Float, w: Int, h: Int, half: Float, labelRoom: Float): Pair<Float, Float> =
        x.coerceIn(half + 4, w - half - 4) to y.coerceIn(half + 4, h - half - labelRoom - 4)

    fun shortTitle(t: String, max: Int = 16): String {
        val s = t.trim().replace(Regex("\\s+"), " ")
        return if (s.length <= max) s else s.take(max - 1).trimEnd() + "…"
    }

    fun letter(s: String): String = s.trim().firstOrNull { it.isLetterOrDigit() }?.uppercase() ?: "?"

    fun hostOf(url: String?): String = HandoffLogic.host(url ?: "")

    /** Hue 0..359 for a letter chip. */
    fun hue(s: String): Float {
        var h = 0
        for (c in s) h = h * 31 + c.code
        return (((h % 360) + 360) % 360).toFloat()
    }

    /** The head is seen only when everything allows it. Quiet = a call or a full-screen app (video, game). */
    fun linkOk(linkUpNow: Boolean, lastUpMs: Long, now: Long): Boolean = linkUpNow || (lastUpMs > 0 && now - lastUpMs <= LINK_GRACE_MS)

    /** [linkUp] = the link is up now or was within [LINK_GRACE_MS] (see [linkOk]). */
    fun shouldShow(enabled: Boolean, permitted: Boolean, linkUp: Boolean, haveApps: Boolean, now: Long, dismissedUntil: Long, fullscreen: Boolean, inCall: Boolean, locked: Boolean): Boolean =
        enabled && permitted && linkUp && haveApps && now >= dismissedUntil && !fullscreen && !inCall && !locked

    /** Damped spring for animations: overshoots, then settles (value 0 -> 1). */
    fun spring(t: Float, damping: Float = 7.5f, freq: Float = 11f): Float {
        if (t >= 1f) return 1f
        return (1.0 - exp(-damping * t.toDouble()) * cos(freq * t.toDouble())).toFloat()
    }
}
