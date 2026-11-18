package com.spacez.unlock_stage1.wear

import android.app.PendingIntent
import android.content.Intent
import android.graphics.drawable.Icon
import androidx.wear.watchface.complications.data.ComplicationData
import androidx.wear.watchface.complications.data.ComplicationType
import androidx.wear.watchface.complications.data.MonochromaticImage
import androidx.wear.watchface.complications.data.MonochromaticImageComplicationData
import androidx.wear.watchface.complications.data.PlainComplicationText
import androidx.wear.watchface.complications.data.ShortTextComplicationData
import androidx.wear.watchface.complications.datasource.ComplicationRequest
import androidx.wear.watchface.complications.datasource.SuspendingComplicationDataSourceService

/**
 * Watch-face complication: PC state at a glance (lock glyph + "Locked" / "Open" / "PC").
 * UPDATE_PERIOD 0: rebuilt only when [Surfaces.refresh] pushes a change. Tap opens the app.
 */
class PcComplicationService : SuspendingComplicationDataSourceService() {
    override suspend fun onComplicationRequest(request: ComplicationRequest): ComplicationData? =
        build(request.complicationType, StatusText.pc(this))

    override fun getPreviewData(type: ComplicationType): ComplicationData? =
        build(type, PcSummary(PcLook.LOCKED, "Locked", "", 0))

    private fun build(type: ComplicationType, s: PcSummary): ComplicationData? {
        val iconRes = when (s.look) { PcLook.UNLOCKED -> R.drawable.ic_lock_open; PcLook.LOCKED -> R.drawable.ic_lock; else -> R.drawable.ic_laptop }
        val image = MonochromaticImage.Builder(Icon.createWithResource(this, iconRes)).build()
        val word = when (s.look) { PcLook.LOCKED -> "Locked"; PcLook.UNLOCKED -> "Open"; PcLook.NOT_PAIRED -> "Pair"; PcLook.UNREACHABLE -> "Away"; PcLook.IDLE -> "PC" }
        val desc = PlainComplicationText.Builder("PC ${s.title}").build()
        val tap = PendingIntent.getActivity(this, 3, Intent(this, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE)
        return when (type) {
            ComplicationType.SHORT_TEXT -> ShortTextComplicationData.Builder(PlainComplicationText.Builder(word).build(), desc)
                .setMonochromaticImage(image).setTapAction(tap).build()
            ComplicationType.MONOCHROMATIC_IMAGE -> MonochromaticImageComplicationData.Builder(image, desc).setTapAction(tap).build()
            else -> null
        }
    }
}
