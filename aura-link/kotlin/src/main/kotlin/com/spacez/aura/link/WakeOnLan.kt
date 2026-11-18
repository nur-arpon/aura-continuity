package com.spacez.aura.link

import java.net.DatagramPacket
import java.net.DatagramSocket
import java.net.InetAddress

/**
 * Wake-on-LAN magic packets (phone and watch share this). Pure JVM: the caller supplies [bind] to pin the
 * socket to the Wi-Fi network (Network.bindSocket on Android) so packets never leave over Tailscale / mobile data.
 */
object WakeOnLan {
    /** "02:00:00:AA:BB:CC" (or - separated) -> 6 bytes, null if malformed. */
    fun parseMac(s: String): ByteArray? {
        val p = s.trim().split(':', '-')
        if (p.size != 6) return null
        return ByteArray(6) { p[it].toIntOrNull(16)?.takeIf { v -> v in 0..255 }?.toByte() ?: return null }
    }

    /** 6 x 0xFF, then the MAC 16 times (102 bytes). */
    fun magicPacket(mac: ByteArray): ByteArray {
        require(mac.size == 6)
        val b = ByteArray(102) { 0xFF.toByte() }
        for (i in 0 until 16) System.arraycopy(mac, 0, b, 6 + i * 6, 6)
        return b
    }

    /** One destination. [wifi] = pin to the Wi-Fi network (LAN broadcast); false = a plain unicast (tailnet relay). */
    data class Target(val host: String, val port: Int, val wifi: Boolean)

    /** Same /24 as [ip]? (home networks; good enough to match a remembered network without the SSID.) */
    fun sameSubnet(a: String, b: String) = a.substringBeforeLast('.') == b.substringBeforeLast('.')

    /** Broadcast address of a /24 from any address in it. */
    fun broadcastOf(ip: String) = ip.substringBeforeLast('.') + ".255"

    /** Fire and forget: every MAC to every target, [repeat] times, [gapMs] apart. Returns how many datagrams went out. */
    fun send(macs: List<ByteArray>, targets: List<Target>, repeat: Int = 3, gapMs: Long = 300, bind: ((DatagramSocket) -> Unit)? = null): Int {
        var sent = 0
        repeat(repeat) { round ->
            for (t in targets) {
                runCatching {
                    DatagramSocket().use { s ->
                        s.broadcast = true
                        if (t.wifi) bind?.invoke(s)
                        val addr = InetAddress.getByName(t.host)
                        for (m in macs) { val pk = magicPacket(m); s.send(DatagramPacket(pk, pk.size, addr, t.port)); sent++ }
                    }
                }
            }
            if (round < repeat - 1) Thread.sleep(gapMs)
        }
        return sent
    }
}
