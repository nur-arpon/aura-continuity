package com.spacez.unlock_stage1.extras

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.wifi.WifiManager
import com.spacez.aura.link.Kind
import com.spacez.aura.link.Message
import com.spacez.aura.link.WakeOnLan
import com.spacez.unlock_stage1.PcState
import com.spacez.unlock_stage1.UnlockState
import com.spacez.unlock_stage1.away.AwayPhone
import org.json.JSONArray
import org.json.JSONObject
import java.net.Inet4Address

/**
 * Remote power: Wake / Sleep / Restart / Shut down the PC.
 *
 * Wake = Wake-on-LAN magic packets. The PC tells the phone its MACs, tailnet relays and the Wi-Fi network it is
 * on (LAPTOP_INFO "wake"); every network seen is remembered, so "set up once on any Wi-Fi" just works while both
 * are connected. Sleep / Restart / Shut down = PC_POWER over the Aura link.
 */
object PcWake {
    private const val PREFS = "pcwake"
    private fun p(c: Context) = c.getSharedPreferences(PREFS, 0)
    private fun log(s: String) = UnlockState.log("wake: $s")

    data class Net(val ssid: String, val bssid: String, val ip: String, val broadcast: String)

    // ------------------------------------------------------------------ remembered state

    fun macs(c: Context): List<String> = strings(p(c).getString("macs", null))
    fun relays(c: Context): List<String> = strings(p(c).getString("relays", null))

    fun networks(c: Context): List<Net> = runCatching {
        val o = JSONObject(p(c).getString("nets", "{}")!!)
        o.keys().asSequence().map { k -> o.getJSONObject(k).let { Net(k, it.optString("bssid"), it.optString("ip"), it.optString("broadcast")) } }.toList()
    }.getOrDefault(emptyList())

    private fun strings(j: String?): List<String> = runCatching {
        val a = JSONArray(j ?: "[]"); (0 until a.length()).map { a.getString(it) }
    }.getOrDefault(emptyList())

    private fun putNet(c: Context, n: Net) {
        val o = runCatching { JSONObject(p(c).getString("nets", "{}")!!) }.getOrDefault(JSONObject())
        o.put(n.ssid, JSONObject().put("bssid", n.bssid).put("ip", n.ip).put("broadcast", n.broadcast))
        p(c).edit().putString("nets", o.toString()).apply()
    }

    fun forget(c: Context, ssid: String) {
        val o = runCatching { JSONObject(p(c).getString("nets", "{}")!!) }.getOrDefault(JSONObject())
        o.remove(ssid); p(c).edit().putString("nets", o.toString()).apply()
    }

    /** LAPTOP_INFO JSON from the PC: keep the "wake" block (macs/relays latest wins, network accumulates). */
    fun save(c: Context, json: String) {
        val w = runCatching { JSONObject(json).optJSONObject("wake") }.getOrNull() ?: return
        val e = p(c).edit()
        w.optJSONArray("macs")?.let { a ->
            val m = (0 until a.length()).map { a.getString(it) }.filter { WakeOnLan.parseMac(it) != null }
            if (m.isNotEmpty()) e.putString("macs", JSONArray(m).toString())
        }
        w.optJSONArray("relays")?.let { a ->
            e.putString("relays", JSONArray((0 until a.length()).map { a.getString(it) }.filter { it.isNotBlank() }).toString())
        }
        e.apply()
        w.optJSONObject("network")?.let { n ->
            val ssid = n.optString("ssid"); val ip = n.optString("ip"); val bc = n.optString("broadcast")
            if (ssid.isNotBlank() && bc.isNotBlank()) putNet(c, Net(ssid, n.optString("bssid"), ip, bc))
        }
        log("saved: ${macs(c).size} MAC(s), ${relays(c).size} relay(s), ${networks(c).size} network(s)")
    }

    fun pcName(c: Context): String = Moonlight.info(c)?.host?.takeIf { it.isNotBlank() } ?: PcState.name(c) ?: "your PC"

    // ------------------------------------------------------------------ the phone's Wi-Fi

    class Wifi(val network: Network, val ssid: String?, val ip: String, val prefix: Int) {
        val broadcast: String get() = broadcastOf(ip, prefix)
    }

    private fun broadcastOf(ip: String, prefix: Int): String = runCatching {
        val b = ip.split('.').map { it.toInt() }
        var v = (b[0] shl 24) or (b[1] shl 16) or (b[2] shl 8) or b[3]
        val mask = if (prefix <= 0) 0 else -1 shl (32 - prefix.coerceAtMost(32))
        v = v or mask.inv()
        "${(v ushr 24) and 255}.${(v ushr 16) and 255}.${(v ushr 8) and 255}.${v and 255}"
    }.getOrDefault(WakeOnLan.broadcastOf(ip))

    @Suppress("DEPRECATION")
    fun wifi(c: Context): Wifi? = runCatching {
        val cm = c.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
        val n = cm.allNetworks.firstOrNull { cm.getNetworkCapabilities(it)?.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) == true } ?: return null
        val la = cm.getLinkProperties(n)?.linkAddresses?.firstOrNull { it.address is Inet4Address } ?: return null
        // The SSID needs location permission; without it Android reports "<unknown ssid>" and the subnet is used instead.
        val raw = runCatching { (c.applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager).connectionInfo?.ssid }.getOrNull()
        val ssid = raw?.trim('"')?.takeIf { it.isNotBlank() && it != "<unknown ssid>" }
        Wifi(n, ssid, la.address.hostAddress ?: return null, la.prefixLength)
    }.getOrNull()

    /** Settings: "Remember this Wi-Fi for Wake". Returns the line to show. */
    fun rememberCurrent(c: Context): String {
        val w = wifi(c) ?: return "Connect to Wi-Fi first"
        val key = w.ssid ?: "Wi-Fi ${w.ip.substringBeforeLast('.')}.x"
        // a real SSID replaces the placeholder stored for this subnet while the name was unreadable
        if (w.ssid != null) networks(c).filter { it.ssid.startsWith("Wi-Fi ") && it.ssid.substringAfter("Wi-Fi ").removeSuffix(".x") == w.ip.substringBeforeLast('.') }.forEach { forget(c, it.ssid) }
        putNet(c, Net(key, "", w.ip, w.broadcast))
        log("remembered $key (${w.broadcast})")
        return "Remembered ${w.ssid ?: "this Wi-Fi"} for Wake"
    }

    fun status(c: Context): Map<String, Any?> {
        val w = wifi(c)
        return mapOf(
            "macs" to macs(c), "relays" to relays(c),
            "networks" to networks(c).map { it.ssid },
            "ssid" to w?.ssid, "wifi" to (w != null),
            "known" to (w != null && matching(c, w).isNotEmpty()),
            "location" to (c.checkSelfPermission(android.Manifest.permission.ACCESS_FINE_LOCATION) == android.content.pm.PackageManager.PERMISSION_GRANTED),
            "name" to pcName(c),
        )
    }

    /** Remembered networks for the Wi-Fi we are on: by SSID first, by subnet only when no SSID matches. */
    private fun matching(c: Context, w: Wifi): List<Net> {
        val all = networks(c)
        if (w.ssid != null) all.filter { it.ssid == w.ssid }.let { if (it.isNotEmpty()) return it }
        return all.filter { it.ip.isNotBlank() && WakeOnLan.sameSubnet(it.ip, w.ip) }
    }

    // ------------------------------------------------------------------ wake

    fun targets(c: Context): List<WakeOnLan.Target> {
        val out = LinkedHashSet<WakeOnLan.Target>()
        val w = wifi(c)
        if (w != null) matching(c, w).forEach { n ->
            for (port in intArrayOf(9, 7)) {
                out += WakeOnLan.Target(n.broadcast, port, true)
                out += WakeOnLan.Target("255.255.255.255", port, true)
            }
        }
        relays(c).forEach { out += WakeOnLan.Target(it, 9, false) }
        return out.toList()
    }

    /** Sends the packets (blocking ~0.6 s). Returns null when nothing could be sent, else the number of datagrams. */
    fun sendPackets(c: Context): Int? {
        val macs = macs(c).mapNotNull { WakeOnLan.parseMac(it) }
        if (macs.isEmpty()) return null
        val t = targets(c)
        if (t.isEmpty()) return null
        val net = wifi(c)?.network
        val n = WakeOnLan.send(macs, t, 3, 300) { s -> runCatching { net?.bindSocket(s) } }
        log("sent $n magic packet(s) to ${t.size} target(s)")
        return n
    }

    /** Blocking (<= ~65 s): send, then watch for the Aura link. Returns the line to show. */
    fun wake(c: Context): String {
        if (macs(c).isEmpty()) return "Open Aura with your PC nearby once so it can learn how to wake it"
        if (sendPackets(c) == null) return "No way to reach it from here yet. Connect to its Wi-Fi once, or turn on Tailscale"
        val l = AwayPhone.link(c) ?: return "Not paired with a PC"
        val end = System.currentTimeMillis() + 60_000
        var nudge = 0L
        while (System.currentTimeMillis() < end) {
            if (l.connected) return "PC is awake"
            if (System.currentTimeMillis() >= nudge) { // knock on the link every 6 s so it reconnects as soon as the PC is up
                AwayPhone.userAction(); runCatching { l.send(Message(Kind.PING, ByteArray(0))) }; nudge = System.currentTimeMillis() + 6_000
            }
            Thread.sleep(500)
        }
        return "No answer — the PC may be off, on battery saver, or on a network that blocks wake packets"
    }

    // ------------------------------------------------------------------ restart / shut down / sleep

    /** [action] = "restart" | "shutdown" | "sleep", over the Aura link (same path as Lock PC). */
    fun power(c: Context, action: String): String {
        if (action !in setOf("restart", "shutdown", "sleep")) return "Unknown action"
        val l = AwayPhone.link(c) ?: return "Not paired with a PC"
        AwayPhone.userAction()
        val body = JSONObject().put("action", action).put("from", android.os.Build.MODEL).toString().toByteArray()
        val t0 = System.currentTimeMillis()
        val err = l.waitAcked(l.send(Message(Kind.PC_POWER, body)), 15_000)
        log("PC_POWER $action: ${err ?: "accepted"} in ${System.currentTimeMillis() - t0} ms")
        val word = when (action) { "restart" -> "Restarting"; "shutdown" -> "Shutting down"; else -> "Sleeping" }
        return if (err == null) "$word ${pcName(c)}" else "PC not reachable: $err"
    }
}
