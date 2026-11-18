package com.spacez.unlock_stage1.away

import com.spacez.unlock_stage1.handoff.HandoffLogic

/**
 * Aura Anywhere (item 17), the pure part of the phone side (unit-tested on the JVM):
 * tailnet addresses, the TAILNET message, the "Away mode" status card, the PC's last activity and
 * the remote unlock result. Android calls live in AwayPhone / Tailnet.
 */
object AwayLogic {
    const val PORT = 47140
    const val TAILSCALE_PKG = "com.tailscale.ipn"

    /** 100.64.0.0/10 */
    fun isTailnetV4(a: ByteArray) = a.size == 4 && (a[0].toInt() and 0xff) == 100 && (a[1].toInt() and 0xc0) == 64

    fun isTailnetV4(s: String): Boolean {
        val p = s.split('.')
        if (p.size != 4) return false
        val b = p.map { it.toIntOrNull()?.takeIf { v -> v in 0..255 } ?: return false }
        return isTailnetV4(b.map { it.toByte() }.toByteArray())
    }

    fun hello(ip: String, port: Int = PORT) = HandoffLogic.json(mapOf("ip" to ip, "port" to port))

    /** The PC's TAILNET message -> (ip, port); null unless it is a tailnet address and a sane port. */
    fun parseTailnet(json: String): Pair<String, Int>? {
        val m = HandoffLogic.parseFlat(json)
        val ip = m["ip"] as? String ?: return null
        val port = (m["port"] as? Number)?.toInt() ?: return null
        if (!isTailnetV4(ip) || port !in 1024..65535) return null
        return ip to port
    }

    /**
     * Battery rule: the phone dials the PC over the internet only right after the owner asked for
     * something (open the app / widget / tile, Lock, Unlock, Continue on PC). Background traffic
     * (suggestions, lock hints) stays on Bluetooth.
     */
    class Gate {
        @Volatile private var until = 0L
        fun open(nowMs: Long, forMs: Long) { until = maxOf(until, nowMs + forMs) }
        fun isOpen(nowMs: Long) = nowMs < until
    }

    data class Card(val code: String, val title: String, val text: String, val action: String)

    /** "Away mode: ready / Tailscale off - tap to turn on" (action: install | open | none). */
    fun card(paired: Boolean, installed: Boolean, vpnUp: Boolean, ownIp: String?, pcIp: String?): Card = when {
        !paired -> Card("unpaired", "Pair with your PC first", "Away mode uses the same pairing as unlock.", "none")
        !installed -> Card("missing", "Away mode needs Tailscale", "Install the free Tailscale app, sign in with the same account as on your PC. Aura finds your PC by itself.", "install")
        !vpnUp || ownIp == null -> Card("off", "Tailscale off - tap to turn on", "Away mode works while Tailscale is connected. At home Bluetooth is used anyway.", "open")
        pcIp == null -> Card("no-pc", "Away mode: almost ready", "Open Aura once while you are near your PC, so it can tell this phone its Tailscale address.", "none")
        else -> Card("ready", "Away mode: ready", "Your PC ($pcIp) is reachable from anywhere. End-to-end encrypted by Aura Link.", "none")
    }

    /** What the PC was last doing (its LAST_ACTIVITY answer). */
    data class Last(val s: HandoffLogic.Suggestion, val pcLocked: Boolean, val fresh: Boolean, val recent: List<HandoffLogic.Suggestion> = emptyList(),
                    /** ms since the owner last touched the PC's keyboard / mouse (item 19), null = unknown */
                    val idleMs: Long? = null) {
        /** "YouTube · 12:34" / "example.com" */
        fun subtitle(nowMs: Long): String {
            if (s.app == "youtube") {
                val p = HandoffLogic.positionNow(s.posMs, s.playing, s.atMs, nowMs)
                return if (p != null) "YouTube · ${clock(p)}" else "YouTube"
            }
            return HandoffLogic.host(s.url)
        }
        fun title() = s.title.ifBlank { HandoffLogic.host(s.url) }
        companion object {
            /**
             * [rxMs] = this phone's clock when the answer arrived. With the PC's `sent_ms`, every `at_ms`
             * is moved onto the phone's clock, so a playing video is extrapolated without the clock skew
             * between the two devices (the request latency is then the only error, tens of ms).
             */
            fun parse(json: String, rxMs: Long = 0): Last? {
                val s = HandoffLogic.Suggestion.parse(json) ?: return null
                val m = HandoffLogic.parseFlat(json)
                val sent = (m["sent_ms"] as? Number)?.toLong() ?: 0L
                val shift = if (rxMs > 0 && sent > 0) rxMs - sent else 0L
                fun fix(x: HandoffLogic.Suggestion) = if (shift != 0L && x.atMs > 0) x.copy(atMs = x.atMs + shift) else x
                return Last(fix(s), m["pc_locked"] == true, m["fresh"] == true, HandoffLogic.parseRecent(json).map(::fix), (m["idle_ms"] as? Number)?.toLong())
            }
        }
    }

    /** One "More from your PC" row: title, "YouTube · 12:34" / host, and how long ago. */
    data class Row(val title: String, val sub: String, val app: String, val ago: String)

    fun row(r: HandoffLogic.Suggestion, nowMs: Long): Row {
        val sub = if (r.app == "youtube") {
            val p = HandoffLogic.positionNow(r.posMs, r.playing, r.atMs, nowMs)
            if (p != null) "YouTube · ${clock(p)}" else "YouTube"
        } else HandoffLogic.host(r.url)
        val title = r.title.ifBlank { HandoffLogic.host(r.url) }
        val ago = when {
            r.playing -> "Playing"
            r.atMs <= 0 || nowMs < r.atMs -> ""
            nowMs - r.atMs < 90_000 -> "Just now"
            nowMs - r.atMs < 3_600_000 -> "${(nowMs - r.atMs) / 60_000} min ago"
            else -> "${(nowMs - r.atMs) / 3_600_000} h ago"
        }
        return Row(title, sub, r.app, ago)
    }

    fun clock(ms: Long): String {
        val t = ms / 1000
        val h = t / 3600; val m = (t % 3600) / 60; val s = t % 60
        return if (h > 0) "%d:%02d:%02d".format(h, m, s) else "%d:%02d".format(m, s)
    }

    data class UnlockResult(val ok: Boolean, val why: String)

    fun parseUnlockResult(json: String): UnlockResult? {
        val m = HandoffLogic.parseFlat(json)
        val ok = m["ok"] as? Boolean ?: return null
        return UnlockResult(ok, (m["why"] as? String).orEmpty())
    }

    /** One line for a toast / notification. */
    fun resultText(r: UnlockResult): String = when {
        r.ok && r.why == "already unlocked" -> "Your PC is already unlocked"
        r.ok -> "Your PC is unlocked"
        else -> r.why.ifBlank { "Your PC did not unlock" }
    }
}
