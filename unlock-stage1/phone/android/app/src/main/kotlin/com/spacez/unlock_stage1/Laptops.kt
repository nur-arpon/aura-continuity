package com.spacez.unlock_stage1

import android.annotation.SuppressLint
import android.bluetooth.BluetoothManager
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import android.security.keystore.KeyProperties
import android.security.keystore.KeyProtection
import com.spacez.aura.link.AuraCrypto
import com.spacez.aura.link.AuraLink
import com.spacez.aura.link.Caps
import com.spacez.aura.link.Conn
import com.spacez.aura.link.Credential
import com.spacez.aura.link.FastRole
import com.spacez.aura.link.Kind
import com.spacez.aura.link.LinkConfig
import com.spacez.aura.link.Message
import com.spacez.aura.link.Role
import com.spacez.aura.link.raceConnect
import com.spacez.unlock_stage1.away.Tailnet
import org.json.JSONArray
import org.json.JSONObject
import javax.crypto.Mac
import javax.crypto.SecretKey
import javax.crypto.spec.SecretKeySpec
import java.security.KeyStore

/**
 * Many devices, phone side (phase 1). The PAIRED laptop from the unlock flow stays the "primary"
 * ([AuraPhone.primary]); further laptops are SECONDARY companions: the phone holds only a level-0 HMAC key per
 * laptop (Keystore alias `cus-<pcId>`, no auth, no unlock levels), so a secondary laptop can never ask this phone
 * to unlock anything. Features that act on "the laptop" (Continue, clipboard, share, chat head) ask
 * [AuraPhone.link] which is now [target]: the connected laptop most recently linked, else the last linked one,
 * else the primary. With no secondary laptop it is exactly the primary link, as before.
 */
object LaptopsLogic {
    /** One laptop as the picker sees it. */
    data class Cand(val id: String, val connected: Boolean, val lastUpMs: Long, val addedMs: Long)

    /**
     * Which laptop an action goes to. [pinned] (the user chose one in Settings) wins while it exists.
     * Otherwise: among CONNECTED laptops the one whose session came up most recently ("nearest"), else the one
     * most recently linked or connected. Ties go to the first in [c] (the primary is listed first).
     */
    fun pick(c: List<Cand>, pinned: String?): String? {
        if (c.isEmpty()) return null
        if (pinned != null && c.any { it.id == pinned }) return pinned
        val up = c.filter { it.connected }
        val pool = up.ifEmpty { c }
        return pool.maxWithOrNull(compareBy<Cand> { maxOf(it.lastUpMs, if (up.isEmpty()) it.addedMs else 0L) })?.id
    }

    /** Pairing QR fields beyond the code: role, PC name, Bluetooth address, tailnet address. */
    data class QrExtras(val role: String?, val name: String?, val addr: String?, val tailnet: String?)

    /** A laptop as the matcher sees it: the primary slot (name from the link, address learned/typed) or a secondary. */
    data class Ref(val id: String, val name: String?, val addr: String?)

    /** What to do with a scanned code: re-pair the primary slot, forget these duplicate secondaries, add a new one. */
    data class Plan(val replacePrimary: Boolean, val dropSecondaries: List<String>, val addAsSecondary: Boolean)

    /** Same physical PC? Both Bluetooth addresses known: they decide. Otherwise the (non-blank) names decide. */
    fun samePc(aName: String?, aAddr: String?, bName: String?, bAddr: String?): Boolean {
        if (!aAddr.isNullOrBlank() && !bAddr.isNullOrBlank()) return aAddr.equals(bAddr, ignoreCase = true)
        return !aName.isNullOrBlank() && !bName.isNullOrBlank() && aName.trim().equals(bName.trim(), ignoreCase = true)
    }

    /**
     * Scanning/typing a code for a laptop already in the list REPLACES its key instead of adding a duplicate.
     * [role] is this phone's role ("main"/"extra"); an "extra" phone's primary slot is just a companion link, so a
     * companion QR for the same PC re-pairs that slot (role stays extra) and any secondary duplicate of it is dropped.
     */
    fun plan(role: String, primary: Ref?, secondaries: List<Ref>, newId: String, qr: QrExtras): Plan {
        val dups = secondaries.filter { it.id == newId || samePc(it.name, it.addr, qr.name, qr.addr) }.map { it.id }
        if (role == "extra" && primary != null && (primary.id == newId || samePc(primary.name, primary.addr, qr.name, qr.addr)))
            return Plan(true, dups, false)
        return Plan(false, dups, true)
    }

    private val MAC = Regex("^[0-9A-Fa-f]{2}(:[0-9A-Fa-f]{2}){5}$")
    private val IP4 = Regex("^100\\.(6[4-9]|[7-9][0-9]|1[01][0-9]|12[0-7])\\.\\d{1,3}\\.\\d{1,3}$")
    fun cleanExtras(role: String?, name: String?, addr: String?, tailnet: String?) = QrExtras(
        role?.takeIf { it == "companion" },
        name?.filter { !it.isISOControl() }?.take(40)?.trim()?.ifEmpty { null },
        addr?.takeIf { MAC.matches(it) }?.uppercase(),
        tailnet?.takeIf { IP4.matches(it) },
    )
}

data class Laptop(val id: String, val name: String, val addr: String?, val tailnetIp: String?, val addedMs: Long)

object Laptops {
    private const val PREFS = "laptops"
    private const val ALIAS = "cus-"
    private fun p(c: Context) = c.getSharedPreferences(PREFS, 0)
    private val links = HashMap<String, AuraLink>()
    private val lastUp = HashMap<String, Long>()

    private fun ks() = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }

    fun list(c: Context): List<Laptop> = runCatching {
        val a = JSONArray(p(c).getString("list", "[]"))
        (0 until a.length()).map { a.getJSONObject(it) }.map {
            Laptop(it.getString("id"), it.optString("name", "Laptop"), it.optString("addr").ifEmpty { null }, it.optString("tn").ifEmpty { null }, it.optLong("added"))
        }
    }.getOrDefault(emptyList())

    private fun save(c: Context, l: List<Laptop>) {
        val a = JSONArray()
        l.forEach { a.put(JSONObject().put("id", it.id).put("name", it.name).put("addr", it.addr ?: "").put("tn", it.tailnetIp ?: "").put("added", it.addedMs)) }
        p(c).edit().putString("list", a.toString()).apply()
    }

    /** "auto" (null) or the id of the laptop the user pinned as the target. */
    fun pinned(c: Context): String? = p(c).getString("pin", null)?.takeIf { it.isNotEmpty() }
    fun setPinned(c: Context, id: String?) = p(c).edit().putString("pin", id ?: "").apply()

    fun idOfCode(code: String): String? = UnlockCrypto.decodeCode(code)?.let { s -> UnlockCrypto.pcIdOf(s).toHex().also { s.fill(0) } }

    /** Queue a harmless ping so the link dials now (a link only connects while something waits to be sent). */
    fun dial(c: Context, id: String) { ensure(c, id)?.send(Message.text("ping:dial")) }

    /** Pair a SECONDARY laptop from its code. Returns its id, or null for a bad code / the primary's own code. */
    fun add(c: Context, code: String, extras: LaptopsLogic.QrExtras): String? {
        val secret = UnlockCrypto.decodeCode(code) ?: return null
        val idBytes = UnlockCrypto.pcIdOf(secret)
        val id = idBytes.toHex()
        if (id == UnlockCrypto.pcId(c)?.toHex()) { secret.fill(0); return null }
        val k0 = UnlockCrypto.levelKey(secret, 0)
        secret.fill(0)
        val ok = runCatching {
            ks().setEntry("$ALIAS$id", KeyStore.SecretKeyEntry(SecretKeySpec(k0, "HmacSHA256")), KeyProtection.Builder(KeyProperties.PURPOSE_SIGN).build())
        }.isSuccess
        k0.fill(0)
        if (!ok) return null
        val cur = list(c).filter { it.id != id }
        save(c, cur + Laptop(id, extras.name ?: "Laptop", extras.addr, extras.tailnet, System.currentTimeMillis()))
        UnlockState.log("laptop added: ${extras.name} ($id) as a companion")
        dial(c, id)
        return id
    }

    fun remove(c: Context, id: String) {
        links.remove(id)?.stop()
        runCatching { ks().deleteEntry("$ALIAS$id") }
        save(c, list(c).filter { it.id != id })
        if (pinned(c) == id) setPinned(c, null)
        lastUp.remove(id)
        UnlockState.log("laptop removed: $id")
    }

    private fun mac(id: String): Mac? = runCatching {
        val key = ks().getKey("$ALIAS$id", null) as SecretKey? ?: return null
        Mac.getInstance("HmacSHA256").apply { init(key) }
    }.getOrNull()

    /** The link for one secondary laptop (created on demand). */
    @Synchronized
    fun ensure(ctx: Context, id: String): AuraLink? {
        links[id]?.let { return it }
        val app = ctx.applicationContext
        val lap = list(app).firstOrNull { it.id == id } ?: return null
        val m = mac(id) ?: return null.also { UnlockState.log("laptop $id: no key") }
        val cfg = LinkConfig(Role.PHONE, Role.PC, id.fromHex(), Credential.Psk(AuraCrypto.pskFromMac(m)), AuraPhone.inbox(app),
            caps = Caps.FILES or Caps.FAST_SERVER or Caps.MEDIA or Caps.RING, name = Build.MODEL, fastRole = FastRole.SERVER)
        val l = AuraLink(cfg, { raceConnect({ connectBt(app, id) }, { tailnet(app, id) }, 1_500) }, SecondaryHandler(app, id))
        links[id] = l
        UnlockState.log("laptop ${lap.name}: link ready")
        return l
    }

    fun ensureAll(c: Context) = list(c).forEach { dial(c, it.id) }

    fun linkOf(id: String): AuraLink? = links[id]

    fun picked(l: AuraLink): String? = links.entries.firstOrNull { it.value === l }?.key

    /** Incoming RFCOMM / tailnet connection from a PC: the link whose pc_id the PC's HELLO carries. */
    fun pickIncoming(pcId: ByteArray): AuraLink? = links[pcId.toHex()]

    fun sessionUp(id: String) { synchronized(this) { lastUp[id] = System.currentTimeMillis() } }

    /** The Aura Link the phone should use for "the laptop" right now. */
    fun target(c: Context, primary: AuraLink?): AuraLink? {
        val lap = list(c)
        if (lap.isEmpty()) return primary
        val primaryId = UnlockCrypto.pcId(c)?.toHex()
        val cands = ArrayList<LaptopsLogic.Cand>()
        if (primary != null && primaryId != null) cands += LaptopsLogic.Cand(primaryId, primary.connected, synchronized(this) { lastUp[primaryId] ?: 0L }, 0L)
        for (l in lap) cands += LaptopsLogic.Cand(l.id, links[l.id]?.connected == true, synchronized(this) { lastUp[l.id] ?: 0L }, l.addedMs)
        val id = LaptopsLogic.pick(cands, pinned(c)) ?: return primary
        return if (id == primaryId) primary else links[id] ?: primary
    }

    fun nameOf(c: Context, id: String): String = list(c).firstOrNull { it.id == id }?.name ?: "Laptop"

    /** For the Settings screen: the primary first, then the secondaries. */
    fun describe(c: Context, primary: AuraLink?): List<Map<String, Any?>> {
        val out = ArrayList<Map<String, Any?>>()
        val pid = UnlockCrypto.pcId(c)?.toHex()
        if (pid != null) out += mapOf("id" to pid, "name" to (PcState.name(c) ?: "Your PC"), "primary" to true, "connected" to (primary?.connected == true))
        list(c).forEach { out += mapOf("id" to it.id, "name" to it.name, "primary" to false, "connected" to (links[it.id]?.connected == true)) }
        return out
    }

    @SuppressLint("MissingPermission")
    private fun connectBt(ctx: Context, id: String): Conn {
        if (!PermCompat.btConnect(ctx)) throw IllegalStateException("no Bluetooth permission")
        val ad = ctx.getSystemService(BluetoothManager::class.java)?.adapter?.takeIf { it.isEnabled } ?: throw IllegalStateException("Bluetooth off")
        val known = list(ctx).firstOrNull { it.id == id }?.addr
        var last: Exception? = null
        if (known != null) try {
            val s = ad.getRemoteDevice(known).createRfcommSocketToServiceRecord(AuraPhone.AURA_UUID)
            s.connect()
            return Conn(s.inputStream, s.outputStream, { runCatching { s.close() } }, "rfcomm")
        } catch (e: Exception) { last = e }
        // no address in the QR (typed code) or it failed: try the other paired computers (ones another laptop owns last);
        // the first that answers is remembered for this laptop
        val owned = list(ctx).filter { it.id != id }.mapNotNull { it.addr?.uppercase() } + listOfNotNull(PcCompanion.pcAddress(ctx))
        val pcs = ad.bondedDevices.filter { it.bluetoothClass?.majorDeviceClass == android.bluetooth.BluetoothClass.Device.Major.COMPUTER && it.address.uppercase() != known?.uppercase() }
            .sortedBy { it.address.uppercase() in owned }
        if (known == null && pcs.isEmpty()) throw IllegalStateException("laptop Bluetooth address not known (pair the laptop in Bluetooth settings)")
        for (d in pcs) {
            val s = runCatching { d.createRfcommSocketToServiceRecord(AuraPhone.AURA_UUID) }.getOrNull() ?: continue
            try {
                s.connect()
                val a = d.address.uppercase()
                synchronized(this) { save(ctx, list(ctx).map { if (it.id == id) it.copy(addr = a) else it }) }
                return Conn(s.inputStream, s.outputStream, { runCatching { s.close() } }, "rfcomm")
            } catch (e: Exception) { last = e; runCatching { s.close() } }
        }
        throw IllegalStateException("no paired laptop answered (${last?.message})")
    }

    /** Tailnet leg: only right after a user action (same gate as the primary), to the address the QR carried. */
    private fun tailnet(ctx: Context, id: String): Conn? {
        if (!com.spacez.unlock_stage1.away.AwayPhone.gate.isOpen(System.currentTimeMillis())) return null
        val ip = list(ctx).firstOrNull { it.id == id }?.tailnetIp ?: return null
        if (Tailnet.ownIp() == null) return null
        return Tailnet.connect(ip, com.spacez.unlock_stage1.away.AwayLogic.PORT, 3_000)
    }

    /** What a secondary laptop may do to this phone: share, continue, ring. Never unlock, never the primary's state. */
    private class SecondaryHandler(private val ctx: Context, private val id: String) : com.spacez.aura.link.LinkHandler {
        override fun log(line: String) { if (!line.contains("FILE_CHUNK")) UnlockState.log("laptop ${id.take(6)} $line") }
        override fun onSession(link: AuraLink, up: Boolean) {
            if (up) { sessionUp(id); link.peer?.name?.takeIf { it.isNotEmpty() }?.let { n -> save(ctx, list(ctx).map { if (it.id == id && it.name == "Laptop") it.copy(name = n) else it }) } }
            UnlockState.log("laptop ${nameOf(ctx, id)} ${if (up) "UP" else "down"}")
        }
        override fun onFile(link: AuraLink, file: java.io.File, offer: com.spacez.aura.link.FileOffer) {
            com.spacez.unlock_stage1.handoff.HandoffPhone.onFile(ctx, file, offer)
        }
        override fun onMessage(link: AuraLink, m: Message): String? {
            when (m.kind) {
                Kind.TEXT -> { val t = String(m.body, Charsets.UTF_8); if (t.startsWith("ping:")) link.send(Message.text("pong:" + t.removePrefix("ping:"))) }
                Kind.URL -> { val (u, pos) = Message.parseUrl(m.body); com.spacez.unlock_stage1.handoff.HandoffPhone.onUrl(ctx, u, pos) }
                Kind.CLIPBOARD -> { val (mime, data) = Message.parseClipboard(m.body); com.spacez.unlock_stage1.handoff.HandoffPhone.onClipboard(ctx, mime, data) }
                Kind.ACTIVITY -> com.spacez.unlock_stage1.handoff.HandoffPhone.onActivity(ctx, String(m.body, Charsets.UTF_8))
                // the chat head follows the picked laptop: only that one may feed it its apps
                Kind.LAPTOP_APPS -> if (AuraPhone.link === link) com.spacez.unlock_stage1.handoff.ChatHead.onLaptopApps(ctx, String(m.body, Charsets.UTF_8))
                Kind.RING -> com.spacez.unlock_stage1.extras.PcRemote.onRingFromPc(ctx, m.body.firstOrNull()?.toInt() == 1)
                else -> {} // everything else is the primary laptop's single-PC state (lock state, tailnet, media): ignored here
            }
            return null
        }
    }
}
