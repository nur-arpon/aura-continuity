package com.spacez.unlock_stage1.away

import android.content.Context
import android.content.Intent
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.net.Uri
import com.spacez.aura.link.AuraLink
import com.spacez.aura.link.Conn
import com.spacez.aura.link.Role
import com.spacez.unlock_stage1.UnlockState
import java.net.Inet4Address
import java.net.InetAddress
import java.net.InetSocketAddress
import java.net.NetworkInterface
import java.net.ServerSocket
import java.net.Socket
import kotlin.concurrent.thread

/**
 * The phone's view of the owner's Tailscale tailnet (item 17). Tailscale is a separate app
 * (com.tailscale.ipn, BSD-3); Aura only looks at the VPN interface it creates and never talks to
 * Tailscale itself. Event-driven: a NetworkCallback for VPN networks (no polling).
 */
object Tailnet {
    /** This phone's tailnet IPv4 (the Tailscale VPN interface), or null when Tailscale is off. */
    fun ownIp(): String? = runCatching {
        NetworkInterface.getNetworkInterfaces()?.toList().orEmpty().filter { it.isUp }.flatMap { it.inetAddresses.toList() }
            .firstOrNull { it is Inet4Address && AwayLogic.isTailnetV4(it.address) }?.hostAddress
    }.getOrNull()

    fun installed(c: Context) = runCatching { c.packageManager.getPackageInfo(AwayLogic.TAILSCALE_PKG, 0); true }.getOrDefault(false)

    /** Play Store page for Tailscale (one tap Install), web fallback when there is no Play Store. */
    fun installIntent(c: Context): Intent {
        val market = Intent(Intent.ACTION_VIEW, Uri.parse("market://details?id=${AwayLogic.TAILSCALE_PKG}")).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        return if (market.resolveActivity(c.packageManager) != null) market
        else Intent(Intent.ACTION_VIEW, Uri.parse("https://play.google.com/store/apps/details?id=${AwayLogic.TAILSCALE_PKG}")).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
    }

    /**
     * The owner turns Tailscale off to save battery: switch it on just for a stream. Tailscale accepts a CONNECT_VPN
     * broadcast from any app (meant for automation). Blocks up to [waitMs] for the tailnet address; true when it is up.
     * Never call on the main thread.
     */
    fun ensureUp(c: Context, waitMs: Long = 7_000): Boolean {
        if (ownIp() != null) return true // already on (the owner's doing): never ours to turn off
        if (!installed(c)) return false
        runCatching {
            c.sendBroadcast(Intent("com.tailscale.ipn.CONNECT_VPN").setClassName(AwayLogic.TAILSCALE_PKG, "com.tailscale.ipn.IPNReceiver"))
        }
        val end = System.currentTimeMillis() + waitMs
        while (System.currentTimeMillis() < end) {
            if (ownIp() != null) break
            Thread.sleep(250)
        }
        val up = ownIp() != null
        // Aura switched it on (it was off before): Aura switches it off again after the stream (setting "Turn Tailscale off after streaming")
        if (up) TailnetAutoOff.armed(c)
        return up
    }

    /** Opens the Tailscale app (sign in / connect). */
    fun openIntent(c: Context): Intent? = c.packageManager.getLaunchIntentForPackage(AwayLogic.TAILSCALE_PKG)?.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)

    fun connect(ip: String, port: Int, timeoutMs: Int): Conn {
        val s = Socket()
        s.connect(InetSocketAddress(InetAddress.getByName(ip), port), timeoutMs)
        return Conn.tcp(s, "tailnet $ip")
    }

    // ------------------------------------------------------------------ VPN up/down + listener

    @Volatile private var cb: ConnectivityManager.NetworkCallback? = null
    @Volatile private var server: ServerSocket? = null
    @Volatile private var boundIp: String? = null

    /**
     * Watch the Tailscale VPN. When it comes up: listen on our tailnet address (so the PC can deliver
     * a reply whose session dropped) and call [onUp] (AwayPhone fetches the PC's last activity once).
     */
    @Synchronized
    fun watch(c: Context, link: () -> AuraLink?, onUp: () -> Unit) {
        if (cb != null) return
        val cm = c.getSystemService(ConnectivityManager::class.java) ?: return
        val req = NetworkRequest.Builder().addTransportType(NetworkCapabilities.TRANSPORT_VPN)
            .removeCapability(NetworkCapabilities.NET_CAPABILITY_NOT_VPN).build()
        val k = object : ConnectivityManager.NetworkCallback() {
            override fun onAvailable(n: Network) { thread(isDaemon = true) { Thread.sleep(800); if (listen(link)) onUp() } }
            override fun onLost(n: Network) { if (ownIp() == null) { close("VPN lost"); TailnetAutoOff.disarm(c.applicationContext, "VPN went down") } }
        }
        runCatching { cm.registerNetworkCallback(req, k); cb = k }.onFailure { UnlockState.log("away: VPN callback not registered: ${it.message}") }
        thread(isDaemon = true) { listen(link) }
    }

    @Synchronized
    fun unwatch(c: Context) {
        cb?.let { k -> runCatching { c.getSystemService(ConnectivityManager::class.java)?.unregisterNetworkCallback(k) } }
        cb = null
        close("stopped")
    }

    @Synchronized
    private fun close(why: String) {
        server?.let { runCatching { it.close() }; UnlockState.log("away: tailnet listener closed ($why)") }
        server = null; boundIp = null
    }

    fun listening(): String? = boundIp

    /** true when a (new) listener is up on our tailnet address. */
    private fun listen(link: () -> AuraLink?): Boolean {
        val ip = ownIp() ?: return false
        synchronized(this) {
            if (server != null && boundIp == ip) return false
            close("address changed")
            val s = runCatching { ServerSocket().apply { reuseAddress = true; bind(InetSocketAddress(InetAddress.getByName(ip), AwayLogic.PORT)) } }
                .getOrElse { UnlockState.log("away: tailnet listen $ip failed: ${it.message}"); return false }
            server = s; boundIp = ip
            UnlockState.log("away: listening on the tailnet $ip:${AwayLogic.PORT}")
            thread(isDaemon = true, name = "aura-tailnet-accept") {
                while (!s.isClosed) {
                    val sock = runCatching { s.accept() }.getOrNull() ?: break
                    val peer = sock.inetAddress
                    if (peer !is Inet4Address || !AwayLogic.isTailnetV4(peer.address)) { runCatching { sock.close() }; continue }
                    thread(isDaemon = true) {
                        runCatching { AuraLink.respondAndAttach(Conn.tcp(sock, "tailnet-in ${peer.hostAddress}"), 10_000) { h -> if (h.role == Role.PC) link() else null } }
                            .onFailure { UnlockState.log("away: tailnet connection from ${peer.hostAddress} refused: ${it.message}") }
                    }
                }
            }
        }
        return true
    }
}
