package com.spacez.unlock_stage1.wear

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.ServiceInfo
import android.hardware.Sensor
import android.hardware.SensorEvent
import android.hardware.SensorEventListener
import android.hardware.SensorManager
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.util.Log
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import com.google.android.gms.wearable.MessageEvent
import com.google.android.gms.wearable.WearableListenerService
import com.spacez.unlock_stage1.watchlink.Kind
import com.spacez.unlock_stage1.watchlink.PcState
import com.spacez.unlock_stage1.watchlink.WatchProtocol
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking

/** Woken by Play services for /cu/watch/... messages only. Holds nothing between messages. */
class PhoneListenerService : WearableListenerService() {
    override fun onMessageReceived(e: MessageEvent) {
        val ctx = applicationContext
        when (e.path) {
            WatchProtocol.PATH_QUERY -> {
                if (e.data.size != WatchProtocol.NONCE_LEN) return
                val s = WatchState.snapshot(ctx, Kind.REPLY, e.data)
                Log.i(TAG, "query from phone -> ${WatchState.describe(s)} ready=${s.readyForAutoUnlock}")
                WatchState.remember(ctx, s)
                // onMessageReceived already runs on a Play services worker thread.
                runBlocking { PhoneLink.sendStatus(ctx, s, e.sourceNodeId) }
            }
            WatchProtocol.PATH_CHALLENGE -> {
                if (e.data.size < WatchProtocol.NONCE_LEN) return
                val nonce = e.data.copyOf(WatchProtocol.NONCE_LEN)
                val label = String(e.data, WatchProtocol.NONCE_LEN, e.data.size - WatchProtocol.NONCE_LEN).take(40)
                Log.i(TAG, "relayed unlock request for '$label'")
                Approvals.show(ctx, nonce, e.sourceNodeId, label)
            }
            WatchProtocol.PATH_PAIR_ACK -> {
                val ok = PhoneLink.pairAckMatches(e.data)
                WatchPrefs(ctx).paired = ok
                Log.i(TAG, "pair ack from phone: ${if (ok) "pinned this watch key" else "key mismatch, not paired"}")
                Surfaces.refresh(ctx)
            }
            WatchProtocol.PATH_PHONE_INPUT_STATE -> PhoneInput.onPhoneState(String(e.data))
            WatchProtocol.PATH_PC_STATE -> {
                val st = PcState.decode(String(e.data).take(200))
                WatchPrefs(ctx).pcState = st
                WatchPrefs(ctx).pcStateMs = System.currentTimeMillis()
                Log.i(TAG, "pc state $st")
                Surfaces.refresh(ctx)
            }
        }
    }
}

/** One-tap Approve / Deny for an unlock request the phone relayed (phone could not auto-approve). */
object Approvals {
    private const val CHANNEL = "cu_approve"
    private const val NOTIF_ID = 7
    const val ACTION = "com.spacez.unlock_stage1.wear.DECIDE"

    fun show(ctx: Context, nonce: ByteArray, node: String, label: String) {
        val nm = ctx.getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(NotificationChannel(CHANNEL, "PC unlock requests", NotificationManager.IMPORTANCE_HIGH))
        fun pi(approve: Boolean) = PendingIntent.getBroadcast(ctx, if (approve) 1 else 2,
            Intent(ctx, ApprovalReceiver::class.java).setAction(ACTION)
                .putExtra("approve", approve).putExtra("nonce", nonce).putExtra("node", node),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        val n = NotificationCompat.Builder(ctx, CHANNEL)
            .setSmallIcon(R.drawable.ic_status)
            .setContentTitle("Unlock ${label.ifBlank { "PC" }}?")
            .setContentText("Tap Approve to unlock with your watch")
            .setColor(0xFF5AA2FF.toInt())
            .setCategory(NotificationCompat.CATEGORY_CALL)
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .setAutoCancel(true)
            .setTimeoutAfter(30_000) // same 30 s window as the phone's Approve
            .addAction(0, "Approve", pi(true))
            .addAction(0, "Deny", pi(false))
            .build()
        runCatching { nm.notify(NOTIF_ID, n) }.onFailure { Log.w(TAG, "cannot post approval: ${it.message}") }
    }

    fun cancel(ctx: Context) = ctx.getSystemService(NotificationManager::class.java).cancel(NOTIF_ID)
}

class ApprovalReceiver : BroadcastReceiver() {
    override fun onReceive(c: Context, i: Intent) {
        if (i.action != Approvals.ACTION) return
        val nonce = i.getByteArrayExtra("nonce")?.takeIf { it.size == WatchProtocol.NONCE_LEN } ?: return
        val node = i.getStringExtra("node") ?: return
        val approve = i.getBooleanExtra("approve", false)
        val ctx = c.applicationContext
        Approvals.cancel(ctx)
        val done = goAsync()
        CoroutineScope(Dispatchers.IO).launch {
            try {
                // The frame carries deviceLocked; the phone refuses an APPROVE from a locked watch.
                val s = WatchState.snapshot(ctx, if (approve) Kind.APPROVE else Kind.DENY, nonce)
                Log.i(TAG, "${if (approve) "APPROVE" else "DENY"} tapped -> ${WatchState.describe(s)}")
                PhoneLink.sendStatus(ctx, s, node)
            } finally { done.finish() }
        }
    }
}

/**
 * Brings the PC listener back without opening the app: after a reboot (BOOT_COMPLETED arrives
 * once the watch is first unlocked) and after an app update. connectedDevice foreground services
 * may still be started from BOOT_COMPLETED on Android 15/16 (only dataSync, camera, media,
 * phoneCall and microphone types are blocked there).
 */
class BootReceiver : BroadcastReceiver() {
    override fun onReceive(c: Context, i: Intent) {
        if (i.action != Intent.ACTION_BOOT_COMPLETED && i.action != Intent.ACTION_MY_PACKAGE_REPLACED) return
        val ctx = c.applicationContext
        PcLinkPrefs(ctx).pairUntil = 0 // a pairing window never survives a reboot / update
        val started = PcLinkService.sync(ctx)
        Log.i(TAG, "${i.action?.substringAfterLast('.')}: pc link wanted=${PcLinkService.wanted(ctx)} started=$started")
        Surfaces.refresh(ctx)
    }
}
