package com.spacez.unlock_stage1.wear

import android.app.KeyguardManager
import android.bluetooth.BluetoothManager
import android.content.Context
import android.hardware.Sensor
import android.hardware.SensorEvent
import android.hardware.SensorEventListener
import android.hardware.SensorManager
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Log
import com.google.android.gms.wearable.CapabilityClient
import com.google.android.gms.wearable.Wearable
import com.spacez.unlock_stage1.watchlink.Kind
import com.spacez.unlock_stage1.watchlink.PcState
import com.spacez.unlock_stage1.watchlink.StatusSigner
import com.spacez.unlock_stage1.watchlink.WatchCodec
import com.spacez.unlock_stage1.watchlink.WatchCommand
import com.spacez.unlock_stage1.watchlink.WatchProtocol
import com.spacez.unlock_stage1.watchlink.WatchStatus
import kotlinx.coroutines.tasks.await
import java.security.KeyPairGenerator
import java.security.KeyStore
import java.security.MessageDigest
import java.security.PrivateKey
import java.security.SecureRandom
import java.security.Signature
import java.security.spec.ECGenParameterSpec
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

const val TAG = "CUWatch"

/** What last happened between this watch and the PC. The watch cannot see the PC's lock screen,
 *  so the home screen shows these honestly ("Unlocked by watch · 2 min ago"). */
enum class PcEvent { UNLOCK_APPROVED, UNLOCK_REFUSED, UNLOCK_PUSHED, LOCK_SENT, LOCK_VIA_PHONE, LOCK_FAILED, PAIRED }

/** EC P-256 key in the watch's Android Keystore. Private part never leaves the secure hardware. */
object WatchKeys {
    private const val ALIAS = "cu-watch-status-v1"

    private fun ks() = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }

    @Synchronized fun ensure() {
        if (ks().containsAlias(ALIAS)) return
        KeyPairGenerator.getInstance(KeyProperties.KEY_ALGORITHM_EC, "AndroidKeyStore").apply {
            initialize(KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_SIGN)
                .setAlgorithmParameterSpec(ECGenParameterSpec("secp256r1"))
                .setDigests(KeyProperties.DIGEST_SHA256)
                .build())
        }.generateKeyPair()
        Log.i(TAG, "watch key created")
    }

    fun publicKeyDer(): ByteArray { ensure(); return ks().getCertificate(ALIAS).publicKey.encoded }
    fun fingerprint(): String = WatchProtocol.fingerprint(publicKeyDer())

    val signer = StatusSigner { data ->
        ensure()
        Signature.getInstance(WatchProtocol.SIG_ALG).run {
            initSign(ks().getKey(ALIAS, null) as PrivateKey); update(data); sign()
        }
    }
}

/** Last known state + pairing flag, in one small prefs file (read by the tile and the app). */
class WatchPrefs(ctx: Context) {
    private val p = ctx.getSharedPreferences("cu_watch", Context.MODE_PRIVATE)
    var paired: Boolean
        get() = p.getBoolean("paired", false)
        set(v) = p.edit().putBoolean("paired", v).apply()
    var liveLog: Boolean
        get() = p.getBoolean("live_log", false)
        set(v) = p.edit().putBoolean("live_log", v).apply()
    /** Look: true = Material 3 Expressive (default), false = Classic navy. */
    var expressive: Boolean
        get() = p.getBoolean("look_expressive", true)
        set(v) = p.edit().putBoolean("look_expressive", v).apply()
    /** Expressive colour: [COLOUR_WATCH] (dynamic) or a [Seed] key. */
    var colour: String
        get() = p.getString("look_colour", COLOUR_WATCH) ?: COLOUR_WATCH
        set(v) = p.edit().putString("look_colour", v).apply()
    var pcState: PcState?
        get() = p.getString("pc_state", null)?.let(PcState::decode)
        set(v) = p.edit().putString("pc_state", v?.encode()).apply()
    var last: String
        get() = p.getString("last", "") ?: ""
        set(v) = p.edit().putString("last", v).apply()
    /** When the phone last told us the PC state (ms), 0 = never. */
    var pcStateMs: Long
        get() = p.getLong("pc_state_ms", 0)
        set(v) = p.edit().putLong("pc_state_ms", v).apply()
    /** adb-only test mode: answers as if worn + unlocked (the "tap" a person would give). 0 = off. */
    var testUntil: Long
        get() = p.getLong("test_until", 0)
        set(v) = p.edit().putLong("test_until", v).apply()
    val testActive get() = System.currentTimeMillis() < testUntil
    /** Last thing that happened between this watch and the PC (see [PcEvent]) and when. */
    var pcEvent: PcEvent?
        get() = p.getString("pc_evt", null)?.let { runCatching { PcEvent.valueOf(it) }.getOrNull() }
        set(v) = p.edit().putString("pc_evt", v?.name).apply()
    var pcEventMs: Long
        get() = p.getLong("pc_evt_ms", 0)
        set(v) = p.edit().putLong("pc_evt_ms", v).apply()
    fun recordPc(e: PcEvent) { p.edit().putString("pc_evt", e.name).putLong("pc_evt_ms", System.currentTimeMillis()).apply() }
    fun register(l: android.content.SharedPreferences.OnSharedPreferenceChangeListener) = p.registerOnSharedPreferenceChangeListener(l)
    fun unregister(l: android.content.SharedPreferences.OnSharedPreferenceChangeListener) = p.unregisterOnSharedPreferenceChangeListener(l)
    /** Monotonic counter; the phone rejects PUSH / COMMAND frames whose seq does not grow. */
    @Synchronized fun nextSeq(): Long { val s = p.getLong("seq", 0) + 1; p.edit().putLong("seq", s).commit(); return s }
}

object WatchState {
    /** Set only while PcLinkService holds a sensor registration; otherwise a one-shot read is used. */
    @Volatile var liveWrist: Boolean? = null

    val sensorHandler: android.os.Handler by lazy {
        android.os.Handler(android.os.HandlerThread("cu-sensor").apply { start() }.looper)
    }

    fun offBodySensor(ctx: Context): Sensor? =
        ctx.getSystemService(SensorManager::class.java)?.getDefaultSensor(Sensor.TYPE_LOW_LATENCY_OFFBODY_DETECT, true)

    /**
     * One-shot on-wrist read. The off-body sensor is on-change; Android reports its current
     * state as the first event after registration, so we register, take that one event and
     * unregister at once. Null when there is no sensor or no event within [timeoutMs].
     * Costs one sensor-hub activation; no wakelock, nothing left running.
     */
    fun readWristOnce(ctx: Context, timeoutMs: Long = 700): Boolean? {
        liveWrist?.let { return it }
        val sm = ctx.getSystemService(SensorManager::class.java) ?: return null
        val s = offBodySensor(ctx) ?: return null
        var v: Boolean? = null
        val latch = CountDownLatch(1)
        val l = object : SensorEventListener {
            override fun onSensorChanged(e: SensorEvent) { if (v == null) { v = e.values[0] >= 0.5f; latch.countDown() } }
            override fun onAccuracyChanged(sensor: Sensor?, accuracy: Int) {}
        }
        // Deliver on our own looper so a caller blocked on the latch never starves the callback.
        sm.registerListener(l, s, SensorManager.SENSOR_DELAY_NORMAL, sensorHandler)
        try { latch.await(timeoutMs, TimeUnit.MILLISECONDS) } finally { sm.unregisterListener(l) }
        return v
    }

    /** [sign] false = display only (seq 0, never sent), so tile refreshes do not burn seq numbers. */
    fun snapshot(ctx: Context, kind: Kind, nonce: ByteArray, wrist: Boolean? = readWristOnce(ctx), sign: Boolean = true): WatchStatus {
        val km = ctx.getSystemService(KeyguardManager::class.java)
        val prefs = WatchPrefs(ctx)
        if (sign && kind == Kind.REPLY && prefs.testActive) {
            // adb test mode (TestReceiver testmode_on, 30 min, off by default): answer as a person
            // who tapped Approve would, even though the watch sits on its charger.
            Log.w(TAG, "TEST MODE answer: real wrist=$wrist locked=${km.isDeviceLocked}, sent as worn + unlocked")
            return WatchStatus(kind, nonce, true, true, false, false, System.currentTimeMillis(), prefs.nextSeq())
        }
        return WatchStatus(kind, nonce, wrist != null, wrist == true, km.isDeviceLocked, km.isKeyguardLocked,
            System.currentTimeMillis(), if (sign) prefs.nextSeq() else 0)
    }

    fun describe(s: WatchStatus) = buildString {
        append(if (!s.wristKnown) "Wrist ?" else if (s.onWrist) "On wrist" else "Off wrist")
        append(" · ")
        append(if (s.deviceLocked) "Locked" else "Unlocked")
    }

    /** Stores the line the tile shows; asks for a tile rebuild only when it changed (no update loop). */
    fun remember(ctx: Context, s: WatchStatus): Boolean {
        val prefs = WatchPrefs(ctx)
        val line = describe(s) + (if (s.readyForAutoUnlock) " · Ready" else "")
        if (line == prefs.last) return false
        prefs.last = line
        Surfaces.refresh(ctx)
        return true
    }

    fun randomNonce() = ByteArray(WatchProtocol.NONCE_LEN).also { SecureRandom().nextBytes(it) }

    /** For the on-watch checklist: can this watch be a BLE peripheral (watch -> PC directly)? */
    fun blePeripheralProbe(ctx: Context): String = runCatching {
        val a = ctx.getSystemService(BluetoothManager::class.java)?.adapter ?: return "no Bluetooth adapter"
        "multi-adv=${a.isMultipleAdvertisementSupported} ext-adv=${a.isLeExtendedAdvertisingSupported}"
    }.getOrElse { "probe failed: ${it.javaClass.simpleName}" }
}

/** Data Layer sends. All event-driven: called from a user tap, a phone message or a sensor event. */
object PhoneLink {
    suspend fun phoneNodes(ctx: Context): List<String> {
        val cap = runCatching {
            Wearable.getCapabilityClient(ctx).getCapability(WatchProtocol.CAPABILITY_PHONE, CapabilityClient.FILTER_REACHABLE).await().nodes
        }.getOrDefault(emptySet())
        // Before the phone patch is installed the phone has no capability yet; pairing then
        // falls back to the connected nodes (normally exactly one: the phone).
        val nodes = cap.ifEmpty { runCatching { Wearable.getNodeClient(ctx).connectedNodes.await().toSet() }.getOrDefault(emptySet()) }
        return nodes.map { it.id }
    }

    suspend fun send(ctx: Context, path: String, data: ByteArray, nodeId: String? = null): Int {
        val ids = nodeId?.let { listOf(it) } ?: phoneNodes(ctx)
        var ok = 0
        for (id in ids) {
            runCatching { Wearable.getMessageClient(ctx).sendMessage(id, path, data).await(); ok++ }
                .onFailure { Log.w(TAG, "send $path to $id failed: ${it.message}") }
        }
        return ok
    }

    suspend fun pair(ctx: Context): Int = send(ctx, WatchProtocol.PATH_PAIR, WatchKeys.publicKeyDer())

    suspend fun sendStatus(ctx: Context, s: WatchStatus, nodeId: String? = null, payload: ByteArray = ByteArray(0)): Int =
        send(ctx, if (s.kind == Kind.COMMAND) WatchProtocol.PATH_COMMAND else WatchProtocol.PATH_STATUS,
            WatchCodec.encode(s, WatchKeys.signer, payload), nodeId)

    /** Lock PC, media keys, slides: a signed COMMAND frame; the phone relays it to the PC. */
    suspend fun command(ctx: Context, c: WatchCommand): Int {
        if (!WatchPrefs(ctx).paired) { Log.i(TAG, "command ${c.wire} skipped: not paired"); return 0 }
        val s = WatchState.snapshot(ctx, Kind.COMMAND, WatchState.randomNonce())
        Log.i(TAG, "command ${c.wire}")
        return sendStatus(ctx, s, payload = c.wire.toByteArray())
    }

    fun pairAckMatches(ack: ByteArray): Boolean =
        MessageDigest.isEqual(ack, MessageDigest.getInstance("SHA-256").digest(WatchKeys.publicKeyDer()))
}
