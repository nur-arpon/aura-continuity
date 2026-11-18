package com.spacez.unlock_stage1.extras

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.net.Uri
import com.spacez.unlock_stage1.away.AwayLogic
import org.json.JSONObject
import java.net.Inet4Address
import java.net.NetworkInterface

/**
 * "Open laptop": Aura only LAUNCHES Moonlight (or its Artemis fork), the client of the Sunshine host that Aura PC
 * manages (both GPL-3.0, separate apps). No Moonlight code is linked or copied, so Aura's own licence is unaffected.
 * The PC tells the phone its addresses over Aura Link (LAPTOP_INFO) so nothing is typed here.
 */
object Moonlight {
    const val PKG = "com.limelight"
    const val ARTEMIS_PKG = "com.limelight.noir"
    private const val PREFS = "laptop"

    /** What the client's shortcut intent needs to start the host's Desktop in one step (PC LAPTOP_INFO "desktop"). */
    data class Desktop(val uuid: String, val name: String, val appId: Long, val appUuid: String?)

    data class Info(val host: String, val lan: List<String>, val tailnet: String?, val port: Int, val installed: Boolean, val running: Boolean, val desktop: Desktop? = null)

    private fun privateV4(s: String): Boolean {
        val p = s.split('.').map { it.toIntOrNull() ?: return false }
        if (p.size != 4 || p.any { it !in 0..255 }) return false
        return p[0] == 10 || (p[0] == 172 && p[1] in 16..31) || (p[0] == 192 && p[1] == 168)
    }

    /** The PC's LAPTOP_INFO JSON -> Info; addresses that are not private LAN / tailnet are dropped. Null if unusable. */
    fun parseInfo(json: String): Info? = runCatching {
        val o = JSONObject(json)
        val lan = o.optJSONArray("lan")?.let { a -> (0 until a.length()).map { a.getString(it) } }.orEmpty().filter { privateV4(it) }
        val tn = o.optString("tailnet", "").takeIf { it.isNotEmpty() && o.opt("tailnet") != JSONObject.NULL && AwayLogic.isTailnetV4(it) }
        if (lan.isEmpty() && tn == null) return null
        val desk = o.optJSONObject("desktop")?.let { d ->
            val u = d.optString("uuid")
            if (u.isBlank()) null else Desktop(u, d.optString("app", "Desktop").ifBlank { "Desktop" }, d.optLong("app_id", 881448767L), if (d.isNull("app_uuid")) null else d.optString("app_uuid").ifBlank { null })
        }
        Info(o.optString("host", "PC").take(60), lan, tn, o.optInt("port", 47989).takeIf { it in 1024..65535 } ?: 47989, o.optBoolean("installed", true), o.optBoolean("running", true), desk)
    }.getOrNull()

    fun save(c: Context, json: String) {
        if (parseInfo(json) != null) c.getSharedPreferences(PREFS, 0).edit().putString("info", json).apply()
    }

    fun info(c: Context): Info? = c.getSharedPreferences(PREFS, 0).getString("info", null)?.let { parseInfo(it) }

    private fun sameSubnet(a: String, b: String) = a.substringBeforeLast('.') == b.substringBeforeLast('.')

    /** At home (the phone shares a /24 with a LAN address of the PC) use that address; otherwise the tailnet one. */
    fun bestAddress(i: Info, phoneIps: List<String>): String? {
        i.lan.firstOrNull { l -> phoneIps.any { sameSubnet(it, l) } }?.let { return it }
        return i.tailnet ?: i.lan.firstOrNull()
    }

    fun phoneIps(): List<String> = runCatching {
        NetworkInterface.getNetworkInterfaces().toList().flatMap { n -> n.inetAddresses.toList().filterIsInstance<Inet4Address>().map { it.hostAddress ?: "" } }
    }.getOrDefault(emptyList())

    /** Artemis (the fork) wins when it is installed, otherwise stock Moonlight; null = neither. */
    fun clientPackage(c: Context): String? = listOf(ARTEMIS_PKG, PKG).firstOrNull { installed(c, it) }

    fun installed(c: Context, pkg: String) = runCatching { c.packageManager.getPackageInfo(pkg, 0); true }.getOrDefault(false)

    fun installIntent(c: Context): Intent {
        val market = Intent(Intent.ACTION_VIEW, Uri.parse("market://details?id=$PKG")).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        return if (market.resolveActivity(c.packageManager) != null) market
        else Intent(Intent.ACTION_VIEW, Uri.parse("https://play.google.com/store/apps/details?id=$PKG")).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
    }

    /**
     * The shortcut intent of Moonlight / Artemis (its ShortcutTrampoline, the one pinned game shortcuts use): starts the stream of one
     * app of one host without the host list. Null when the PC has not told us the host's ids yet.
     */
    fun desktopIntent(i: Info, pkg: String): Intent? {
        val d = i.desktop ?: return null
        return Intent().setClassName(pkg, "com.limelight.ShortcutTrampoline").addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP)
            .putExtra("Name", i.host).putExtra("UUID", d.uuid).putExtra("AppName", d.name).putExtra("AppId", d.appId.toString())
            .apply { d.appUuid?.let { putExtra("AppUUID", it) } }
    }

    /**
     * Away from the laptop (no Bluetooth link) with Tailscale switched off, the stream cannot reach the PC (its host only
     * accepts the tailnet and the home network): turn Tailscale on first, then run [open] off the main thread.
     * Returns a toast line, or null when nothing needed doing (the caller opens straight away).
     */
    private fun viaTailnet(c: Context, open: () -> String): String? {
        val near = com.spacez.unlock_stage1.AuraPhone.link?.connected == true
        val ts = com.spacez.unlock_stage1.away.Tailnet
        if (near || ts.ownIp() != null || !ts.installed(c)) return null
        val app = c.applicationContext
        Thread {
            val up = ts.ensureUp(app)
            android.os.Handler(android.os.Looper.getMainLooper()).post {
                val msg = if (up) open() else "Tailscale did not come up: open Tailscale, then try again"
                android.widget.Toast.makeText(app, msg, android.widget.Toast.LENGTH_SHORT).show()
            }
        }.start()
        return "Turning on Tailscale, then opening your laptop"
    }

    /** Same shortcut intent, for one app entry of the host (the PC registered it: [name] = host app name, [appUuid] its uuid). */
    fun openApp(c: Context, name: String, appUuid: String): String {
        viaTailnet(c) { openAppNow(c, name, appUuid) }?.let { return it }
        return openAppNow(c, name, appUuid)
    }

    private fun openAppNow(c: Context, name: String, appUuid: String): String {
        val pkg = clientPackage(c)
        val i = info(c)
        val d = i?.desktop
        // only Artemis takes the app by UUID (stock Moonlight needs an integer AppId): anything else shows the Desktop
        if (pkg != ARTEMIS_PKG || i == null || d == null || !i.running) return openDesktop(c)
        val intent = Intent().setClassName(pkg, "com.limelight.ShortcutTrampoline").addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP)
            .putExtra("Name", i.host).putExtra("UUID", d.uuid).putExtra("AppName", name).putExtra("AppUUID", appUuid)
        return if (runCatching { c.startActivity(intent) }.isSuccess) "Opening $name in ${if (pkg == ARTEMIS_PKG) "Artemis" else "Moonlight"}" else openDesktop(c)
    }

    /** Open the laptop's Desktop stream directly (no host list); the plain client otherwise. Short sentence for a toast. */
    fun openDesktop(c: Context): String {
        viaTailnet(c) { openDesktopNow(c) }?.let { return it }
        return openDesktopNow(c)
    }

    private fun openDesktopNow(c: Context): String {
        val pkg = clientPackage(c)
        val i = info(c)
        val intent = if (pkg != null && i != null && i.running) desktopIntent(i, pkg) else null
        if (intent != null) {
            val r = runCatching { c.startActivity(intent) }
            if (r.isSuccess) return "Opening your laptop desktop in ${if (pkg == ARTEMIS_PKG) "Artemis" else "Moonlight"}"
        }
        return open(c)
    }

    private fun copy(c: Context, text: String) {
        runCatching { (c.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager).setPrimaryClip(ClipData.newPlainText("PC address", text)) }
    }

    /** Returns a short plain sentence for the toast; the Dart side treats "Opening ..." as success. */
    fun open(c: Context): String {
        val pkg = clientPackage(c) ?: return runCatching { c.startActivity(installIntent(c)); "Install Moonlight (free) from Google Play, then tap Open laptop again" }
            .getOrElse { "Could not open Google Play (${it.message})" }
        val name = if (pkg == ARTEMIS_PKG) "Artemis" else "Moonlight"
        val i = info(c)
        val addr = i?.let { bestAddress(it, phoneIps()) }
        val launch = c.packageManager.getLaunchIntentForPackage(pkg)?.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK) ?: return "$name would not open"
        if (addr != null) copy(c, addr)
        return runCatching {
            c.startActivity(launch)
            when {
                i == null -> "Opening $name. Open Aura PC > Remote > Laptop on phone once so this phone learns the PC's address"
                !i.running -> "Opening $name, but the PC's stream host is off. Turn on Laptop on phone in Aura PC"
                else -> "Opening your laptop in $name (PC address $addr copied: tap + and paste it the first time)"
            }
        }.getOrElse { "$name would not open (${it.message})" }
    }
}
