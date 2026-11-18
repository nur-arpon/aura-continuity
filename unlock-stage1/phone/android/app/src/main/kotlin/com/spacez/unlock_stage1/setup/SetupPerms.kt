package com.spacez.unlock_stage1.setup

import com.spacez.unlock_stage1.BuildConfig

import android.Manifest
import android.app.AppOpsManager
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.PowerManager
import android.os.Process
import android.provider.Settings
import com.google.android.gms.tasks.Tasks
import com.google.android.gms.wearable.CapabilityClient
import com.google.android.gms.wearable.Wearable
import com.spacez.unlock_stage1.handoff.MediaWatch
import com.spacez.unlock_stage1.handoff.PcBubble
import com.spacez.unlock_stage1.watchlink.WatchProtocol
import com.spacez.unlock_stage1.watchlink.WatchTrust
import com.spacez.unlock_stage1.zerotap.AuraAccessibility
import java.util.concurrent.TimeUnit

/** What the "Allow everything" checklist reads and the intent each step opens. Nothing here grants anything itself. */
object SetupPerms {
    val RUNTIME = mapOf(
        "notifications" to arrayOf(Manifest.permission.POST_NOTIFICATIONS),
        "bluetooth" to arrayOf(Manifest.permission.BLUETOOTH_CONNECT, Manifest.permission.BLUETOOTH_ADVERTISE),
        "location" to arrayOf(Manifest.permission.ACCESS_FINE_LOCATION),
    )

    fun available(): List<String> = SetupLogic.ORDER.filter {
        (it != "bubbles" || Build.VERSION.SDK_INT >= 30) &&
            // nothing to ask where the permission does not exist: Bluetooth before Android 12, notifications before 13
            (it != "bluetooth" || Build.VERSION.SDK_INT >= 31) &&
            (it != "notifications" || Build.VERSION.SDK_INT >= 33) &&
            // Play build: no usage access, no accessibility, no battery-exemption prompt (policy cut)
            !(BuildConfig.PLAY && it in setOf("usage", "accessibility", "battery"))
    }

    private fun runtimeGranted(c: Context, id: String) =
        RUNTIME[id]!!.all { com.spacez.unlock_stage1.PermCompat.has(c, it) }

    fun usageGranted(c: Context): Boolean {
        val ops = c.getSystemService(AppOpsManager::class.java) ?: return false
        val mode = if (Build.VERSION.SDK_INT >= 29) ops.unsafeCheckOpNoThrow(AppOpsManager.OPSTR_GET_USAGE_STATS, Process.myUid(), c.packageName)
        else @Suppress("DEPRECATION") ops.checkOpNoThrow(AppOpsManager.OPSTR_GET_USAGE_STATS, Process.myUid(), c.packageName)
        return mode == AppOpsManager.MODE_ALLOWED
    }

    fun accessibilityGranted(c: Context): Boolean {
        val flat = Settings.Secure.getString(c.contentResolver, Settings.Secure.ENABLED_ACCESSIBILITY_SERVICES) ?: return false
        return flat.split(':').any { ComponentName.unflattenFromString(it)?.let { n -> n.packageName == c.packageName && n.className.endsWith("AuraAccessibility") } == true }
    }

    fun granted(c: Context): Map<String, Boolean> = mapOf(
        "notifications" to runtimeGranted(c, "notifications"),
        "bluetooth" to runtimeGranted(c, "bluetooth"),
        "location" to runtimeGranted(c, "location"),
        "usage" to usageGranted(c),
        "listener" to MediaWatch.granted(c),
        "accessibility" to accessibilityGranted(c),
        "battery" to (c.getSystemService(PowerManager::class.java)?.isIgnoringBatteryOptimizations(c.packageName) == true),
        "bubbles" to (Build.VERSION.SDK_INT < 30 || PcBubble.allowedBySystem(c)),
        "overlay" to com.spacez.unlock_stage1.handoff.ChatHead.permitted(c),
    )

    /** The most specific Settings page for a non-runtime step; the caller falls back to [fallback]. */
    fun intent(c: Context, id: String): Intent = when (id) {
        "usage" -> Intent(Settings.ACTION_USAGE_ACCESS_SETTINGS, Uri.parse("package:${c.packageName}"))
        // always the in-app disclosure first (Play prominent disclosure); its button opens the system switch
        "listener" -> Intent(c, com.spacez.unlock_stage1.handoff.MediaAccessActivity::class.java)
        "listener-system" ->
            if (Build.VERSION.SDK_INT >= 30) Intent(Settings.ACTION_NOTIFICATION_LISTENER_DETAIL_SETTINGS)
                .putExtra(Settings.EXTRA_NOTIFICATION_LISTENER_COMPONENT_NAME, MediaWatch.component(c).flattenToString())
            else Intent(Settings.ACTION_NOTIFICATION_LISTENER_SETTINGS)
        "accessibility" ->
            if (Build.VERSION.SDK_INT >= 33) Intent("android.settings.ACCESSIBILITY_DETAILS_SETTINGS")
                .putExtra(Intent.EXTRA_COMPONENT_NAME, ComponentName(c, AuraAccessibility::class.java).flattenToString())
            else Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS)
        "battery" -> Intent(Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS, Uri.parse("package:${c.packageName}"))
        "bubbles" -> PcBubble.settingsIntent(c)
        "overlay" -> com.spacez.unlock_stage1.handoff.ChatHead.permissionIntent(c)
        else -> fallback(c)
    }.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)

    fun fallback(c: Context): Intent =
        Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.parse("package:${c.packageName}")).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)

    /** Blocking (Data Layer calls): call from a worker thread. */
    fun watch(c: Context): Map<String, Any?> {
        val pinned = WatchTrust.pinnedNode(c)
        fun <T> q(f: () -> T): T? = runCatching { f() }.getOrNull()
        val connected = q { Tasks.await(Wearable.getNodeClient(c).connectedNodes, 3, TimeUnit.SECONDS) } ?: emptyList()
        val withApp = q { Tasks.await(Wearable.getCapabilityClient(c).getCapability(WatchProtocol.CAPABILITY_WATCH, CapabilityClient.FILTER_REACHABLE), 3, TimeUnit.SECONDS).nodes }
            ?: emptySet()
        val state = SetupLogic.watchState(connected.size, withApp.size, pinned != null, pinned != null && withApp.any { it.id == pinned })
        val node = withApp.firstOrNull() ?: connected.firstOrNull()
        return mapOf("state" to state, "name" to (node?.displayName ?: ""))
    }

    fun playIntent(c: Context) =
        Intent(Intent.ACTION_VIEW, Uri.parse("market://details?id=${c.packageName}")).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)

    fun playWebIntent(c: Context) =
        Intent(Intent.ACTION_VIEW, Uri.parse("https://play.google.com/store/apps/details?id=${c.packageName}")).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
}
