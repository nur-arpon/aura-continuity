package com.spacez.unlock_stage1

import android.app.Activity
import android.bluetooth.BluetoothDevice
import android.companion.AssociationInfo
import android.companion.AssociationRequest
import android.companion.BluetoothDeviceFilter
import android.companion.CompanionDeviceManager
import android.companion.CompanionDeviceService
import android.companion.DevicePresenceEvent
import android.companion.ObservingDevicePresenceRequest
import android.content.Context
import android.content.IntentSender
import android.os.Build
import android.util.Log
import java.util.regex.Pattern

/**
 * "Always ready" without polling: the PC is linked as this app's companion device through the
 * platform CompanionDeviceManager (CDM). That gives three supported things:
 *  1. REQUEST_COMPANION_RUN_IN_BACKGROUND: while associated the system puts the app on the
 *     power-save allowlist, so it may start its foreground service from the background
 *     (adb test hook, boot, update) and is not treated as a sleeping / restricted app.
 *  2. REQUEST_COMPANION_START_FOREGROUND_SERVICES_FROM_BACKGROUND: explicit FGS-start exemption.
 *  3. Device presence observing: when the PC's Bluetooth link to the phone comes up the system
 *     binds [PcPresenceService] by itself (also after a reboot); it (re)starts the listener.
 * See phone/STATUS.md for the Android 16 rule behind each start path.
 */
object PcCompanion {
    private const val TAG = "UnlockStage1"
    private fun p(c: Context) = c.getSharedPreferences("unlock", 0)
    private val MAC = Pattern.compile("^[0-9A-F]{2}(:[0-9A-F]{2}){5}$")

    /** Bluetooth address the paired PC last connected from (learned on RFCOMM, never typed). */
    fun pcAddress(c: Context): String? = p(c).getString("pc_bt_addr", null)

    fun rememberPc(c: Context, addr: String?) {
        val a = addr?.uppercase() ?: return
        if (!MAC.matcher(a).matches() || a == pcAddress(c)) return
        p(c).edit().putString("pc_bt_addr", a).apply()
        UnlockState.log("PC Bluetooth address learned: $a")
    }

    fun supported(c: Context) = Build.VERSION.SDK_INT >= 33 &&
        c.packageManager.hasSystemFeature("android.software.companion_device_setup")

    private fun cdm(c: Context): CompanionDeviceManager? =
        if (supported(c)) c.getSystemService(CompanionDeviceManager::class.java) else null

    /** Our associations; the one for the PC first if the address is known. */
    fun associations(c: Context): List<AssociationInfo> {
        val all = runCatching { cdm(c)?.myAssociations }.getOrNull().orEmpty()
        val pc = pcAddress(c)
        return all.sortedByDescending { it.deviceMacAddress?.toString()?.equals(pc, true) == true }
    }

    fun linked(c: Context) = associations(c).isNotEmpty()

    fun describe(c: Context): String = associations(c).joinToString { "#${it.id} ${it.deviceMacAddress ?: "?"}" }.ifEmpty { "none" }

    /**
     * Ask the system to wake us on the PC's presence. Idempotent and cheap (a binder call);
     * the observing itself is done by the Bluetooth stack, not by this app.
     */
    fun ensureObserving(c: Context) {
        val m = cdm(c) ?: return
        for (a in associations(c)) {
            runCatching {
                if (Build.VERSION.SDK_INT >= 36) {
                    m.startObservingDevicePresence(ObservingDevicePresenceRequest.Builder().setAssociationId(a.id).build())
                } else {
                    @Suppress("DEPRECATION")
                    a.deviceMacAddress?.toString()?.let { m.startObservingDevicePresence(it) }
                }
            }.onSuccess { UnlockState.log("companion #${a.id}: observing PC presence") }
             .onFailure { UnlockState.log("companion #${a.id}: observe failed: ${it.message}") }
        }
    }

    /**
     * Shows the system "link this device" sheet (one tap by the user). Bonded devices that
     * match the filter are offered without the PC having to be discoverable.
     */
    fun associate(act: Activity, done: (String) -> Unit) {
        val m = cdm(act) ?: return done("Companion linking needs Android 13 or later")
        val filter = BluetoothDeviceFilter.Builder().apply {
            pcAddress(act)?.let { setAddress(it) }
        }.build()
        val req = AssociationRequest.Builder().addDeviceFilter(filter).setSingleDevice(pcAddress(act) != null).build()
        m.associate(req, act.mainExecutor, object : CompanionDeviceManager.Callback() {
            override fun onAssociationPending(sender: IntentSender) {
                runCatching { act.startIntentSenderForResult(sender, 42, null, 0, 0, 0) }
                    .onFailure { done("could not open the system sheet: ${it.message}") }
            }
            override fun onAssociationCreated(info: AssociationInfo) {
                UnlockState.log("companion linked: #${info.id} ${info.deviceMacAddress}")
                ensureObserving(act)
                if (Settings.enabled(act)) UnlockService.start(act, "companion linked")
                done("linked")
            }
            override fun onFailure(error: CharSequence?) { done("not linked: $error") }
        })
    }
}

/**
 * Bound by the system (BIND_COMPANION_DEVICE_SERVICE) when the associated PC appears, e.g.
 * its Bluetooth link to the phone comes up. While the system holds this binding the app may
 * start its foreground service. We only (re)start the listener; we do not stop it when the
 * PC goes away (the idle listener costs nothing, and it must be up before the PC asks).
 */
class PcPresenceService : CompanionDeviceService() {
    private fun kick(why: String) {
        Log.i("UnlockStage1", "presence: $why")
        if (UnlockCrypto.pcId(this) == null || !Settings.enabled(this)) return
        // after an app update the fresh process has no head yet: the presence binding brings it back by itself
        runCatching { com.spacez.unlock_stage1.handoff.ChatHead.attach(applicationContext) }
        if (UnlockState.running) { UnlockState.log("presence: $why (listener already up)"); return }
        UnlockService.start(this, "PC presence: $why")
    }

    override fun onDevicePresenceEvent(event: DevicePresenceEvent) {
        when (event.event) {
            DevicePresenceEvent.EVENT_BT_CONNECTED, DevicePresenceEvent.EVENT_BLE_APPEARED -> PcState.setNear(this, true)
            DevicePresenceEvent.EVENT_BT_DISCONNECTED, DevicePresenceEvent.EVENT_BLE_DISAPPEARED -> PcState.setNear(this, false)
        }
        val name = when (event.event) {
            DevicePresenceEvent.EVENT_BLE_APPEARED -> "BLE appeared"
            DevicePresenceEvent.EVENT_BLE_DISAPPEARED -> "BLE disappeared"
            DevicePresenceEvent.EVENT_BT_CONNECTED -> "BT connected"
            DevicePresenceEvent.EVENT_BT_DISCONNECTED -> "BT disconnected"
            else -> "event ${event.event}"
        }
        kick("$name (#${event.associationId})")
    }

    @Deprecated("API 33-35 path; API 36 calls onDevicePresenceEvent")
    override fun onDeviceAppeared(info: AssociationInfo) { PcState.setNear(this, true); kick("appeared #${info.id}") }

    @Deprecated("API 33-35 path; API 36 calls onDevicePresenceEvent")
    override fun onDeviceDisappeared(info: AssociationInfo) { PcState.setNear(this, false) }
}
