package com.spacez.unlock_stage1.wear

import android.content.Context
import android.util.Log
import androidx.wear.protolayout.ActionBuilders
import androidx.wear.protolayout.LayoutElementBuilders
import androidx.wear.protolayout.ResourceBuilders
import androidx.wear.protolayout.TimelineBuilders
import androidx.wear.protolayout.material3.ColorScheme
import androidx.wear.protolayout.material3.button
import androidx.wear.protolayout.material3.icon
import androidx.wear.protolayout.material3.materialScope
import androidx.wear.protolayout.material3.primaryLayout
import androidx.wear.protolayout.material3.text
import androidx.wear.protolayout.material3.textEdgeButton
import androidx.wear.protolayout.material3.titleCard
import androidx.wear.protolayout.modifiers.clickable
import androidx.wear.protolayout.modifiers.loadAction
import androidx.wear.protolayout.types.LayoutColor
import androidx.wear.protolayout.types.layoutString
import androidx.wear.tiles.RequestBuilders
import androidx.wear.tiles.TileBuilders
import com.google.android.horologist.tiles.SuspendingTileService
import com.spacez.unlock_stage1.watchlink.Kind
import com.spacez.unlock_stage1.watchlink.WatchProtocol
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch

/**
 * Tile: PC state + a big Lock PC edge button. Freshness 0: rebuilt only when something asks
 * (a state change via [Surfaces.refresh], a tap, the user swiping to it). No timer updates.
 */
@OptIn(com.google.android.horologist.annotations.ExperimentalHorologistApi::class)
class StatusTileService : SuspendingTileService() {

    override suspend fun tileRequest(requestParams: RequestBuilders.TileRequest): TileBuilders.Tile {
        val ctx = applicationContext
        when (requestParams.currentState.lastClickableId) {
            "lock" -> if (!locking) {
                locking = true
                Feedback.play(ctx, Feedback.Cue.LOCK)
                work.launch {
                    val r = LockAction.run(ctx)
                    lastResult = r; lastResultMs = System.currentTimeMillis(); locking = false
                    Feedback.play(ctx, if (r == LockResult.DIRECT || r == LockResult.VIA_PHONE) Feedback.Cue.SUCCESS else Feedback.Cue.ERROR)
                    Surfaces.refresh(ctx)
                }
            }
            "unlock" -> if (!unlocking) {
                unlocking = true
                Feedback.play(ctx, Feedback.Cue.TAP)
                work.launch {
                    val r = UnlockAction.run(ctx)
                    lastUnlock = r; lastResultMs = System.currentTimeMillis(); unlocking = false
                    Feedback.play(ctx, if (r == UnlockResult.SENT) Feedback.Cue.APPROVED else Feedback.Cue.ERROR)
                    Surfaces.refresh(ctx)
                }
            }
            "" -> work.launch {
                // refresh the wrist / lock line while the user looks at the tile (one sensor read);
                // remember() asks for a rebuild only if the line changed, so this cannot loop.
                val s = WatchState.snapshot(ctx, Kind.PUSH, ByteArray(WatchProtocol.NONCE_LEN), sign = false)
                if (!WatchState.remember(ctx, s)) Log.d(TAG, "tile state unchanged")
            }
        }
        return TileBuilders.Tile.Builder()
            .setResourcesVersion(RES_VERSION)
            .setFreshnessIntervalMillis(0)
            .setTileTimeline(TimelineBuilders.Timeline.fromLayoutElement(layout(ctx, requestParams.deviceConfiguration)))
            .build()
    }

    override suspend fun resourcesRequest(requestParams: RequestBuilders.ResourcesRequest): ResourceBuilders.Resources = resources()

    internal fun resources(): ResourceBuilders.Resources {
        val b = ResourceBuilders.Resources.Builder().setVersion(RES_VERSION)
        for ((id, res) in ICONS) b.addIdToImageMapping(id, ResourceBuilders.ImageResource.Builder()
            .setAndroidResourceByResId(ResourceBuilders.AndroidImageResourceByResId.Builder().setResourceId(res).build()).build())
        return b.build()
    }

    internal fun layout(ctx: Context, device: androidx.wear.protolayout.DeviceParametersBuilders.DeviceParameters): LayoutElementBuilders.LayoutElement {
        val lp = PcLinkPrefs(ctx)
        val wp = WatchPrefs(ctx)
        val s = StatusText.pc(ctx)
        val recent = lastResult != null && System.currentTimeMillis() - lastResultMs < 8_000
        val recentUnlock = lastUnlock != null && System.currentTimeMillis() - lastResultMs < 8_000
        val title = when {
            locking -> "Locking…"
            unlocking -> "Approving…"
            recentUnlock && lastUnlock == UnlockResult.SENT -> "Approved"
            recentUnlock && lastUnlock == UnlockResult.NOT_READY -> "Wear + unlock watch"
            recentUnlock -> "Out of reach"
            recent && (lastResult == LockResult.DIRECT || lastResult == LockResult.VIA_PHONE) -> "PC locked"
            recent && lastResult == LockResult.FAILED -> "Out of reach"
            else -> if (s.look == PcLook.NOT_PAIRED) "Not paired" else "PC ${s.title.lowercase()}"
        }
        val detail = when {
            locking || unlocking -> "Sending to ${lp.pcName.ifBlank { "your PC" }}"
            s.look == PcLook.NOT_PAIRED -> "Open the app to pair"
            else -> s.detail
        }
        val watchLine = when {
            wp.testActive -> "Test mode on"
            !lp.paired -> ""
            !lp.enabled -> ctx.getString(R.string.feature_unlock) + " off"
            wp.last.endsWith("Ready") -> ctx.getString(R.string.feature_unlock) + " ready"
            wp.last.isNotBlank() -> wp.last.substringBefore(" · Ready")
            else -> ctx.getString(R.string.feature_unlock) + " on"
        }
        val iconId = when {
            locking || s.look == PcLook.LOCKED -> "lock"
            s.look == PcLook.UNLOCKED -> "lock_open"
            else -> "laptop"
        }
        val open = clickable(
            ActionBuilders.LaunchAction.Builder().setAndroidActivity(
                ActionBuilders.AndroidActivity.Builder().setPackageName(ctx.packageName).setClassName(MainActivity::class.java.name).build()
            ).build(), "open")
        val paired = lp.paired || wp.paired

        return materialScope(ctx, device, allowDynamicTheme = false, defaultColorScheme = SCHEME) {
            primaryLayout(
                titleSlot = { text((lp.pcName.ifBlank { ctx.getString(R.string.app_name) }).layoutString) },
                mainSlot = {
                  if (lp.paired) androidx.wear.protolayout.LayoutElementBuilders.Column.Builder()
                    .setWidth(androidx.wear.protolayout.DimensionBuilders.expand())
                    .addContent(titleCard(
                        onClick = open,
                        title = { text(title.layoutString) },
                        content = { text(detail.layoutString, maxLines = 1) },
                        time = { icon(iconId, tintColor = LayoutColor(Palette.PRIMARY)) },
                        colors = with(androidx.wear.protolayout.material3.CardDefaults) { filledTonalCardColors() },
                    ))
                    .addContent(androidx.wear.protolayout.LayoutElementBuilders.Spacer.Builder().setHeight(androidx.wear.protolayout.DimensionBuilders.dp(4f)).build())
                    .addContent(button(
                        onClick = clickable(loadAction(), "unlock"),
                        labelContent = { text("Unlock PC".layoutString) },
                        iconContent = { icon("lock_open") },
                        width = androidx.wear.protolayout.DimensionBuilders.expand(),
                        colors = with(androidx.wear.protolayout.material3.ButtonDefaults) { filledTonalButtonColors() },
                    ))
                    .build()
                  else titleCard(
                        onClick = open,
                        title = { text(title.layoutString) },
                        content = { text((if (watchLine.isBlank()) detail else "$detail\n$watchLine").layoutString, maxLines = 3) },
                        time = { icon(iconId, tintColor = LayoutColor(Palette.PRIMARY)) },
                        colors = with(androidx.wear.protolayout.material3.CardDefaults) { filledTonalCardColors() },
                    )
                },
                bottomSlot = {
                    if (paired) textEdgeButton(onClick = clickable(loadAction(), "lock")) { text("Lock PC".layoutString) }
                    else textEdgeButton(onClick = open) { text("Pair".layoutString) }
                },
            )
        }
    }

    companion object {
        private const val RES_VERSION = "2"
        private val ICONS = mapOf("lock" to R.drawable.ic_lock, "lock_open" to R.drawable.ic_lock_open, "laptop" to R.drawable.ic_laptop)
        private val work = CoroutineScope(SupervisorJob() + Dispatchers.IO)
        @Volatile private var locking = false
        @Volatile private var lastResult: LockResult? = null
        @Volatile private var lastResultMs = 0L
        @Volatile private var unlocking = false
        @Volatile private var lastUnlock: UnlockResult? = null

        private fun c(argb: Int) = LayoutColor(argb)
        private val SCHEME = ColorScheme(
            primary = c(Palette.PRIMARY), primaryDim = c(0xFF3F86E6.toInt()), primaryContainer = c(Palette.PRIMARY_DEEP),
            onPrimary = c(0xFF04152E.toInt()), onPrimaryContainer = c(0xFFDCEBFF.toInt()),
            secondary = c(0xFF9DBBE6.toInt()), secondaryDim = c(0xFF7F9DC8.toInt()), secondaryContainer = c(Palette.NAVY_750),
            onSecondary = c(Palette.NAVY_900), onSecondaryContainer = c(0xFFD6E4F8.toInt()),
            tertiary = c(Palette.SKY), tertiaryDim = c(0xFF5FA6E6.toInt()), tertiaryContainer = c(0xFF173F66.toInt()),
            onTertiary = c(0xFF04152E.toInt()), onTertiaryContainer = c(0xFFD8ECFF.toInt()),
            surfaceContainerLow = c(0xFF0D1A2F.toInt()), surfaceContainer = c(0xFF132542.toInt()), surfaceContainerHigh = c(0xFF1A3053.toInt()),
            onSurface = c(Palette.TEXT), onSurfaceVariant = c(Palette.TEXT_2),
            outline = c(Palette.BLUE_500), outlineVariant = c(Palette.NAVY_700),
            background = c(Palette.NAVY_950), onBackground = c(Palette.TEXT),
            error = c(0xFFA9CCFF.toInt()), errorDim = c(0xFF8DB4EE.toInt()), errorContainer = c(0xFF2A3F66.toInt()),
            onError = c(0xFF071830.toInt()), onErrorContainer = c(0xFFDCE9FF.toInt()),
        )
    }
}
