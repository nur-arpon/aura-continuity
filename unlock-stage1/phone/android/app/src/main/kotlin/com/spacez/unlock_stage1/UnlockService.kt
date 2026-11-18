package com.spacez.unlock_stage1

import android.Manifest
import android.annotation.SuppressLint
import android.app.KeyguardManager
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.bluetooth.*
import android.bluetooth.le.AdvertiseCallback
import android.bluetooth.le.AdvertiseData
import android.bluetooth.le.AdvertiseSettings
import android.bluetooth.le.BluetoothLeAdvertiser
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import android.os.Handler
import android.os.HandlerThread
import android.os.IBinder
import android.os.Looper
import android.os.ParcelUuid
import android.util.Log
import com.spacez.unlock_stage1.watchlink.WatchGate
import com.spacez.unlock_stage1.watchlink.WatchTrust
import java.io.DataInputStream
import java.io.DataOutputStream
import java.io.InputStream
import java.io.OutputStream
import java.net.Inet4Address
import java.net.InetAddress
import java.net.InetSocketAddress
import java.net.NetworkInterface
import java.net.ServerSocket
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference
import javax.crypto.Mac

/** Shared state between the service, the approval screen and the Flutter UI. */
object UnlockState {
    @Volatile var running = false
    @Volatile var advertising = false
    @Volatile var gattUp = false
    @Volatile var rfcommUp = false
    @Volatile var tetherAddr: String? = null
    @Volatile var loopbackUp = false
    @Volatile var answered = 0
    @Volatile var restarts = 0
    @Volatile var lastUserPresentMs = 0L
    val log = ArrayDeque<String>()
    val pending = AtomicReference<Pending?>(null)

    class Pending(val c: UnlockCrypto.Challenge, val transport: String, val at: Long, val level: Int, val reply: (ByteArray) -> Unit)

    @Synchronized fun log(s: String) {
        Log.i("UnlockStage1", s)
        log.addFirst("%tT %s".format(System.currentTimeMillis(), s))
        while (log.size > 80) log.removeLast()
    }
    @Synchronized fun logLines(): List<String> = log.toList()
}

/** User settings. Nothing runs unless the user turned the listener on. */
object Settings {
    private fun p(c: Context) = c.getSharedPreferences("unlock", 0)
    fun enabled(c: Context) = p(c).getBoolean("enabled", false)
    fun setEnabled(c: Context, on: Boolean) = p(c).edit().putBoolean("enabled", on).apply()
    /** auto | tap | fingerprint. Default auto (falls back to fingerprint when auto is not allowed). */
    fun mode(c: Context) = p(c).getString("mode", "auto") ?: "auto"
    fun setMode(c: Context, m: String) = p(c).edit().putString("mode", m).apply()
    /** Stage-1 TEST MODE: answers at TAP level without a tap. Expires by itself. */
    fun testModeOn(c: Context) = System.currentTimeMillis() < p(c).getLong("test_until", 0)
    fun setTestMode(c: Context, on: Boolean) =
        p(c).edit().putLong("test_until", if (on) System.currentTimeMillis() + TEST_MODE_MS else 0).apply()
    const val TEST_MODE_MS = 30 * 60_000L
}

class UnlockService : Service() {
    companion object {
        val SERVICE_UUID: UUID = UUID.fromString("7c1e0001-5a1f-4e6b-9d2a-3c0b1a2f4e01")
        val CHAL_UUID: UUID = UUID.fromString("7c1e0002-5a1f-4e6b-9d2a-3c0b1a2f4e01")
        val RESP_UUID: UUID = UUID.fromString("7c1e0003-5a1f-4e6b-9d2a-3c0b1a2f4e01")
        val RFCOMM_UUID: UUID = UUID.fromString("7c1e0010-5a1f-4e6b-9d2a-3c0b1a2f4e01")
        val CCCD: UUID = UUID.fromString("00002902-0000-1000-8000-00805f9b34fb")
        const val TCP_PORT = 47821
        const val CH_SERVICE = "unlock_service"
        const val CH_APPROVE = "unlock_approve"
        const val NOTIF_SERVICE = 1
        const val NOTIF_APPROVE = 2
        const val ACTION_APPROVE = "approve"
        const val ACTION_DENY = "deny"
        const val APPROVE_WINDOW_MS = 30_000L
        /** max allowed by AdvertiseSettings.setTimeout */
        const val ADV_WINDOW_MS = 180_000

        /**
         * Start the listener as a connectedDevice foreground service. Allowed from the
         * background only on the exempt paths listed in phone/STATUS.md; otherwise Android
         * throws ForegroundServiceStartNotAllowedException, which is logged (not retried).
         */
        fun start(ctx: Context, why: String = "app"): Boolean = runCatching {
            // not paired / switched off: nothing to run (starting it anyway made a fresh install crash on the
            // foreground-service rule); the chat head is attached directly instead
            if (!Settings.enabled(ctx) || UnlockCrypto.pcId(ctx) == null) {
                com.spacez.unlock_stage1.handoff.ChatHead.attach(ctx)
                return@runCatching false
            }
            ctx.startForegroundService(Intent(ctx, UnlockService::class.java).putExtra("why", why)); true
        }.getOrElse {
            UnlockState.log("start refused ($why): ${it.javaClass.simpleName}: ${it.message}")
            false
        }

        /** Finish the pending request with a ready Mac (fingerprint) or at its own level (tap / test). */
        fun answer(level: Int, mac: Mac? = null) {
            val p = UnlockState.pending.getAndSet(null) ?: return
            val m = mac ?: UnlockCrypto.mac(if (level == UnlockCrypto.DENY) "0" else "2")
            if (m == null) { UnlockState.log("cannot sign: not paired"); return }
            val r = runCatching { UnlockCrypto.buildResponse(m, p.c, level, System.currentTimeMillis()) }
                .onFailure { UnlockState.log("sign failed: $it") }.getOrNull() ?: return
            p.reply(r)
            UnlockState.answered++
            UnlockState.log("answered ${levelName(level)} via ${p.transport} after ${System.currentTimeMillis() - p.at} ms")
        }

        fun levelName(l: Int) = when (l) { 0 -> "deny"; 1 -> "auto"; 2 -> "tap"; 3 -> "fingerprint"; else -> "?" }
    }

    private val main = Handler(Looper.getMainLooper())
    private val bgThread = HandlerThread("unlock-bg").apply { start() }
    private val bg = Handler(bgThread.looper)
    private var gatt: BluetoothGattServer? = null
    private var respChar: BluetoothGattCharacteristic? = null
    private var advertiser: BluetoothLeAdvertiser? = null
    private var advBackoffMs = 1000L
    private var rfcommBackoffMs = 1000L
    private val mtu = ConcurrentHashMap<String, Int>()
    private val writeBuf = ConcurrentHashMap<String, ByteArray>()
    private val lastResp = ConcurrentHashMap<String, ByteArray>()
    private var rfcommThread: Thread? = null
    private var rfcommServer: BluetoothServerSocket? = null
    private var loopbackServer: ServerSocket? = null
    private var tetherServer: ServerSocket? = null
    private val seenNonces = ArrayDeque<String>()
    @Volatile private var stopping = false

    private val btManager get() = getSystemService(BluetoothManager::class.java)
    private val adapter: BluetoothAdapter? get() = btManager?.adapter

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        val nm = getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(NotificationChannel(CH_SERVICE, "Unlock listener", NotificationManager.IMPORTANCE_MIN))
        nm.createNotificationChannel(NotificationChannel(CH_APPROVE, "Unlock requests", NotificationManager.IMPORTANCE_HIGH))
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_APPROVE -> { answer(UnlockCrypto.TAP); cancelApproveNotif(); return START_NOT_STICKY }
            ACTION_DENY -> { answer(UnlockCrypto.DENY); cancelApproveNotif(); return START_NOT_STICKY }
        }
        com.spacez.unlock_stage1.handoff.ChatHead.attach(this)
        if (!Settings.enabled(this) || UnlockCrypto.pcId(this) == null) {
            // never run work the user did not ask for - but a service started with startForegroundService MUST call
            // startForeground before stopping, or Android kills the app ("Aura keeps stopping" on a fresh install)
            runCatching { startForeground(NOTIF_SERVICE, serviceNotif(), ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE) }
            runCatching { stopForeground(STOP_FOREGROUND_REMOVE) }
            stopSelf(); return START_NOT_STICKY
        }
        val ok = runCatching {
            startForeground(NOTIF_SERVICE, serviceNotif(), ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE)
        }.onFailure { UnlockState.log("startForeground refused: ${it.javaClass.simpleName}: ${it.message}") }.isSuccess
        if (!ok) { stopSelf(); return START_NOT_STICKY }
        val why = intent?.getStringExtra("why") ?: "system restart (START_STICKY)"
        if (!UnlockState.running) {
            UnlockState.running = true
            stopping = false
            registerReceiver(events, IntentFilter().apply {
                addAction(BluetoothAdapter.ACTION_STATE_CHANGED)
                addAction(Intent.ACTION_USER_PRESENT)
                addAction("android.hardware.usb.action.USB_STATE")
            })
            UnlockState.log("listener started by $why, mode ${Settings.mode(this)} (crypto self-test ${if (UnlockCrypto.selfTest()) "ok" else "FAILED"})")
            startAll()
            bg.post { PcCompanion.ensureObserving(this) }
        }
        return START_STICKY
    }

    override fun onDestroy() {
        stopping = true
        UnlockState.running = false
        main.removeCallbacksAndMessages(null)
        bg.removeCallbacksAndMessages(null)
        runCatching { unregisterReceiver(events) }
        stopAll()
        bgThread.quitSafely()
        UnlockState.log("listener stopped")
        super.onDestroy()
    }

    private fun hasPerms() = listOf(Manifest.permission.BLUETOOTH_CONNECT, Manifest.permission.BLUETOOTH_ADVERTISE)
        .all { PermCompat.has(this, it) }

    private fun startAll() {
        // item 19: the Usage-access fallback for zero-tap continue (only acts without the accessibility service)
        runCatching { com.spacez.unlock_stage1.zerotap.Foreground.start(applicationContext) }
        bg.post { refreshTether(); if (Settings.testModeOn(this)) startLoopback() }
        // Aura Link + Away mode do not need Bluetooth (item 17); the RFCOMM listener starts only when it is on
        AuraPhone.start(this)
        if (!hasPerms()) { UnlockState.log("Bluetooth permissions missing"); return }
        if (adapter?.isEnabled != true) { UnlockState.log("Bluetooth is off: BLE / RFCOMM wait for it"); return }
        startGatt()
        advertiseWindow("listener start")
        startRfcomm()
        AuraPhone.start(this)
    }

    private fun stopAll() {
        stopBluetooth()
        AuraPhone.stopAll(this)
        runCatching { loopbackServer?.close() }; loopbackServer = null; UnlockState.loopbackUp = false
        runCatching { tetherServer?.close() }; tetherServer = null; UnlockState.tetherAddr = null
    }

    @SuppressLint("MissingPermission")
    private fun stopBluetooth() {
        runCatching { advertiser?.stopAdvertising(advCallback) }
        advertiser = null; UnlockState.advertising = false
        runCatching { gatt?.close() }
        gatt = null; UnlockState.gattUp = false
        runCatching { rfcommServer?.close() }
        rfcommServer = null; UnlockState.rfcommUp = false
        AuraPhone.stopServer()
    }

    // ------------------------------------------------------------ self-heal: events only, no polling

    private val events = object : BroadcastReceiver() {
        override fun onReceive(c: Context, i: Intent) {
            when (i.action) {
                BluetoothAdapter.ACTION_STATE_CHANGED -> when (i.getIntExtra(BluetoothAdapter.EXTRA_STATE, -1)) {
                    BluetoothAdapter.STATE_ON -> { UnlockState.log("Bluetooth on: restarting BLE + RFCOMM"); main.postDelayed({ startAll() }, 500) }
                    BluetoothAdapter.STATE_TURNING_OFF, BluetoothAdapter.STATE_OFF -> { UnlockState.log("Bluetooth off"); stopBluetooth() }
                }
                Intent.ACTION_USER_PRESENT -> { UnlockState.lastUserPresentMs = System.currentTimeMillis(); healthCheck("unlock"); advertiseWindow("phone unlocked") }
                // cable plugged / unplugged / function changed: re-check the tether interface a few
                // times while it comes up (no periodic polling)
                "android.hardware.usb.action.USB_STATE" -> for (d in longArrayOf(0, 1500, 4000, 8000)) bg.postDelayed({ refreshTether() }, d)
            }
        }
    }

    /** Cheap check, run only on events (phone unlocked, BT on), never on a timer. */
    private fun healthCheck(why: String) {
        if (adapter?.isEnabled != true || !hasPerms()) return
        if (gatt == null) { UnlockState.restarts++; UnlockState.log("heal ($why): GATT server reopened"); startGatt() }
        if (rfcommThread?.isAlive != true) { UnlockState.restarts++; UnlockState.log("heal ($why): RFCOMM restarted"); startRfcomm() }
        AuraPhone.start(this)
    }

    // ------------------------------------------------------------ approval policy (all transports)

    private fun keyguardLocked() = getSystemService(KeyguardManager::class.java).isDeviceLocked

    private fun onChallenge(bytes: ByteArray, transport: String, peer: String? = null, reply: (ByteArray) -> Unit) {
        val c = UnlockCrypto.parseChallenge(bytes)
        if (c == null) { UnlockState.log("bad challenge on $transport (${bytes.size} bytes, need protocol v2)"); return }
        val myPc = UnlockCrypto.pcId(this)
        if (myPc == null || !myPc.contentEquals(c.pcId)) { UnlockState.log("challenge from an unknown PC on $transport"); return }
        // remember where our PC connects from: the filter for the companion link sheet
        PcCompanion.rememberPc(this, peer)
        val skew = System.currentTimeMillis() - c.issuedMs
        if (kotlin.math.abs(skew) > 120_000) { UnlockState.log("challenge clock skew $skew ms on $transport, ignored"); return }
        val n = c.nonce.toHex()
        synchronized(seenNonces) {
            if (seenNonces.contains(n)) { UnlockState.log("repeated nonce on $transport, ignored"); return }
            seenNonces.addLast(n); while (seenNonces.size > 256) seenNonces.removeFirst()
        }
        val mode = Settings.mode(this)
        // HANDS_FREE flag (0x80, MAC'd): the PC is racing several legs (phone RFCOMM / BLE,
        // watch). Answer AUTO if allowed, else say NOT_NOW and do NOT prompt: the PC decides
        // whether to escalate (it will not if the watch already approved).
        val handsFree = (c.minLevel and UnlockCrypto.HANDS_FREE) != 0
        val minL = c.minLevel and 0x7f
        val plan = ApprovalPlan.steps(mode, minL, handsFree, Settings.testModeOn(this), WatchTrust.pinnedKey(this) != null)
        for (step in plan) when (step) {
            // AUTO: the Keystore releases the AUTO key only if the phone is unlocked or was
            // unlocked by the user (PIN / fingerprint) within the window.
            ApprovalPlan.Step.PHONE_AUTO -> {
                val m = UnlockCrypto.autoMac()
                val r = m?.let { runCatching { UnlockCrypto.buildResponse(it, c, UnlockCrypto.AUTO, System.currentTimeMillis()) }.getOrNull() }
                if (r != null) {
                    reply(r); UnlockState.answered++
                    UnlockState.log("answered auto via $transport (locked=${keyguardLocked()})")
                    return
                }
                UnlockState.log("auto not allowed (phone locked and not unlocked in the last ${UnlockCrypto.AUTO_WINDOW_S / 60} min)")
            }
            // WATCH: signed "on wrist + unlocked" from the paired watch, bound to this challenge's
            // nonce, replaces the tap (owner decision 2026-10-20, no-lockout rule 8 as amended).
            ApprovalPlan.Step.WATCH -> {
                val t0 = System.currentTimeMillis()
                val v = WatchGate.check(this, ApprovalPlan.WATCH_TIMEOUT_MS, challengeNonce = c.nonce)
                if (ApprovalPlan.watchApproves(v)) {
                    val m = UnlockCrypto.mac("2")
                    val r = m?.let { runCatching { UnlockCrypto.buildResponse(it, c, ApprovalPlan.WATCH_LEVEL, System.currentTimeMillis()) }.getOrNull() }
                    if (r != null) {
                        reply(r); UnlockState.answered++
                        UnlockState.log("answered by watch (on wrist + unlocked, no tap) via $transport in ${System.currentTimeMillis() - t0} ms")
                        return
                    }
                    UnlockState.log("watch allowed but the tap key could not sign (not paired?)")
                } else {
                    UnlockState.log("watch did not approve: $v (${System.currentTimeMillis() - t0} ms)")
                }
            }
            // stage-1 TEST MODE: TAP level without a tap (expires by itself)
            ApprovalPlan.Step.TEST_TAP -> {
                UnlockState.pending.set(UnlockState.Pending(c, transport, System.currentTimeMillis(), UnlockCrypto.TAP, reply))
                answer(UnlockCrypto.TAP)
                return
            }
            ApprovalPlan.Step.NOT_NOW -> {
                reply(UnlockCrypto.NOT_NOW_FRAME)
                UnlockState.log("hands-free not possible via $transport: told the PC NOT_NOW (no prompt)")
                return
            }
            ApprovalPlan.Step.PROMPT -> { /* below */ }
        }
        // TAP or FINGERPRINT: tell the PC to wait for the user, then ask the user
        val level = if (mode == "tap" && minL <= UnlockCrypto.TAP) UnlockCrypto.TAP else UnlockCrypto.FINGERPRINT
        reply(UnlockCrypto.WAITING_FRAME)
        UnlockState.pending.set(UnlockState.Pending(c, transport, System.currentTimeMillis(), level, reply))
        UnlockState.log("unlock request via $transport, waiting for ${levelName(level)}")
        showApproveNotif(transport, level)
        main.postDelayed({
            UnlockState.pending.get()?.let { if (System.currentTimeMillis() - it.at >= APPROVE_WINDOW_MS) { UnlockState.pending.set(null); cancelApproveNotif() } }
        }, APPROVE_WINDOW_MS)
    }

    // ------------------------------------------------------------ BLE GATT server

    @SuppressLint("MissingPermission")
    private fun startGatt() {
        if (gatt != null) return
        val g = btManager?.openGattServer(this, gattCallback) ?: run { UnlockState.log("openGattServer failed"); return }
        val svc = BluetoothGattService(SERVICE_UUID, BluetoothGattService.SERVICE_TYPE_PRIMARY)
        val chal = BluetoothGattCharacteristic(CHAL_UUID, BluetoothGattCharacteristic.PROPERTY_WRITE, BluetoothGattCharacteristic.PERMISSION_WRITE)
        val resp = BluetoothGattCharacteristic(RESP_UUID,
            BluetoothGattCharacteristic.PROPERTY_READ or BluetoothGattCharacteristic.PROPERTY_NOTIFY,
            BluetoothGattCharacteristic.PERMISSION_READ)
        resp.addDescriptor(BluetoothGattDescriptor(CCCD, BluetoothGattDescriptor.PERMISSION_READ or BluetoothGattDescriptor.PERMISSION_WRITE))
        svc.addCharacteristic(chal)
        svc.addCharacteristic(resp)
        g.addService(svc)
        gatt = g; respChar = resp; UnlockState.gattUp = true
        UnlockState.log("GATT server up")
    }

    @SuppressLint("MissingPermission")
    private fun bleReply(device: BluetoothDevice, r: ByteArray) {
        val g = gatt ?: return
        val ch = respChar ?: return
        val payload = if (r.size <= 2 || (mtu[device.address] ?: 23) - 3 >= r.size) r else { lastResp[device.address] = r; UnlockCrypto.READY_FRAME }
        val rc = g.notifyCharacteristicChanged(device, ch, false, payload)
        if (rc != BluetoothStatusCodes.SUCCESS) UnlockState.log("notify failed rc=$rc")
    }

    private val gattCallback = object : BluetoothGattServerCallback() {
        override fun onConnectionStateChange(device: BluetoothDevice, status: Int, newState: Int) {
            if (newState == BluetoothProfile.STATE_DISCONNECTED) {
                mtu.remove(device.address); writeBuf.remove(device.address); lastResp.remove(device.address)
            }
        }
        override fun onMtuChanged(device: BluetoothDevice, m: Int) { mtu[device.address] = m }

        @SuppressLint("MissingPermission")
        override fun onCharacteristicWriteRequest(device: BluetoothDevice, requestId: Int, ch: BluetoothGattCharacteristic,
                                                  prepared: Boolean, responseNeeded: Boolean, offset: Int, value: ByteArray) {
            if (prepared) {
                val old = writeBuf[device.address] ?: ByteArray(0)
                val nb = old.copyOf(maxOf(old.size, offset + value.size))
                System.arraycopy(value, 0, nb, offset, value.size)
                writeBuf[device.address] = nb
            }
            if (responseNeeded) gatt?.sendResponse(device, requestId, BluetoothGatt.GATT_SUCCESS, offset, value)
            // off the binder thread: the WATCH step can wait up to ApprovalPlan.WATCH_TIMEOUT_MS
            if (!prepared && ch.uuid == CHAL_UUID) bg.post { onChallenge(value, "ble") { bleReply(device, it) } }
        }

        @SuppressLint("MissingPermission")
        override fun onExecuteWrite(device: BluetoothDevice, requestId: Int, execute: Boolean) {
            gatt?.sendResponse(device, requestId, BluetoothGatt.GATT_SUCCESS, 0, null)
            val b = writeBuf.remove(device.address)
            if (execute && b != null) bg.post { onChallenge(b, "ble") { bleReply(device, it) } }
        }

        @SuppressLint("MissingPermission")
        override fun onCharacteristicReadRequest(device: BluetoothDevice, requestId: Int, offset: Int, ch: BluetoothGattCharacteristic) {
            val v = lastResp[device.address] ?: ByteArray(0)
            val part = if (offset < v.size) v.copyOfRange(offset, v.size) else ByteArray(0)
            gatt?.sendResponse(device, requestId, BluetoothGatt.GATT_SUCCESS, offset, part)
        }

        @SuppressLint("MissingPermission")
        override fun onDescriptorWriteRequest(device: BluetoothDevice, requestId: Int, d: BluetoothGattDescriptor,
                                              prepared: Boolean, responseNeeded: Boolean, offset: Int, value: ByteArray) {
            if (responseNeeded) gatt?.sendResponse(device, requestId, BluetoothGatt.GATT_SUCCESS, offset, value)
        }
    }

    // ------------------------------------------------------------ BLE advertising
    /*
     * Found live 2026-10-19: this phone is bonded to the PC, so Windows resolves our
     * advertisement to the bonded LE device and uses its GATT cache. When this GATT service
     * is added while the PC is already connected, Windows can keep a stale cache without it
     * until Bluetooth restarts. A non-resolvable advertising address would avoid that, but
     * setOwnAddressType is a system API. The PC retries discovery and falls back to RFCOMM.
     *
     * Battery: LOW_POWER mode (~1 s interval). The advertising is done by the Bluetooth
     * controller; the app is not woken for it.
     */
    private val advCallback = object : AdvertiseCallback() {
        override fun onStartSuccess(settingsInEffect: AdvertiseSettings?) {
            UnlockState.advertising = true; UnlockState.log("advertising (low power)")
        }
        override fun onStartFailure(errorCode: Int) {
            UnlockState.advertising = false
            advertiser = null
            // no retry loop: the next unlock event opens the next window
            UnlockState.log("advertise failed ($errorCode)")
        }
    }

    /**
     * BLE advertising only in a short window after the user unlocks the phone (Apple-style):
     * the bonded RFCOMM path needs no advertising at all, BLE is only an RSSI bonus. The
     * controller stops it by itself at the timeout (hardware-offloaded, CPU asleep).
     */
    @SuppressLint("MissingPermission")
    private fun advertiseWindow(why: String) {
        if (stopping) return
        runCatching { advertiser?.stopAdvertising(advCallback) }
        advertiser = null
        startAdvertising()
        UnlockState.log("BLE window ${ADV_WINDOW_MS / 1000} s ($why)")
        main.removeCallbacks(advEnded)
        main.postDelayed(advEnded, ADV_WINDOW_MS + 1000L)
    }
    private val advEnded = Runnable { advertiser = null; UnlockState.advertising = false }

    @SuppressLint("MissingPermission")
    private fun startAdvertising() {
        if (advertiser != null || stopping) return
        val a = adapter?.bluetoothLeAdvertiser ?: run { UnlockState.log("no BLE advertiser (Bluetooth off?)"); return }
        val settings = AdvertiseSettings.Builder()
            .setAdvertiseMode(AdvertiseSettings.ADVERTISE_MODE_LOW_POWER)
            .setTxPowerLevel(AdvertiseSettings.ADVERTISE_TX_POWER_MEDIUM)
            .setConnectable(true).setTimeout(ADV_WINDOW_MS).build()
        val data = AdvertiseData.Builder().addServiceUuid(ParcelUuid(SERVICE_UUID)).setIncludeDeviceName(false).build()
        advertiser = a
        a.startAdvertising(settings, data, advCallback)
    }

    // ------------------------------------------------------------ classic Bluetooth RFCOMM
    // Idle cost is ~zero: the phone is already page-scannable for its paired devices; the
    // accept() thread just blocks.

    @SuppressLint("MissingPermission")
    private fun startRfcomm() {
        if (rfcommThread?.isAlive == true || stopping) return
        val ad = adapter ?: return
        rfcommThread = Thread({
            var ok = false
            try {
                val server = ad.listenUsingRfcommWithServiceRecord("PC Unlock stage 1", RFCOMM_UUID)
                rfcommServer = server; UnlockState.rfcommUp = true
                UnlockState.log("RFCOMM listening")
                rfcommBackoffMs = 1000; ok = true
                while (true) {
                    val s = server.accept() ?: break
                    val peer = runCatching { s.remoteDevice.address }.getOrNull()
                    Thread { serveStream(s.inputStream, s.outputStream, "rfcomm", peer) { runCatching { s.close() } } }.start()
                }
            } catch (t: Throwable) {
                if (!stopping) UnlockState.log("RFCOMM listener ended: ${t.message}")
            } finally {
                UnlockState.rfcommUp = false
                // restart with backoff while Bluetooth is on (event-driven, not polling)
                if (!stopping && adapter?.isEnabled == true) {
                    val d = if (ok) 1000 else rfcommBackoffMs
                    rfcommBackoffMs = minOf(rfcommBackoffMs * 2, 60_000)
                    main.postDelayed({ startRfcomm() }, d)
                }
            }
        }, "rfcomm-accept").apply { isDaemon = true; start() }
    }

    /** Length-prefixed frames both ways. Shared by RFCOMM and TCP. */
    private fun serveStream(ins: InputStream, outs: OutputStream, transport: String, peer: String? = null, close: () -> Unit) {
        try {
            val din = DataInputStream(ins)
            val n = din.readUnsignedShort()
            if (n > 256) return
            val buf = ByteArray(n); din.readFully(buf)
            val dout = DataOutputStream(outs)
            var mine: UnlockState.Pending? = null
            onChallenge(buf, transport, peer) { frame ->
                synchronized(dout) { runCatching { dout.writeShort(frame.size); dout.write(frame); dout.flush() } }
            }
            mine = UnlockState.pending.get()?.takeIf { it.transport == transport && it.c.nonce.contentEquals(buf.copyOfRange(2, 18)) }
            // Block until the PC hangs up (it does once it has decided). If our prompt is still
            // open then, another leg (e.g. the watch) won or the PC gave up: withdraw the prompt.
            runCatching { ins.read() }
            if (mine != null && UnlockState.pending.compareAndSet(mine, null)) {
                cancelApproveNotif()
                UnlockState.log("PC hung up: prompt withdrawn")
            }
        } catch (t: Throwable) {
            UnlockState.log("$transport session error: ${t.message}")
        } finally {
            close()
        }
    }

    // ------------------------------------------------------------ TCP: USB tethering link (+ loopback for the adb test)

    private fun startLoopback() {
        if (loopbackServer?.isClosed == false) return
        runCatching {
            val s = ServerSocket(); s.reuseAddress = true
            s.bind(InetSocketAddress(InetAddress.getLoopbackAddress(), TCP_PORT), 4)
            loopbackServer = s; UnlockState.loopbackUp = true
            acceptLoop(s, "usb-adb")
        }.onFailure { UnlockState.log("loopback listen failed: ${it.message}") }
    }

    private fun tetherAddress(): Inet4Address? = runCatching {
        NetworkInterface.getNetworkInterfaces()?.toList().orEmpty()
            .filter { it.isUp && (it.name.startsWith("rndis") || it.name.startsWith("ncm") || it.name.startsWith("usb")) }
            .flatMap { it.inetAddresses.toList() }.filterIsInstance<Inet4Address>().firstOrNull()
    }.getOrNull()

    private fun refreshTether() {
        if (stopping) return
        val a = tetherAddress()
        val cur = tetherServer
        if (a == null) {
            if (cur != null) { runCatching { cur.close() }; tetherServer = null; UnlockState.tetherAddr = null; UnlockState.log("USB link gone") }
            return
        }
        if (cur != null && !cur.isClosed && cur.inetAddress == a) return
        runCatching { cur?.close() }
        runCatching {
            val s = ServerSocket(); s.reuseAddress = true
            s.bind(InetSocketAddress(a, TCP_PORT), 4)
            tetherServer = s; UnlockState.tetherAddr = a.hostAddress
            UnlockState.log("USB link listening on ${a.hostAddress}:$TCP_PORT")
            acceptLoop(s, "usb")
        }.onFailure { UnlockState.log("USB link listen failed: ${it.message}") }
    }

    private fun acceptLoop(s: ServerSocket, transport: String) {
        Thread({
            try {
                while (!s.isClosed) {
                    val c = s.accept()
                    c.tcpNoDelay = true; c.soTimeout = (APPROVE_WINDOW_MS + 4000).toInt()
                    Thread { serveStream(c.getInputStream(), c.getOutputStream(), transport) { runCatching { c.close() } } }.start()
                }
            } catch (_: Throwable) {
            } finally {
                if (transport == "usb-adb") UnlockState.loopbackUp = false
            }
        }, "$transport-accept").apply { isDaemon = true; start() }
    }

    // ------------------------------------------------------------ notifications

    private fun serviceNotif(): Notification {
        val open = PendingIntent.getActivity(this, 0, Intent(this, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE)
        return Notification.Builder(this, CH_SERVICE)
            .setSmallIcon(R.drawable.ic_aura_glyph)
            .setContentTitle("Aura is ready")
            .setContentText("Answers only your paired PC. No polling, near-zero battery.")
            .setContentIntent(open).setOngoing(true).build()
    }

    private fun showApproveNotif(transport: String, level: Int) {
        val b = Notification.Builder(this, CH_APPROVE)
            .setSmallIcon(R.drawable.ic_aura_unlock)
            .setContentTitle("Unlock your PC?")
            .setCategory(Notification.CATEGORY_CALL)
            .setTimeoutAfter(APPROVE_WINDOW_MS)
            .setAutoCancel(true)
        if (level == UnlockCrypto.FINGERPRINT) {
            val open = PendingIntent.getActivity(this, 3, Intent(this, ApproveActivity::class.java)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK), PendingIntent.FLAG_IMMUTABLE)
            b.setContentText("Request over $transport. Tap, then touch the fingerprint sensor.")
                .setContentIntent(open).apply { if (!BuildConfig.PLAY) setFullScreenIntent(open, true) }
        } else {
            fun action(a: String, code: Int) = PendingIntent.getService(this, code,
                Intent(this, UnlockService::class.java).setAction(a), PendingIntent.FLAG_IMMUTABLE)
            b.setContentText("Request over $transport. Approve within 30 s.")
                .addAction(Notification.Action.Builder(null, "Approve", action(ACTION_APPROVE, 1)).build())
                .addAction(Notification.Action.Builder(null, "Deny", action(ACTION_DENY, 2)).build())
        }
        getSystemService(NotificationManager::class.java).notify(NOTIF_APPROVE, b.build())
    }

    private fun cancelApproveNotif() = getSystemService(NotificationManager::class.java).cancel(NOTIF_APPROVE)
}

/**
 * BOOT_COMPLETED and MY_PACKAGE_REPLACED are both on Android's list of broadcasts that may
 * start a foreground service from the background; connectedDevice is not one of the types
 * Android 15+ forbids from BOOT_COMPLETED. Only if the user left the listener on.
 */
class BootReceiver : BroadcastReceiver() {
    override fun onReceive(c: Context, i: Intent) {
        Log.i("UnlockStage1", "boot receiver: ${i.action}")
        if (UnlockCrypto.pcId(c) == null || !Settings.enabled(c)) return
        PcCompanion.ensureObserving(c)
        UnlockService.start(c, i.action?.substringAfterLast('.') ?: "boot")
    }
}

/**
 * adb test hooks (no screen taps). Protected by android.permission.DUMP: only the shell /
 * system hold it, so other apps cannot use it.
 *   adb shell am broadcast -n com.spacez.unlock_stage1/.TestReceiver --es cmd start|stop|status|testmode_on|testmode_off|mode_auto|mode_tap|mode_fingerprint
 */
class TestReceiver : BroadcastReceiver() {
    override fun onReceive(c: Context, i: Intent) {
        val ctx = c.applicationContext
        val cmd = i.getStringExtra("cmd")
        when (cmd) {
            "start" -> { Settings.setEnabled(ctx, true); UnlockService.start(ctx, "adb test hook") }
            "observe" -> PcCompanion.ensureObserving(ctx)
            "stop" -> { Settings.setEnabled(ctx, false); ctx.stopService(Intent(ctx, UnlockService::class.java)) }
            "testmode_on" -> Settings.setTestMode(ctx, true)
            "testmode_off" -> Settings.setTestMode(ctx, false)
            "mode_auto", "mode_tap", "mode_fingerprint" -> Settings.setMode(ctx, cmd!!.removePrefix("mode_"))
            "aura_test_on" -> AuraPhone.setTest(ctx, true)
            "aura_test_off" -> AuraPhone.setTest(ctx, false)
            "aura_text" -> AuraPhone.link?.send(com.spacez.aura.link.Message.text(i.getStringExtra("text") ?: "hello from the phone"))
            "aura_status" -> UnlockState.log("aura: ${AuraPhone.status()} test=${AuraPhone.testOn(ctx)}")
            // item 17 (Aura Anywhere): status / fetch / Lock PC over whatever pipe connects. Remote UNLOCK has no
            // adb hook on purpose: it needs a finger on the sensor (RemoteUnlockActivity).
            "away_status" -> UnlockState.log("away: ${com.spacez.unlock_stage1.away.AwayPhone.status(ctx)}")
            "away_fetch" -> { com.spacez.unlock_stage1.away.AwayPhone.userAction(); kotlin.concurrent.thread { UnlockState.log("away fetch: ${com.spacez.unlock_stage1.away.AwayPhone.fetch(ctx, "adb")}") } }
            "away_lock" -> kotlin.concurrent.thread { UnlockState.log("away lock: ${com.spacez.unlock_stage1.away.AwayPhone.lockPc(ctx)}") }
            // item 19: simulate "this app came to the front" (the accessibility / usage trigger), and the state
            "zt_fg" -> com.spacez.unlock_stage1.zerotap.ZeroTap.onForeground(ctx, i.getStringExtra("pkg") ?: "app.morphe.android.youtube", "adb")
            "zt_status" -> UnlockState.log("zero-tap: ${com.spacez.unlock_stage1.zerotap.ZeroTap.status(ctx)}")
            "away_continue" -> kotlin.concurrent.thread { UnlockState.log("away continue: ${com.spacez.unlock_stage1.away.AwayPhone.continueFromLaptop(ctx)}") }
            // item 12 Handoff switches (adb only; the app UI has the same toggles)
            "clip_sync_on", "clip_sync_off" -> com.spacez.unlock_stage1.handoff.HandoffPhone.setClipSync(ctx, cmd == "clip_sync_on")
            "suggest_on", "suggest_off" -> com.spacez.unlock_stage1.handoff.HandoffPhone.setSuggest(ctx, cmd == "suggest_on")
            "handoff_status" -> UnlockState.log("handoff: clipSync=${com.spacez.unlock_stage1.handoff.HandoffPhone.clipSync(ctx)} suggest=${com.spacez.unlock_stage1.handoff.HandoffPhone.suggest(ctx)} media=${com.spacez.unlock_stage1.handoff.MediaWatch.granted(ctx)} now=${com.spacez.unlock_stage1.handoff.MediaWatch.now()?.let { "${it.pkg} playing=${it.playing} pos=${it.posMs} link=${it.url != null}" }} aura=${AuraPhone.status()}")
            "status" -> {}
            else -> Log.i("UnlockStage1", "test: unknown cmd $cmd")
        }
        UnlockState.log("test cmd $cmd: enabled=${Settings.enabled(ctx)} running=${UnlockState.running} mode=${Settings.mode(ctx)} test=${Settings.testModeOn(ctx)} rfcomm=${UnlockState.rfcommUp} adv=${UnlockState.advertising} paired=${UnlockCrypto.pcId(ctx) != null} companion=${PcCompanion.describe(ctx)} pcAddr=${PcCompanion.pcAddress(ctx)} allowlisted=${ctx.getSystemService(android.os.PowerManager::class.java).isIgnoringBatteryOptimizations(ctx.packageName)}")
    }
}
