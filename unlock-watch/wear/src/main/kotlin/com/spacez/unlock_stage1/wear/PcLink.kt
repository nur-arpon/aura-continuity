package com.spacez.unlock_stage1.wear

import android.Manifest
import android.annotation.SuppressLint
import android.app.KeyguardManager
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothManager
import android.bluetooth.BluetoothServerSocket
import android.bluetooth.BluetoothSocket
import android.bluetooth.le.AdvertiseCallback
import android.bluetooth.le.AdvertiseData
import android.bluetooth.le.AdvertiseSettings
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.SharedPreferences
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import android.hardware.Sensor
import android.hardware.SensorEvent
import android.hardware.SensorEventListener
import android.hardware.SensorManager
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.os.ParcelUuid
import android.util.Log
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import com.spacez.unlock_stage1.watchlink.Kind
import com.spacez.unlock_stage1.watchlink.WatchCodec
import com.spacez.unlock_stage1.watchlink.WatchCommand
import com.spacez.unlock_stage1.watchlink.WatchProtocol
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import java.io.DataInputStream
import java.io.DataOutputStream
import java.util.UUID
import kotlin.concurrent.thread

/**
 * Direct watch <-> PC Continuity link (Apple-style: the PC initiates, the watch answers; no
 * phone needed). Mirrors unlock-stage1/pc/src/link.rs. Every frame: ver(1)=2 | type(1) | body,
 * u16 length-prefixed on the RFCOMM stream.
 */
object PcLinkProtocol {
    const val VER: Byte = 2
    const val T_CHALLENGE: Byte = 0x01
    const val T_NOT_NOW: Byte = 0x05
    const val T_WATCH_ATTEST: Byte = 0x06
    const val T_EVENT: Byte = 0x07
    const val T_PAIR_REQ: Byte = 0x08
    const val T_PAIR_KEY: Byte = 0x09
    const val CHALLENGE_LEN = 35
    /** watch listener (PC -> watch) */
    val WATCH_UUID: UUID = UUID.fromString("7c1e0020-5a1f-4e6b-9d2a-3c0b1a2f4e01")
    /** BLE advertising window, RSSI only */
    val WATCH_BLE_UUID: UUID = UUID.fromString("7c1e0021-5a1f-4e6b-9d2a-3c0b1a2f4e01")
    /** PC listener (watch -> PC events: Lock PC, off-wrist), hosted by autolock */
    val PC_LINK_UUID: UUID = UUID.fromString("7c1e0030-5a1f-4e6b-9d2a-3c0b1a2f4e01")
    /** PC listener for the manual "Unlock PC" push, hosted by the unlock helper (HELPER-CONTRACT.md section 9) */
    val UNLOCK_PUSH_UUID: UUID = UUID.fromString("7c1e0031-5a1f-4e6b-9d2a-3c0b1a2f4e01")
    const val PAIR_WINDOW_MS = 60_000L
    const val ADV_WINDOW_MS = 60_000
}

class PcLinkPrefs(ctx: Context) {
    private val p = ctx.getSharedPreferences("cu_pclink", Context.MODE_PRIVATE)
    /** "Hands-free unlock": answer the PC's challenge when worn + unlocked. Pairing turns it on. */
    var enabled: Boolean
        get() = p.getBoolean("enabled", false)
        set(v) = p.edit().putBoolean("enabled", v).apply()
    /** pinned at pairing: the PC's Bluetooth address and its pc_id (8 bytes, hex) */
    var pcAddress: String?
        get() = p.getString("pc_addr", null)
        set(v) = p.edit().putString("pc_addr", v).apply()
    var pcId: String?
        get() = p.getString("pc_id", null)
        set(v) = p.edit().putString("pc_id", v).apply()
    var pcName: String
        get() = p.getString("pc_name", "") ?: ""
        set(v) = p.edit().putString("pc_name", v).apply()
    var pairUntil: Long
        get() = p.getLong("pair_until", 0)
        set(v) = p.edit().putLong("pair_until", v).apply()
    val pairOpen get() = System.currentTimeMillis() < pairUntil
    val paired get() = pcAddress != null && pcId != null
    fun forget() = p.edit().remove("pc_addr").remove("pc_id").remove("pc_name").putBoolean("enabled", false).apply()
    fun register(l: SharedPreferences.OnSharedPreferenceChangeListener) = p.registerOnSharedPreferenceChangeListener(l)
    fun unregister(l: SharedPreferences.OnSharedPreferenceChangeListener) = p.unregisterOnSharedPreferenceChangeListener(l)
}

private fun ByteArray.hex() = joinToString("") { "%02x".format(it) }

/** Watch -> PC events (Lock PC, off-wrist). One short client connection per event. */
object PcDirect {
    @SuppressLint("MissingPermission")
    fun send(ctx: Context, frame: ByteArray, uuid: UUID = PcLinkProtocol.PC_LINK_UUID): Boolean {
        val addr = PcLinkPrefs(ctx).pcAddress ?: return false
        if (ctx.checkSelfPermission(Manifest.permission.BLUETOOTH_CONNECT) != PackageManager.PERMISSION_GRANTED) return false
        // Aura Link (item 11, off by default): the same signed frame as a typed message; else the legacy path below
        if (WatchAura.enabled(ctx)) {
            val body = byteArrayOf(PcLinkProtocol.VER, PcLinkProtocol.T_EVENT) + frame
            val ok = runCatching { if (uuid == PcLinkProtocol.UNLOCK_PUSH_UUID) WatchAura.trySendUnlock(body) else WatchAura.trySendLock(body) }.getOrDefault(false)
            if (ok) { Log.i(TAG, "pc event delivered over Aura Link"); return true }
        }
        val ad = ctx.getSystemService(BluetoothManager::class.java)?.adapter ?: return false
        val t0 = System.currentTimeMillis()
        var s: BluetoothSocket? = null
        return runCatching {
            s = ad.getRemoteDevice(addr).createRfcommSocketToServiceRecord(uuid)
            s!!.connect()
            val out = DataOutputStream(s!!.outputStream)
            out.writeShort(frame.size + 2); out.write(byteArrayOf(PcLinkProtocol.VER, PcLinkProtocol.T_EVENT)); out.write(frame); out.flush()
            val din = DataInputStream(s!!.inputStream)
            val n = din.readUnsignedShort(); val ack = ByteArray(n); din.readFully(ack)
            Log.i(TAG, "pc event delivered in ${System.currentTimeMillis() - t0} ms (ack ${ack.hex()})")
            true
        }.getOrElse { Log.w(TAG, "pc event failed after ${System.currentTimeMillis() - t0} ms: ${it.message}"); false }
            .also { runCatching { s?.close() } }
    }

    fun lockPc(ctx: Context): Boolean {
        val s = WatchState.snapshot(ctx, Kind.COMMAND, WatchState.randomNonce())
        return send(ctx, WatchCodec.encode(s, WatchKeys.signer, "lock_pc".toByteArray()))
    }
}

enum class LockResult { DIRECT, VIA_PHONE, FAILED, NOT_PAIRED }

enum class UnlockResult { SENT, NOT_READY, FAILED, NOT_PAIRED }

/**
 * Manual "Unlock PC": the watch pushes ONE signed approval to the PC's unlock helper, which hands it
 * to the sign-in tile's next poll (valid 15 s). Payload "unlock_pc" || pc_id(8); signed COMMAND frame
 * carrying the real wrist / lock flags. The PC refuses it unless the watch is on the wrist and
 * unlocked, so the watch checks the same thing first and says why instead of sending.
 * Direct RFCOMM only for now: the phone relay for this event is documented but not built.
 */
object UnlockAction {
    suspend fun run(ctx: Context): UnlockResult = withContext(Dispatchers.IO) {
        val app = ctx.applicationContext
        val lp = PcLinkPrefs(app)
        val wp = WatchPrefs(app)
        val pcId = lp.pcId
        if (!lp.paired || pcId == null) return@withContext UnlockResult.NOT_PAIRED
        val real = WatchState.snapshot(app, Kind.COMMAND, WatchState.randomNonce())
        val st = if (wp.testActive) {
            Log.w(TAG, "TEST MODE unlock push: real ${WatchState.describe(real)}, sent as worn + unlocked")
            real.copy(wristKnown = true, onWrist = true, deviceLocked = false, keyguardLocked = false)
        } else real
        if (!st.readyForAutoUnlock) {
            Log.i(TAG, "Unlock PC refused on the watch: ${WatchState.describe(st)}")
            return@withContext UnlockResult.NOT_READY
        }
        val payload = "unlock_pc".toByteArray() + pcId.chunked(2).map { it.toInt(16).toByte() }.toByteArray()
        val ok = runCatching { PcDirect.send(app, WatchCodec.encode(st, WatchKeys.signer, payload), PcLinkProtocol.UNLOCK_PUSH_UUID) }.getOrDefault(false)
        val r = if (ok) UnlockResult.SENT else UnlockResult.FAILED
        if (ok) wp.recordPc(PcEvent.UNLOCK_PUSHED)
        Log.i(TAG, "Unlock PC -> $r")
        Surfaces.refresh(app)
        r
    }
}

/** The one Lock PC path (app button, tile, adb): straight to the PC, the phone relay as a fallback. */
object LockAction {
    suspend fun run(ctx: Context): LockResult = withContext(Dispatchers.IO) {
        val app = ctx.applicationContext
        val lp = PcLinkPrefs(app)
        val wp = WatchPrefs(app)
        val r = when {
            !lp.paired && !wp.paired -> LockResult.NOT_PAIRED
            lp.paired && runCatching { PcDirect.lockPc(app) }.getOrDefault(false) -> LockResult.DIRECT
            wp.paired && runCatching { PhoneLink.command(app, WatchCommand.LOCK_PC) }.getOrDefault(0) > 0 -> LockResult.VIA_PHONE
            else -> LockResult.FAILED
        }
        when (r) {
            LockResult.DIRECT -> wp.recordPc(PcEvent.LOCK_SENT)
            LockResult.VIA_PHONE -> wp.recordPc(PcEvent.LOCK_VIA_PHONE)
            LockResult.FAILED -> wp.recordPc(PcEvent.LOCK_FAILED)
            LockResult.NOT_PAIRED -> {}
        }
        Log.i(TAG, "Lock PC -> $r")
        Surfaces.refresh(app)
        r
    }
}

/**
 * Foreground service holding the watch end of the link: a blocked RFCOMM accept() (no CPU,
 * no wakelock; the Bluetooth controller does the page scanning it already does for the phone)
 * plus, only if "Lock PC when watch comes off" is on, one registration on the hardware on-change
 * off-body sensor. BLE advertising only in a 60 s LOW_POWER window after the watch is unlocked /
 * put on. Started at boot / app update by [BootReceiver]; START_STICKY if the system kills it.
 */
class PcLinkService : Service() {
    private val main = Handler(Looper.getMainLooper())
    @Volatile private var server: BluetoothServerSocket? = null
    @Volatile private var stopping = false
    private var lastPushed: String? = null
    private var advertising = false
    private var sensorOn = false
    private val starting = java.util.concurrent.atomic.AtomicBoolean(false)

    private val adapter get() = getSystemService(BluetoothManager::class.java)?.adapter

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        val nm = getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(NotificationChannel("cu_link", "PC link", NotificationManager.IMPORTANCE_MIN))
        val open = PendingIntent.getActivity(this, 0, Intent(this, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE)
        val n = NotificationCompat.Builder(this, "cu_link").setSmallIcon(R.drawable.ic_status)
            .setContentTitle(getString(R.string.app_name)).setContentText("Ready to answer your PC")
            .setContentIntent(open).setColor(Palette.PRIMARY)
            .setOngoing(true).setSilent(true).setPriority(NotificationCompat.PRIORITY_MIN).build()
        ServiceCompat.startForeground(this, 12, n, ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE)
        running = true
        registerReceiver(events, IntentFilter().apply {
            addAction(Intent.ACTION_USER_PRESENT); addAction(Intent.ACTION_SCREEN_OFF)
            addAction(BluetoothAdapter.ACTION_STATE_CHANGED)
        })
        startListener()
        WatchAura.start(this)
        Log.i(TAG, "pc link on (paired=${PcLinkPrefs(this).paired}); ble ${WatchState.blePeripheralProbe(this)}")
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (!wanted(this)) { stopSelf(); return START_NOT_STICKY }
        apply()
        return START_STICKY
    }

    /** Re-read the settings: the sensor is registered only while "Lock PC when watch comes off"
     *  is on, plus one delayed re-check when a pairing window closes (not a loop). */
    private fun apply() {
        val wantSensor = WatchPrefs(this).liveLog
        val sm = getSystemService(SensorManager::class.java)
        if (wantSensor && !sensorOn) {
            val s = WatchState.offBodySensor(this)
            if (s == null) Log.w(TAG, "no off-body sensor")
            else sensorOn = sm.registerListener(sensorListener, s, SensorManager.SENSOR_DELAY_NORMAL)
        } else if (!wantSensor && sensorOn) {
            sm.unregisterListener(sensorListener); sensorOn = false; WatchState.liveWrist = null; lastPushed = null
        }
        startListener() // no-op while listening; picks up a permission granted after start
        main.removeCallbacks(pairEnd)
        val lp = PcLinkPrefs(this)
        if (lp.pairOpen) main.postDelayed(pairEnd, lp.pairUntil - System.currentTimeMillis() + 500)
        Log.i(TAG, "pc link settings: handsFree=${lp.enabled} lockWhenOff=$wantSensor pairOpen=${lp.pairOpen} paired=${lp.paired}")
    }
    private val pairEnd = Runnable { sync(this) }

    override fun onDestroy() {
        running = false
        listening = false
        stopping = true
        runCatching { server?.close() }
        WatchAura.stop()
        getSystemService(SensorManager::class.java).unregisterListener(sensorListener); sensorOn = false
        runCatching { unregisterReceiver(events) }
        stopAdvertising()
        main.removeCallbacksAndMessages(null)
        WatchState.liveWrist = null
        Surfaces.refresh(this)
        Log.i(TAG, "pc link off")
        super.onDestroy()
    }

    // ------------------------------------------------------------ RFCOMM listener

    @SuppressLint("MissingPermission")
    private fun startListener() {
        if (stopping || server != null || !starting.compareAndSet(false, true)) return
        if (checkSelfPermission(Manifest.permission.BLUETOOTH_CONNECT) != PackageManager.PERMISSION_GRANTED) {
            starting.set(false); Log.w(TAG, "BLUETOOTH_CONNECT not granted: no PC link"); return
        }
        val ad = adapter
        if (ad == null || !ad.isEnabled) { starting.set(false); Log.i(TAG, "Bluetooth off: listener waits for it"); return }
        thread(name = "cu-rfcomm", isDaemon = true) {
            try {
                val srv = ad.listenUsingRfcommWithServiceRecord("Continuity watch", PcLinkProtocol.WATCH_UUID)
                server = srv
                listening = true
                Surfaces.refresh(this)
                Log.i(TAG, "RFCOMM listening")
                while (!stopping) {
                    val s = srv.accept() ?: break
                    thread(name = "cu-rfcomm-conn", isDaemon = true) { serve(s) }
                }
            } catch (t: Throwable) {
                if (!stopping) Log.w(TAG, "RFCOMM listener ended: ${t.message}")
            } finally {
                server = null
                listening = false
                starting.set(false)
                if (!stopping) Surfaces.refresh(this)
                // restart once Bluetooth is back (ACTION_STATE_CHANGED), or after a short backoff
                if (!stopping && adapter?.isEnabled == true) main.postDelayed({ startListener() }, 2000)
            }
        }
    }

    private fun serve(s: BluetoothSocket) {
        val t0 = System.currentTimeMillis()
        try {
            val din = DataInputStream(s.inputStream)
            val out = DataOutputStream(s.outputStream)
            val n = din.readUnsignedShort()
            if (n < 2 || n > 512) return
            val f = ByteArray(n); din.readFully(f)
            if (f[0] != PcLinkProtocol.VER) return
            val prefs = PcLinkPrefs(this)
            val peer = s.remoteDevice.address
            when (f[1]) {
                PcLinkProtocol.T_CHALLENGE -> {
                    if (f.size != PcLinkProtocol.CHALLENGE_LEN || !prefs.paired) { Log.i(TAG, "challenge refused: not paired"); return }
                    if (!peer.equals(prefs.pcAddress, true) || f.copyOfRange(26, 34).hex() != prefs.pcId) {
                        Log.i(TAG, "challenge from an unknown PC $peer, ignored"); return
                    }
                    if (!prefs.enabled) {
                        // Hands-free unlock is off: say so at once, so the PC does not wait on us.
                        out.writeShort(2); out.write(byteArrayOf(PcLinkProtocol.VER, PcLinkProtocol.T_NOT_NOW)); out.flush()
                        Log.i(TAG, "PC challenge: hands-free unlock is off, answered NOT_NOW")
                        WatchPrefs(this).recordPc(PcEvent.UNLOCK_REFUSED); Surfaces.refresh(this)
                        runCatching { s.inputStream.read() }
                        return
                    }
                    val st = WatchState.snapshot(this, Kind.REPLY, f.copyOfRange(2, 18))
                    val frame = WatchCodec.encode(st, WatchKeys.signer, f)
                    out.writeShort(frame.size + 2); out.write(byteArrayOf(PcLinkProtocol.VER, PcLinkProtocol.T_WATCH_ATTEST)); out.write(frame); out.flush()
                    Log.i(TAG, "PC challenge answered in ${System.currentTimeMillis() - t0} ms: ${WatchState.describe(st)} ready=${st.readyForAutoUnlock}")
                    WatchPrefs(this).recordPc(if (st.readyForAutoUnlock) PcEvent.UNLOCK_APPROVED else PcEvent.UNLOCK_REFUSED)
                    if (!WatchState.remember(this, st)) Surfaces.refresh(this)
                    runCatching { s.inputStream.read() } // let the PC hang up first
                }
                PcLinkProtocol.T_PAIR_REQ -> {
                    if (!prefs.pairOpen || f.size < 10) { Log.i(TAG, "pair request from $peer outside the pairing window, ignored"); return }
                    prefs.pcAddress = peer; prefs.pcId = f.copyOfRange(2, 10).hex()
                    prefs.pcName = String(f, 10, f.size - 10).take(40); prefs.pairUntil = 0
                    prefs.enabled = true // pairing with a PC means: answer it hands-free
                    val der = WatchKeys.publicKeyDer()
                    out.writeShort(der.size + 2); out.write(byteArrayOf(PcLinkProtocol.VER, PcLinkProtocol.T_PAIR_KEY)); out.write(der); out.flush()
                    Log.i(TAG, "paired with PC '${prefs.pcName}' $peer; key ${WatchKeys.fingerprint()}")
                    WatchPrefs(this).recordPc(PcEvent.PAIRED); Surfaces.refresh(this)
                    main.post { apply() }
                    runCatching { s.inputStream.read() }
                }
                else -> Log.i(TAG, "unknown frame type ${f[1]} from $peer")
            }
        } catch (t: Throwable) {
            Log.w(TAG, "pc session error: ${t.message}")
        } finally {
            runCatching { s.close() }
        }
    }

    // ------------------------------------------------------------ events (no polling)

    private val events = object : BroadcastReceiver() {
        override fun onReceive(c: Context, i: Intent) {
            when (i.action) {
                Intent.ACTION_USER_PRESENT -> { advertiseWindow("unlocked"); check("unlock") }
                Intent.ACTION_SCREEN_OFF -> check("screen off")
                BluetoothAdapter.ACTION_STATE_CHANGED ->
                    if (i.getIntExtra(BluetoothAdapter.EXTRA_STATE, -1) == BluetoothAdapter.STATE_ON) main.postDelayed({ startListener() }, 500)
            }
        }
    }

    private val sensorListener = object : SensorEventListener {
        override fun onSensorChanged(e: SensorEvent) {
            val on = e.values[0] >= 0.5f
            if (WatchState.liveWrist != on) Log.i(TAG, "WRIST ${if (on) "on" else "off"}")
            WatchState.liveWrist = on
            if (on) advertiseWindow("on wrist")
            check("wrist")
            main.removeCallbacks(recheck); main.postDelayed(recheck, 2_000) // single re-check, not a loop
        }
        override fun onAccuracyChanged(sensor: Sensor?, accuracy: Int) {}
    }
    private val recheck = Runnable { check("recheck") }

    /** Push a signed state change to the PC directly (and to the phone, if paired) when it changes. */
    private fun check(why: String) {
        if (!WatchPrefs(this).liveLog) return
        val km = getSystemService(KeyguardManager::class.java)
        val key = "${WatchState.liveWrist}/${km.isDeviceLocked}"
        if (key == lastPushed) return
        lastPushed = key
        val st = WatchState.snapshot(this, Kind.PUSH, ByteArray(WatchProtocol.NONCE_LEN), WatchState.liveWrist)
        Log.i(TAG, "STATE ${WatchState.describe(st)} ready=${st.readyForAutoUnlock} ($why)")
        WatchState.remember(this, st)
        val frame = WatchCodec.encode(st, WatchKeys.signer)
        thread(isDaemon = true) {
            if (PcLinkPrefs(this).paired) PcDirect.send(this, frame)
            if (WatchPrefs(this).paired) runBlocking { PhoneLink.sendStatus(this@PcLinkService, st) }
        }
    }

    // ------------------------------------------------------------ BLE: short LOW_POWER window, RSSI only

    private val advCb = object : AdvertiseCallback() {
        override fun onStartSuccess(settingsInEffect: AdvertiseSettings?) { Log.i(TAG, "BLE window on (${PcLinkProtocol.ADV_WINDOW_MS / 1000} s, low power)") }
        override fun onStartFailure(errorCode: Int) { advertising = false; Log.i(TAG, "BLE window failed ($errorCode)") }
    }

    @SuppressLint("MissingPermission")
    private fun advertiseWindow(why: String) {
        if (advertising || checkSelfPermission(Manifest.permission.BLUETOOTH_ADVERTISE) != PackageManager.PERMISSION_GRANTED) return
        val adv = adapter?.bluetoothLeAdvertiser ?: return
        runCatching {
            // the controller stops it by itself after the timeout (hardware-offloaded); no app timer needed
            adv.startAdvertising(
                AdvertiseSettings.Builder().setAdvertiseMode(AdvertiseSettings.ADVERTISE_MODE_LOW_POWER)
                    .setTxPowerLevel(AdvertiseSettings.ADVERTISE_TX_POWER_LOW).setConnectable(false)
                    .setTimeout(PcLinkProtocol.ADV_WINDOW_MS).build(),
                AdvertiseData.Builder().addServiceUuid(ParcelUuid(PcLinkProtocol.WATCH_BLE_UUID)).setIncludeDeviceName(false).build(),
                advCb)
            advertising = true
            Log.i(TAG, "BLE window requested ($why)")
            main.postDelayed({ advertising = false }, PcLinkProtocol.ADV_WINDOW_MS + 1000L)
        }.onFailure { Log.i(TAG, "BLE window not possible: ${it.message}") }
    }

    @SuppressLint("MissingPermission")
    private fun stopAdvertising() {
        if (!advertising) return
        runCatching { adapter?.bluetoothLeAdvertiser?.stopAdvertising(advCb) }
        advertising = false
    }

    companion object {
        @Volatile var running = false
        /** True while the RFCOMM accept() is up, i.e. the PC can reach this watch. */
        @Volatile var listening = false

        fun wanted(ctx: Context): Boolean {
            val lp = PcLinkPrefs(ctx)
            return (lp.enabled && lp.paired) || WatchPrefs(ctx).liveLog || lp.pairOpen
        }

        /** Runs while hands-free unlock (paired), "Lock PC when watch comes off" or a pairing window is on. */
        fun sync(ctx: Context): Boolean {
            val i = Intent(ctx, PcLinkService::class.java)
            if (!wanted(ctx)) { ctx.stopService(i); return true }
            return runCatching { ctx.startForegroundService(i); true }
                .getOrElse { Log.w(TAG, "pc link start refused: ${it.message}"); false }
        }
    }
}

/**
 * Test hooks for adb (no screen taps needed). Protected by android.permission.DUMP, which only
 * the shell / system hold, so other apps cannot trigger it.
 *   adb shell am broadcast -a com.spacez.unlock_stage1.wear.TEST --es cmd <cmd>
 *     -n com.spacez.unlock_stage1/com.spacez.unlock_stage1.wear.TestReceiver
 * cmd: status | link_on | link_off | lockwhenoff_on | lockwhenoff_off | pair | lock | unlock
 *      testmode_on | testmode_off  (test mode: answer the PC as worn + unlocked for 30 min; off by default)
 */
class TestReceiver : BroadcastReceiver() {
    override fun onReceive(c: Context, i: Intent) {
        val ctx = c.applicationContext
        val lp = PcLinkPrefs(ctx)
        val wp = WatchPrefs(ctx)
        when (val cmd = i.getStringExtra("cmd")) {
            "link_on" -> { lp.enabled = true; PcLinkService.sync(ctx) }
            "link_off" -> { lp.enabled = false; wp.liveLog = false; PcLinkService.sync(ctx) }
            "lockwhenoff_on" -> { wp.liveLog = true; PcLinkService.sync(ctx) }
            "lockwhenoff_off" -> { wp.liveLog = false; PcLinkService.sync(ctx) }
            "pair" -> {
                lp.pairUntil = System.currentTimeMillis() + PcLinkProtocol.PAIR_WINDOW_MS
                Log.i(TAG, "PC pairing window open 60 s; key ${WatchKeys.fingerprint()} service=${PcLinkService.sync(ctx)}")
            }
            "lock" -> { val done = goAsync(); thread { try { Log.i(TAG, "test lock -> ${runBlocking { LockAction.run(ctx) }}") } finally { done.finish() } } }
            "unlock" -> { val done = goAsync(); thread { try { Log.i(TAG, "test unlock -> ${runBlocking { UnlockAction.run(ctx) }}") } finally { done.finish() } } }
            "testmode_on" -> {
                wp.testUntil = System.currentTimeMillis() + TEST_MODE_MS
                Log.w(TAG, "TEST MODE ON for 30 min: PC challenges answered as worn + unlocked"); Surfaces.refresh(ctx)
            }
            "aura_on" -> WatchAura.setEnabled(ctx, true)
            "aura_off" -> WatchAura.setEnabled(ctx, false)
            "aura_status" -> Log.i(TAG, "aura: enabled=${WatchAura.enabled(ctx)} ${WatchAura.link?.status() ?: "off"}")
            "testmode_off" -> { wp.testUntil = 0; Log.w(TAG, "TEST MODE OFF"); Surfaces.refresh(ctx) }
            "status" -> Log.i(TAG, "status handsFree=${lp.enabled} lockWhenOff=${wp.liveLog} pcPaired=${lp.paired} pc=${lp.pcAddress} '${lp.pcName}' " +
                "phonePaired=${wp.paired} service=${PcLinkService.running} listening=${PcLinkService.listening} testMode=${wp.testActive} " +
                "last=${wp.pcEvent}@${wp.pcEventMs} key=${WatchKeys.fingerprint()}")
            else -> Log.i(TAG, "test: unknown cmd $cmd")
        }
    }
    companion object { const val TEST_MODE_MS = 30 * 60_000L }
}
