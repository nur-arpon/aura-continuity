package com.spacez.unlock_stage1.wear

import kotlin.math.hypot

/**
 * Two-finger tap = right click (FUTURE-PLANS item 9). Wear Compose has no multi-pointer tap detector, so the touchpad's
 * pointer loop feeds every pointer event here: [update] with the pointers currently down (id -> x,y) and the time.
 * A tap is: a second finger goes down, and both lift again within [maxMs] of the second one landing, with neither
 * finger having travelled more than [slopPx] from where it landed. Pure Kotlin so it can be unit tested.
 */
class TwoFingerTap(private val maxMs: Long = 250, private val slopPx: Float = 24f) {
    private var startMs = -1L
    private val start = HashMap<Long, Pair<Float, Float>>()
    private var armed = false
    private var sawTwo = false

    /** True while two or more fingers are down, so the one-finger drag / tap handlers can stand down. */
    val active get() = sawTwo

    /** @return true exactly once when the last of the two fingers lifts and the whole thing counts as a two-finger tap. */
    fun update(down: Map<Long, Pair<Float, Float>>, nowMs: Long): Boolean {
        if (down.size >= 2) {
            if (!sawTwo) { sawTwo = true; armed = true; startMs = nowMs; start.clear(); start.putAll(down) }
            else if (armed) {
                if (nowMs - startMs > maxMs) armed = false
                for ((id, p) in down) { val s = start[id] ?: continue; if (hypot(p.first - s.first, p.second - s.second) > slopPx) armed = false }
                if (down.size > 2) armed = false
            }
            return false
        }
        if (!sawTwo) return false
        val ok = armed && down.isEmpty() && nowMs - startMs <= maxMs
        if (down.isEmpty()) { sawTwo = false; armed = false; start.clear() }
        else if (armed && nowMs - startMs > maxMs) armed = false
        return ok
    }
}
