package com.spacez.unlock_stage1.buds

import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothProfile
import android.content.Context
import android.os.Binder
import android.os.Looper
import android.os.Parcel
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

/**
 * The part that runs with the adb SHELL identity (uid 2000, which holds BLUETOOTH_PRIVILEGED): it can
 * disconnect / connect ONE device's A2DP + hands-free profiles, nothing else of Bluetooth.
 *
 * Two ways to get here, same code:
 *  - Shizuku user service ([Service]) - what the phone app uses;
 *  - over adb from the PC (no Shizuku needed):
 *    CLASSPATH=$(pm path com.spacez.unlock_stage1 | cut -d: -f2) app_process / com.spacez.unlock_stage1.buds.BudsShell disconnect|connect|status [MAC]
 * [apply] returns a one-line result: "ok ..." or "err ...".
 * NOTE: written against the hidden BluetoothA2dp/BluetoothHeadset disconnect()/connect(); not yet run on the phone.
 */
object BudsShell {
    private const val NAME = "buds core"

    @JvmStatic
    fun main(args: Array<String>) {
        // profile proxies answer on the main looper: it must be looping while the work waits on a worker thread
        if (Looper.getMainLooper() == null) Looper.prepareMainLooper()
        Thread {
            Looper.prepare() // some framework calls make a Handler on the calling thread
            println(apply(args.getOrNull(0) ?: "status", args.getOrNull(1)))
            System.exit(0)
        }.start()
        Looper.loop()
    }

    // runs as the shell user (app_process / Shizuku), which the hidden-API blocklist does not apply to
    @android.annotation.SuppressLint("BlockedPrivateApi", "PrivateApi", "DiscouragedPrivateApi")
    private fun shellContext(): Context {
        val at = Class.forName("android.app.ActivityThread")
        val thread = runCatching { at.getMethod("currentActivityThread").invoke(null) }.getOrNull()
            ?: run {
                if (Looper.getMainLooper() == null) Looper.prepareMainLooper()
                at.getMethod("systemMain").invoke(null)
            }
        // a bare app_process never ran ActivityThread.initializeMainlineModules(), so the Bluetooth service manager is
        // unset and every Bluetooth API returns null ("no bluetooth adapter"): run it here
        runCatching { at.getDeclaredMethod("initializeMainlineModules").apply { isAccessible = true }.invoke(null) }
        val sys = at.getMethod("getSystemContext").invoke(thread) as Context
        // a context whose package IS the caller (shell), so the Bluetooth stack's uid/package check agrees
        return runCatching { sys.createPackageContext("com.android.shell", 0) }.getOrDefault(sys)
    }

    // the proxy is requested from (and answers on) a looper thread of its own, so the caller can simply wait
    private val proxyThread by lazy { android.os.HandlerThread("buds-proxy").apply { start() } }

    private fun proxy(ctx: Context, ad: BluetoothAdapter, profile: Int): BluetoothProfile? {
        val got = arrayOfNulls<BluetoothProfile>(1)
        val latch = CountDownLatch(1)
        android.os.Handler(proxyThread.looper).post {
            ad.getProfileProxy(ctx, object : BluetoothProfile.ServiceListener {
                override fun onServiceConnected(p: Int, pr: BluetoothProfile) { got[0] = pr; latch.countDown() }
                override fun onServiceDisconnected(p: Int) {}
            }, profile)
        }
        latch.await(4, TimeUnit.SECONDS)
        return got[0]
    }

    private fun find(ad: BluetoothAdapter, mac: String?): BluetoothDevice? =
        ad.bondedDevices.firstOrNull { if (mac != null) it.address.equals(mac, true) else (it.name ?: "").lowercase().contains(NAME) }

    /** action: disconnect | connect | status. Returns "ok ..." or "err ...". */
    @JvmStatic
    fun apply(action: String, mac: String?): String = try {
        if (Looper.myLooper() == null && Looper.getMainLooper() == null) Looper.prepareMainLooper()
        val ctx = shellContext()
        // outside an app process getDefaultAdapter() is null: take it from the shell context's BluetoothManager
        @Suppress("DEPRECATION")
        val ad = (ctx.getSystemService(Context.BLUETOOTH_SERVICE) as? android.bluetooth.BluetoothManager)?.adapter ?: BluetoothAdapter.getDefaultAdapter()
        if (ad == null) "err no bluetooth adapter" else {
            val dev = find(ad, mac)
            if (dev == null) "err Buds not paired with the phone" else {
                val sb = StringBuilder("ok ${dev.address}")
                for ((label, id) in listOf("a2dp" to BluetoothProfile.A2DP, "hfp" to BluetoothProfile.HEADSET)) {
                    val p = proxy(ctx, ad, id)
                    if (p == null) { sb.append(" $label=noproxy"); continue }
                    try {
                        val state = p.getConnectionState(dev)
                        when (action) {
                            "disconnect" -> if (state == BluetoothProfile.STATE_CONNECTED || state == BluetoothProfile.STATE_CONNECTING) {
                                val r = p.javaClass.getMethod("disconnect", BluetoothDevice::class.java).invoke(p, dev)
                                sb.append(" $label=disconnect:$r")
                            } else sb.append(" $label=idle")
                            "connect" -> if (state != BluetoothProfile.STATE_CONNECTED) {
                                val r = p.javaClass.getMethod("connect", BluetoothDevice::class.java).invoke(p, dev)
                                sb.append(" $label=connect:$r")
                            } else sb.append(" $label=already")
                            else -> sb.append(" $label=state$state")
                        }
                    } finally { runCatching { ad.closeProfileProxy(id, p) } }
                }
                sb.toString()
            }
        }
    } catch (e: Throwable) {
        "err ${(e.cause ?: e).javaClass.simpleName}: ${(e.cause ?: e).message}"
    }

    /** Shizuku user service: a plain Binder, so no AIDL is needed. transact(1, "action[ mac]") -> result string. */
    class Service : Binder() {
        override fun onTransact(code: Int, data: Parcel, reply: Parcel?, flags: Int): Boolean {
            if (code != 1) return super.onTransact(code, data, reply, flags)
            val parts = (data.readString() ?: "status").split(' ')
            val out = apply(parts[0], parts.getOrNull(1))
            reply?.writeNoException(); reply?.writeString(out)
            return true
        }
    }
}
