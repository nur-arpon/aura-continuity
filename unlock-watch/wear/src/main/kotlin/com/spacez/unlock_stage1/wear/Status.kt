package com.spacez.unlock_stage1.wear

import android.content.ComponentName
import android.content.Context
import android.text.format.DateFormat
import android.text.format.DateUtils
import androidx.wear.tiles.TileService
import androidx.wear.watchface.complications.datasource.ComplicationDataSourceUpdateRequester

/** Asks the tile and the complication to rebuild. Event-driven only (called on a state change). */
object Surfaces {
    fun refresh(ctx: Context) {
        val app = ctx.applicationContext
        runCatching { TileService.getUpdater(app).requestUpdate(StatusTileService::class.java) }
        runCatching {
            ComplicationDataSourceUpdateRequester.create(app, ComponentName(app, PcComplicationService::class.java)).requestUpdateAll()
        }
    }
}

enum class PcLook { NOT_PAIRED, LOCKED, UNLOCKED, IDLE, UNREACHABLE }

/** What the watch can honestly say about the PC: it cannot see the PC's lock screen, so this is
 *  the last thing that happened between them (or what the phone last reported), with a time. */
data class PcSummary(val look: PcLook, val title: String, val lead: String, val atMs: Long) {
    /** The status line, with a time that is worked out when it is read: "By this watch · 2 h ago". Empty text once it is stale. */
    val detail: String get() = detailAt(System.currentTimeMillis())
    fun detailAt(now: Long): String {
        if (atMs <= 0) return lead
        val ago = StatusText.ago(atMs, now) ?: return ""
        return if (lead.isEmpty()) ago else "$lead · $ago"
    }
}

object StatusText {
    /** "Just now", "12 min ago", "2 h ago", "Yesterday"; null when older than that (the line is hidden). Pure and unit-tested. */
    fun ago(atMs: Long, now: Long): String? {
        if (atMs <= 0) return null
        val d = now - atMs
        return when {
            d < 0 -> "Just now"
            d < 60_000L -> "Just now"
            d < 3_600_000L -> "${d / 60_000L} min ago"
            d < 24 * 3_600_000L -> "${d / 3_600_000L} h ago"
            d < 48 * 3_600_000L -> "Yesterday"
            else -> null
        }
    }

    fun clock(ctx: Context, ms: Long): String =
        if (ms <= 0) "" else {
            val day = java.util.Calendar.getInstance().apply { timeInMillis = ms }
            val now = java.util.Calendar.getInstance()
            val sameDay = day.get(java.util.Calendar.YEAR) == now.get(java.util.Calendar.YEAR) &&
                day.get(java.util.Calendar.DAY_OF_YEAR) == now.get(java.util.Calendar.DAY_OF_YEAR)
            if (sameDay) DateFormat.getTimeFormat(ctx).format(ms)
            else DateUtils.formatDateTime(ctx, ms, DateUtils.FORMAT_SHOW_DATE or DateUtils.FORMAT_ABBREV_MONTH or DateUtils.FORMAT_NO_YEAR)
        }

    fun pc(ctx: Context): PcSummary {
        val lp = PcLinkPrefs(ctx)
        val wp = WatchPrefs(ctx)
        if (!lp.paired && !wp.paired) return PcSummary(PcLook.NOT_PAIRED, "Not paired", "Pair with your PC to start", 0)
        val evMs = wp.pcEventMs
        val st = wp.pcState
        val stMs = wp.pcStateMs
        if (st != null && stMs > evMs) {
                        return when {
                !st.connected -> PcSummary(PcLook.UNREACHABLE, "Not connected", "Phone can't see the PC", stMs)
                st.locked == true -> PcSummary(PcLook.LOCKED, "Locked", "Reported by phone", stMs)
                st.locked == false -> PcSummary(PcLook.UNLOCKED, "Unlocked", "Reported by phone", stMs)
                else -> PcSummary(PcLook.IDLE, "Connected", "Via phone", stMs)
            }
        }
        return when (wp.pcEvent) {
            PcEvent.UNLOCK_APPROVED -> PcSummary(PcLook.UNLOCKED, "Unlocked", "By this watch", evMs)
            PcEvent.UNLOCK_PUSHED -> PcSummary(PcLook.UNLOCKED, "Unlock sent", "Approved from this watch", evMs)
            PcEvent.UNLOCK_REFUSED -> PcSummary(PcLook.LOCKED, "Locked", "Unlock declined", evMs)
            PcEvent.LOCK_SENT -> PcSummary(PcLook.LOCKED, "Locked", "From this watch", evMs)
            PcEvent.LOCK_VIA_PHONE -> PcSummary(PcLook.LOCKED, "Locked", "Sent via phone", evMs)
            PcEvent.LOCK_FAILED -> PcSummary(PcLook.UNREACHABLE, "Out of reach", "Lock didn't arrive", evMs)
            PcEvent.PAIRED -> PcSummary(PcLook.IDLE, "Paired", "Waiting for the PC", evMs)
            null -> PcSummary(PcLook.IDLE, "Ready", if (lp.paired) "No activity yet" else "Paired via phone", 0)
        }
    }

    /** "A1B2-C3D4-E5F6" -> "A1B2 · C3D4 · E5F6" */
    fun code(fp: String) = fp.replace("-", " · ")
}
