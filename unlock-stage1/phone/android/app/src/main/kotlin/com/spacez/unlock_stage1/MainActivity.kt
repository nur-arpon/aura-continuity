package com.spacez.unlock_stage1

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Bundle
import android.os.PowerManager
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import com.spacez.unlock_stage1.watchlink.WatchProtocol
import com.spacez.unlock_stage1.watchlink.WatchTrust
import io.flutter.plugin.common.MethodChannel

class MainActivity : FlutterActivity() {
    private val perms = arrayOf(
        Manifest.permission.BLUETOOTH_ADVERTISE,
        Manifest.permission.BLUETOOTH_CONNECT,
        Manifest.permission.POST_NOTIFICATIONS,
    )

    /** A home-screen shortcut (wake / lock / laptop / mouse) that opened us; the Dart side takes it once. */
    private var pendingShortcut: String? = null
    private fun noteShortcut(i: Intent?) { i?.getStringExtra("aura_shortcut")?.let { pendingShortcut = it; i.removeExtra("aura_shortcut") } }

    private fun pinShortcut(id: String): Boolean {
        val label = when (id) { "wake" -> "Wake PC"; "lock" -> "Lock PC"; "laptop" -> "Show laptop"; "mouse" -> "Touchpad"; else -> return false }
        val sm = getSystemService(android.content.pm.ShortcutManager::class.java) ?: return false
        if (!sm.isRequestPinShortcutSupported) return false
        val intent = Intent(this, MainActivity::class.java).setAction(Intent.ACTION_MAIN).putExtra("aura_shortcut", id)
        val info = android.content.pm.ShortcutInfo.Builder(this, "aura-$id")
            .setShortLabel(label).setLongLabel("Aura: $label")
            .setIcon(android.graphics.drawable.Icon.createWithResource(this, R.mipmap.ic_launcher))
            .setIntent(intent).build()
        return sm.requestPinShortcut(info, null)
    }

    private fun uiPrefs() = getSharedPreferences("aura_ui", MODE_PRIVATE)

    /** "Send file": whatever the system picker returns goes through the same path as the share sheet. */
    @Deprecated("Deprecated in Java")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode != 41 || resultCode != RESULT_OK || data == null) return
        val uris = ArrayList<android.net.Uri>()
        data.data?.let { uris += it }
        data.clipData?.let { cd -> for (k in 0 until cd.itemCount) uris += cd.getItemAt(k).uri }
        if (uris.isEmpty()) return
        val send = Intent(this, com.spacez.unlock_stage1.handoff.ShareToPcActivity::class.java)
            .setAction(Intent.ACTION_SEND_MULTIPLE).setType("*/*")
            .putParcelableArrayListExtra(Intent.EXTRA_STREAM, uris)
            .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        var clip: android.content.ClipData? = null
        uris.forEach { u -> if (clip == null) clip = android.content.ClipData.newRawUri("file", u) else clip!!.addItem(android.content.ClipData.Item(u)) }
        send.clipData = clip
        startActivity(send)
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        noteShortcut(intent)
        // first run: the onboarding asks for permissions at the step that explains them (item 13)
        if (UnlockCrypto.pcId(this) != null) askPermsThenStart()
    }

    // ---- item 25: "Allow everything" walk. The Dart side asks for one step at a time; the answer
    // comes back when the user returns to Aura (or the runtime dialog closes), so it can re-check.
    private var setupPending: MethodChannel.Result? = null
    private var setupLeft = false

    private fun setupAnswer(v: Any?) { setupPending?.let { setupPending = null; setupLeft = false; it.success(v) } }

    private fun setupRequest(id: String, appSettings: Boolean, result: MethodChannel.Result) {
        setupAnswer("replaced")
        val rt = com.spacez.unlock_stage1.setup.SetupPerms.RUNTIME[id]
        if (rt != null && !appSettings) {
            setupPending = result
            requestPermissions(rt, 8)
            return
        }
        val first = if (appSettings) com.spacez.unlock_stage1.setup.SetupPerms.fallback(this) else com.spacez.unlock_stage1.setup.SetupPerms.intent(this, id)
        setupPending = result; setupLeft = false
        if (runCatching { startActivity(first) }.isFailure &&
            runCatching { startActivity(com.spacez.unlock_stage1.setup.SetupPerms.fallback(this)) }.isFailure) setupAnswer("unavailable")
    }

    override fun onPause() {
        if (setupPending != null) setupLeft = true
        super.onPause()
    }

    override fun onResume() {
        super.onResume()
        com.spacez.unlock_stage1.handoff.ChatHead.attach(this)
        // Buds hand-off: one-time Shizuku grant, asked when the owner opens Aura (only while Shizuku is running)
        com.spacez.unlock_stage1.buds.BudsHandoff.requestPermission()
        if (setupPending != null && setupLeft) setupAnswer("back")
        com.spacez.unlock_stage1.update.AppUpdate.continueAfterSettings(this)
        if (intent?.getBooleanExtra(com.spacez.unlock_stage1.update.AppUpdate.EXTRA, false) == true) {
            intent.removeExtra(com.spacez.unlock_stage1.update.AppUpdate.EXTRA)
            com.spacez.unlock_stage1.update.AppUpdate.install(this)
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        noteShortcut(intent)
    }

    override fun onStart() {
        super.onStart()
        com.spacez.unlock_stage1.update.AppUpdate.scanDownloads(this)
        Feedback.warm(this)
        com.spacez.unlock_stage1.away.AwayWidget.updateAll(this)
    }

    override fun onStop() {
        Feedback.release()
        super.onStop()
    }

    private fun version(): String = runCatching {
        val i = packageManager.getPackageInfo(packageName, 0)
        "${i.versionName} (${i.longVersionCode})"
    }.getOrDefault("")

    private fun missing() = perms.filter { !PermCompat.has(this, it) }

    /**
     * Bluetooth-pair with the PC right after the code pairing. With the PC's address (from the QR) the phone asks for the
     * bond itself (Android shows "Pair?", Windows shows "Add a device" - one confirm each). Without it (typed code) and no
     * paired computer at all, open Bluetooth settings with a hint, since the link cannot work until the PC is paired.
     */
    @android.annotation.SuppressLint("MissingPermission")
    private fun bondPcIfNeeded(addr: String?) {
        runCatching {
            val ad = getSystemService(android.bluetooth.BluetoothManager::class.java)?.adapter?.takeIf { it.isEnabled } ?: return
            if (!PermCompat.btConnect(this)) return
            if (addr != null) {
                val d = ad.getRemoteDevice(addr)
                if (d.bondState == android.bluetooth.BluetoothDevice.BOND_NONE) {
                    UnlockState.log("Bluetooth pairing with the PC started ($addr)")
                    d.createBond()
                }
                return
            }
            val pcs = ad.bondedDevices.count { it.bluetoothClass?.majorDeviceClass == android.bluetooth.BluetoothClass.Device.Major.COMPUTER }
            if (pcs == 0) {
                android.widget.Toast.makeText(this, "Last step: pair this phone with your PC in Bluetooth (open Bluetooth on the PC too)", android.widget.Toast.LENGTH_LONG).show()
                startActivity(android.content.Intent(android.provider.Settings.ACTION_BLUETOOTH_SETTINGS))
            }
        }.onFailure { UnlockState.log("Bluetooth pairing hint failed: ${it.message}") }
    }

    /** Primary pairing (also a RE-pair of the primary slot with a new key). Dials the laptop right away. */
    private fun pairPrimary(code: String, role: String, addr: String?, name: String?): Boolean {
        val levels = UnlockCrypto.pair(this, code) ?: return false
        // "main" (unlocks the PC) or "extra" (share / control only): chosen on the pairing screen, shown on Home
        getSharedPreferences("cu_role", 0).edit().putString("role", role).apply()
        // a QR from "Add another phone" carries this PC's Bluetooth address (a phone never bonded through the unlock flow needs it)
        addr?.let { PcCompanion.rememberPc(this, it) }
        name?.let { PcState.setName(this, it) }
        UnlockState.log("paired with PC ${UnlockCrypto.pcId(this)?.toHex()}, levels: $levels")
        // the link runs over Bluetooth: a phone that was never Bluetooth-paired with this PC could not reach it
        // (the A50 case). Start the Bluetooth pairing now so the user only confirms it on both screens.
        bondPcIfNeeded(addr)
        Settings.setEnabled(this, true); askPermsThenStart()
        // new key: drop the old link and dial now, so the laptop sees this phone connect within seconds
        AuraPhone.repair(this)
        return true
    }

    private fun askPermsThenStart() {
        val m = missing()
        if (m.isNotEmpty()) requestPermissions(m.toTypedArray(), 7)
        else if (UnlockCrypto.pcId(this) != null && Settings.enabled(this)) UnlockService.start(this, "app opened")
    }

    // lazy location ask (first Wake / Remember this Wi-Fi): the answer goes back to Dart once the dialog closes
    private var locationPending: MethodChannel.Result? = null

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, grantResults: IntArray) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        if (requestCode == 9) locationPending?.let { locationPending = null; it.success(checkSelfPermission(Manifest.permission.ACCESS_FINE_LOCATION) == PackageManager.PERMISSION_GRANTED) }
        if (requestCode == 8) setupAnswer("back")
        if (UnlockCrypto.pcId(this) != null && Settings.enabled(this)) UnlockService.start(this, "app opened")
    }

    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)
        MethodChannel(flutterEngine.dartExecutor.binaryMessenger, "unlock").setMethodCallHandler { call, result ->
            when (call.method) {
                "status" -> {
                    val pm = getSystemService(PowerManager::class.java)
                    val p = UnlockState.pending.get()
                    result.success(mapOf(
                        "paired" to (UnlockCrypto.pcId(this) != null),
                        "running" to UnlockState.running,
                        "auto" to Settings.testModeOn(this),
                        "mode" to Settings.mode(this),
                        "enabled" to Settings.enabled(this),
                        "levels" to (getSharedPreferences("unlock", 0).getString("levels", "") ?: ""),
                        "advertising" to UnlockState.advertising,
                        "gatt" to UnlockState.gattUp,
                        "rfcomm" to UnlockState.rfcommUp,
                        "usb" to (UnlockState.tetherAddr ?: ""),
                        "loopback" to UnlockState.loopbackUp,
                        "answered" to UnlockState.answered,
                        "restarts" to UnlockState.restarts,
                        "batteryUnrestricted" to pm.isIgnoringBatteryOptimizations(packageName),
                        "companion" to PcCompanion.linked(this),
                        "companionSupported" to PcCompanion.supported(this),
                        "pcAddr" to (PcCompanion.pcAddress(this) ?: ""),
                        "watchPinnedFp" to (getSharedPreferences("cu_watch_trust", 0).getString("key", null)?.let {
                            WatchProtocol.fingerprint(android.util.Base64.decode(it, android.util.Base64.NO_WRAP)) } ?: ""),
                        "watchPendingFp" to (WatchTrust.pendingFingerprint(this) ?: ""),
                        "missingPerms" to missing(),
                        "pending" to (p?.transport ?: ""),
                        "pendingLevel" to (p?.level ?: -1),
                        "log" to UnlockState.logLines(),
                        // item 13: what the home screen shows about the PC
                        "pcName" to (PcState.name(this) ?: ""),
                        "pcNear" to PcState.near(this),
                        "pcLocked" to (PcState.locked(this)?.let { if (it) 1 else 0 } ?: -1),
                        "pcLockedAt" to PcState.lockedAt(this),
                        "lastSeen" to PcState.lastSeen(this),
                        "linkVia" to (AuraPhone.link?.takeIf { it.connected }?.status()?.substringAfter("session=")?.substringBefore('#') ?: ""),
                        "unlockBusy" to com.spacez.unlock_stage1.away.RemoteUnlockActivity.busy,
                        "inboxUnseen" to HandoffInbox.unseen(this),
                        "hapticLevel" to Feedback.intensity(this),
                        "sound" to Feedback.soundOn(this),
                        "uiLook" to uiPrefs().getString("look", "expressive"),
                        "uiSeed" to uiPrefs().getInt("seed", 0),
                        "laptops" to Laptops.describe(this, AuraPhone.primary),
                        "laptopPin" to (Laptops.pinned(this) ?: ""),
                        "version" to version(),
                        "play" to BuildConfig.PLAY,
                        "phoneRole" to (getSharedPreferences("cu_role", 0).getString("role", "main") ?: "main"),
                    ) + com.spacez.unlock_stage1.update.AppUpdate.info(this))
                }
                "pair" -> {
                    val addr = LaptopsLogic.cleanExtras(null, null, call.argument<String>("addr"), null).addr
                    result.success(pairPrimary(call.argument<String>("code") ?: "", if (call.argument<String>("role") == "extra") "extra" else "main", addr, null))
                }
                // many devices: further laptops as secondary companions. A laptop already in the list (same PC name /
                // Bluetooth address, or the primary slot of an "extra" phone) gets its key REPLACED, never a duplicate.
                "laptopAdd" -> {
                    val x = LaptopsLogic.cleanExtras("companion", call.argument<String>("name"), call.argument<String>("addr"), call.argument<String>("tailnet"))
                    val code = call.argument<String>("code") ?: ""
                    val newId = Laptops.idOfCode(code)
                    if (newId == null) { result.success(false) } else {
                        val role = getSharedPreferences("cu_role", 0).getString("role", "main") ?: "main"
                        val pid = UnlockCrypto.pcId(this)?.toHex()
                        val prim = pid?.let { LaptopsLogic.Ref(it, PcState.name(this), PcCompanion.pcAddress(this)) }
                        val plan = LaptopsLogic.plan(role, prim, Laptops.list(this).map { LaptopsLogic.Ref(it.id, it.name, it.addr) }, newId, x)
                        UnlockState.log("laptop code for ${x.name} ($newId): " + (if (plan.replacePrimary) "re-pair the primary slot" else "add") + (if (plan.dropSecondaries.isNotEmpty()) ", replace ${plan.dropSecondaries}" else ""))
                        if (plan.replacePrimary) {
                            plan.dropSecondaries.forEach { Laptops.remove(this, it) }
                            result.success(pairPrimary(code, "extra", x.addr, x.name))
                        } else {
                            plan.dropSecondaries.filter { it != newId }.forEach { Laptops.remove(this, it) }
                            val id = Laptops.add(this, code, x)
                            if (id != null) { Settings.setEnabled(this, true); UnlockService.start(this, "laptop added") }
                            result.success(id != null)
                        }
                    }
                }
                "laptopRemove" -> { Laptops.remove(this, call.argument<String>("id") ?: ""); result.success(true) }
                "laptopPin" -> { Laptops.setPinned(this, call.argument<String>("id")?.takeIf { it.isNotEmpty() }); result.success(true) }
                "unpair" -> {
                    UnlockCrypto.unpair(this)
                    getSharedPreferences("cu_role", 0).edit().clear().apply()
                    PcState.forget(this)
                    Settings.setEnabled(this, false)
                    stopService(Intent(this, UnlockService::class.java))
                    result.success(true)
                }
                "start" -> { Settings.setEnabled(this, true); askPermsThenStart(); result.success(true) }
                "stop" -> { Settings.setEnabled(this, false); stopService(Intent(this, UnlockService::class.java)); result.success(true) }
                "approve" -> {
                    val p = UnlockState.pending.get()
                    if (p?.level == UnlockCrypto.FINGERPRINT) startActivity(Intent(this, ApproveActivity::class.java))
                    else UnlockService.answer(UnlockCrypto.TAP)
                    result.success(true)
                }
                "watchConfirm" -> Thread { val ok = WatchTrust.confirmPending(this); runOnUiThread { result.success(ok) } }.start()
                "watchUnpair" -> { WatchTrust.unpair(this); result.success(true) }
                "linkCompanion" -> PcCompanion.associate(this) { msg -> UnlockState.log("companion: $msg"); result.success(msg) }
                "deny" -> { UnlockService.answer(UnlockCrypto.DENY); result.success(true) }
                "setMode" -> {
                    val m = call.argument<String>("mode") ?: "auto"
                    if (m in listOf("auto", "tap", "fingerprint")) Settings.setMode(this, m)
                    UnlockState.log("approval mode $m")
                    result.success(Settings.mode(this))
                }
                "setAuto" -> {
                    val on = call.argument<Boolean>("on") == true
                    Settings.setTestMode(this, on)
                    UnlockState.log("TEST MODE ${if (on) "ON for 30 min" else "off"}")
                    result.success(on)
                }
                // ---- Handoff (item 12)
                "handoff" -> result.success(mapOf(
                    "clipSync" to com.spacez.unlock_stage1.handoff.HandoffPhone.clipSync(this),
                    "suggest" to com.spacez.unlock_stage1.handoff.HandoffPhone.suggest(this),
                    "mediaAccess" to com.spacez.unlock_stage1.handoff.MediaWatch.granted(this),
                ) + com.spacez.unlock_stage1.handoff.ChatHead.status(this) + com.spacez.unlock_stage1.handoff.PcBubble.status(this) + com.spacez.unlock_stage1.handoff.LinkPrefs.status(this) + com.spacez.unlock_stage1.zerotap.ZeroTap.status(this).mapKeys { "zt_" + it.key })
                "continueOnPc" -> { startActivity(Intent(this, com.spacez.unlock_stage1.handoff.ContinueActivity::class.java)); result.success(true) }
                "sendClipboard" -> {
                    val t = runCatching { getSystemService(android.content.ClipboardManager::class.java).primaryClip?.takeIf { it.itemCount > 0 }?.getItemAt(0)?.coerceToText(this)?.toString() }.getOrNull()
                    if (t.isNullOrEmpty()) result.success("The clipboard is empty")
                    else Thread { val r = com.spacez.unlock_stage1.handoff.HandoffPhone.sendClipboardText(applicationContext, t); runOnUiThread { result.success(r) } }.start()
                }
                "mediaAccess" -> { startActivity(Intent(this, com.spacez.unlock_stage1.handoff.MediaAccessActivity::class.java)); result.success(true) }
                "setClipSync" -> { com.spacez.unlock_stage1.handoff.HandoffPhone.setClipSync(this, call.argument<Boolean>("on") == true); result.success(true) }
                "setSuggest" -> { com.spacez.unlock_stage1.handoff.HandoffPhone.setSuggest(this, call.argument<Boolean>("on") == true); result.success(true) }
                "setChatHead" -> {
                    val on = call.argument<Boolean>("on") == true
                    com.spacez.unlock_stage1.handoff.ChatHead.setEnabled(this, on)
                    if (on && !com.spacez.unlock_stage1.handoff.ChatHead.permitted(this)) runCatching { startActivity(com.spacez.unlock_stage1.handoff.ChatHead.permissionIntent(this)) }
                    result.success(true)
                }
                "chatHeadSettings" -> { runCatching { startActivity(com.spacez.unlock_stage1.handoff.ChatHead.permissionIntent(this)) }; result.success(true) }
                "setBubble" -> { com.spacez.unlock_stage1.handoff.PcBubble.setEnabled(this, call.argument<Boolean>("on") == true); result.success(true) }
                "testBubble" -> { result.success(com.spacez.unlock_stage1.handoff.PcBubble.test(this)) }
                "bubbleSettings" -> { runCatching { startActivity(com.spacez.unlock_stage1.handoff.PcBubble.settingsIntent(this)) }; result.success(true) }
                "setYtHandler" -> { com.spacez.unlock_stage1.handoff.LinkPrefs.setYtHandler(this, call.argument<String>("pkg") ?: ""); result.success(true) }
                "setLinkMode" -> {
                    com.spacez.unlock_stage1.handoff.LinkLogic.Mode.parse(call.argument<String>("mode"))?.let { com.spacez.unlock_stage1.handoff.LinkPrefs.set(this, call.argument<String>("app") ?: "", it) }
                    result.success(true)
                }
                // ---- Aura Anywhere (item 17)
                "away" -> result.success(com.spacez.unlock_stage1.away.AwayPhone.status(this))
                "awayFetch" -> { com.spacez.unlock_stage1.away.AwayPhone.autoFetch(this, "app opened"); result.success(true) }
                "awayRefresh" -> Thread { val r = com.spacez.unlock_stage1.away.AwayPhone.fetch(applicationContext, "app refresh"); runOnUiThread { result.success(r) } }.start()
                "awayOpenRecent" -> { com.spacez.unlock_stage1.away.AwayPhone.userAction(); val ix = (call.argument<Number>("index") ?: -1).toInt(); Thread { val r = com.spacez.unlock_stage1.away.AwayPhone.openRecent(applicationContext, ix, com.spacez.unlock_stage1.handoff.LinkLogic.Mode.parse(call.argument<String>("mode"))); runOnUiThread { result.success(r) } }.start() }
                "awayContinue" -> { com.spacez.unlock_stage1.away.AwayPhone.userAction(); Thread { val r = com.spacez.unlock_stage1.away.AwayPhone.continueFromLaptop(applicationContext); runOnUiThread { result.success(r) } }.start() }
                // ---- remote power (Wake / Sleep / Restart / Shut down)
                "locationEnsure" -> {
                    if (checkSelfPermission(Manifest.permission.ACCESS_FINE_LOCATION) == PackageManager.PERMISSION_GRANTED) result.success(true)
                    else { locationPending?.success(false); locationPending = result; requestPermissions(arrayOf(Manifest.permission.ACCESS_FINE_LOCATION), 9) }
                }
                "wakePc" -> { com.spacez.unlock_stage1.away.AwayPhone.userAction(70_000); Thread { val r = com.spacez.unlock_stage1.extras.PcWake.wake(applicationContext); runOnUiThread { result.success(r) } }.start() }
                "pcPower" -> { val a = call.argument<String>("action") ?: ""; Thread { val r = com.spacez.unlock_stage1.extras.PcWake.power(applicationContext, a); runOnUiThread { result.success(r) } }.start() }
                "wakeStatus" -> result.success(com.spacez.unlock_stage1.extras.PcWake.status(this))
                "wakeRemember" -> result.success(com.spacez.unlock_stage1.extras.PcWake.rememberCurrent(this))
                "wakeForget" -> { com.spacez.unlock_stage1.extras.PcWake.forget(this, call.argument<String>("ssid") ?: ""); result.success(true) }
                "awayLock" -> Thread { val r = com.spacez.unlock_stage1.away.AwayPhone.lockPc(applicationContext); runOnUiThread { result.success(r) } }.start()
                // ---- Zero-tap continue (item 19)
                "zeroTap" -> result.success(com.spacez.unlock_stage1.zerotap.ZeroTap.status(this))
                "zeroTapSet" -> { com.spacez.unlock_stage1.zerotap.ZeroTap.set(this, call.argument<String>("key") ?: "", call.argument<Boolean>("on") == true); result.success(true) }
                "zeroTapAfter" -> { com.spacez.unlock_stage1.zerotap.ZeroTap.setAfterContinue(this, call.argument<String>("v") ?: "pause"); result.success(true) }
                "zeroTapSetup" -> { startActivity(Intent(this, com.spacez.unlock_stage1.zerotap.ZeroTapSetupActivity::class.java)); result.success(true) }
                "awayUnlock" -> { startActivity(Intent(this, com.spacez.unlock_stage1.away.RemoteUnlockActivity::class.java)); result.success(true) }
                "tailscale" -> {
                    val i = if (com.spacez.unlock_stage1.away.Tailnet.installed(this)) com.spacez.unlock_stage1.away.Tailnet.openIntent(this) else com.spacez.unlock_stage1.away.Tailnet.installIntent(this)
                    runCatching { startActivity(i) }.onFailure { UnlockState.log("away: could not open Tailscale / Play Store: ${it.message}") }
                    result.success(true)
                }
                "setTsAutoOff" -> { com.spacez.unlock_stage1.away.TailnetAutoOff.setEnabled(this, call.argument<Boolean>("on") != false); result.success(true) }
                "vpnSettings" -> { runCatching { startActivity(Intent(android.provider.Settings.ACTION_VPN_SETTINGS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) }; result.success(true) }
                // ---- Laptop apps picker: the PC's installed apps with icons, pin one / open one
                "laptopApps" -> result.success(com.spacez.unlock_stage1.handoff.LaptopCatalog.list(this).map { mapOf("id" to it.id, "name" to it.name, "recent" to it.recent, "icon" to com.spacez.unlock_stage1.handoff.LaptopCatalog.iconBytes(this, it.id)) })
                "laptopAppsAsk" -> { com.spacez.unlock_stage1.handoff.LaptopCatalog.ask(this); result.success(true) }
                "pinLaptopApp" -> result.success(runCatching { com.spacez.unlock_stage1.handoff.LaptopCatalog.pin(this, call.argument<String>("id") ?: "") }.getOrDefault(false))
                "openLaptopApp" -> {
                    val id = call.argument<String>("id") ?: ""
                    startActivity(com.spacez.unlock_stage1.handoff.LaptopCatalog.launchIntent(this, id, call.argument<String>("name") ?: id))
                    result.success(true)
                }
                "takeShortcut" -> { val v = pendingShortcut; pendingShortcut = null; result.success(v) }
                "pinShortcut" -> result.success(runCatching { pinShortcut(call.argument<String>("id") ?: "") }.getOrDefault(false))
                // straight into the Desktop stream (no host list), switching Tailscale on first when away with it off
                "openLaptop" -> result.success(com.spacez.unlock_stage1.extras.Moonlight.openDesktop(this))
                // ---- Aura app (item 13): feedback, inbox, onboarding helpers
                "fx" -> { Feedback.play(this, call.argument<String>("cue") ?: "tap"); result.success(true) }
                "setHaptics" -> { Feedback.setIntensity(this, call.argument<Int>("level") ?: 3); result.success(Feedback.intensity(this)) }
                "setUi" -> {
                    uiPrefs().edit().apply {
                        call.argument<String>("look")?.let { putString("look", it) }
                        call.argument<Number>("seed")?.let { putInt("seed", it.toInt()) }
                    }.apply()
                    result.success(true)
                }
                "pickFile" -> {
                    runCatching {
                        startActivityForResult(Intent(Intent.ACTION_OPEN_DOCUMENT).addCategory(Intent.CATEGORY_OPENABLE)
                            .setType("*/*").putExtra(Intent.EXTRA_ALLOW_MULTIPLE, true), 41)
                    }.onFailure { UnlockState.log("pickFile: ${it.message}") }
                    result.success(true)
                }
                "setSound" -> { Feedback.setSound(this, call.argument<Boolean>("on") == true); result.success(Feedback.soundOn(this)) }
                "inbox" -> result.success(HandoffInbox.all(this).map { it.map() })
                "inboxSeen" -> { HandoffInbox.markSeen(this); result.success(true) }
                "inboxRemove" -> { HandoffInbox.remove(this, (call.argument<Number>("id"))?.toLong() ?: 0); result.success(true) }
                "inboxClear" -> { HandoffInbox.clear(this); result.success(true) }
                "inboxOpen" -> {
                    val it = HandoffInbox.get(this, (call.argument<Number>("id"))?.toLong() ?: 0)
                    val i = when {
                        it == null -> null
                        it.url != null -> com.spacez.unlock_stage1.handoff.HandoffPhone.viewIntent(this, it.url, null)
                        it.uri != null -> Intent(Intent.ACTION_VIEW).setDataAndType(android.net.Uri.parse(it.uri), it.mime?.ifEmpty { null } ?: "*/*")
                            .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
                        else -> null
                    }
                    if (i == null) result.success(if (it?.kind == "clip") "It is on the clipboard: long-press any text field and Paste" else "Nothing to open")
                    else result.success(runCatching { startActivity(i); "ok" }.getOrElse { "No app can open this (${it.message})" })
                }
                "setupStatus" -> {
                    val g = com.spacez.unlock_stage1.setup.SetupPerms.granted(this)
                    result.success(com.spacez.unlock_stage1.setup.SetupPerms.available().map { mapOf("id" to it, "granted" to (g[it] == true)) })
                }
                "setupRequest" -> setupRequest(call.argument<String>("id") ?: "", call.argument<Boolean>("appSettings") == true, result)
                "watchSetup" -> Thread { val r = com.spacez.unlock_stage1.setup.SetupPerms.watch(applicationContext); runOnUiThread { result.success(r) } }.start()
                "watchInstall" -> {
                    val ok = runCatching { startActivity(com.spacez.unlock_stage1.setup.SetupPerms.playIntent(this)) }.isSuccess ||
                        runCatching { startActivity(com.spacez.unlock_stage1.setup.SetupPerms.playWebIntent(this)) }.isSuccess
                    result.success(ok)
                }
                "requestPerms" -> { askPermsThenStart(); result.success(missing().isEmpty()) }
                "batterySettings" -> {
                    // the app's own page: Battery > Unrestricted (no extra permission needed)
                    val i = Intent(android.provider.Settings.ACTION_APPLICATION_DETAILS_SETTINGS, android.net.Uri.parse("package:$packageName"))
                    runCatching { startActivity(i) }
                    result.success(true)
                }
                // ---- item 15: ring / media / presenter / Extend Unlock guide
                "remoteState" -> result.success(com.spacez.unlock_stage1.extras.PcRemote.status() + mapOf(
                    "allowRing" to com.spacez.unlock_stage1.extras.PhoneRinger.allowed(this)))
                "ringPc" -> Thread { val r = com.spacez.unlock_stage1.extras.PcRemote.ringPc(applicationContext, call.argument<Boolean>("on") == true); runOnUiThread { result.success(r) } }.start()
                "mediaCmd" -> Thread { val r = com.spacez.unlock_stage1.extras.PcRemote.mediaCmd(applicationContext, call.argument<Int>("cmd") ?: 0); runOnUiThread { result.success(r ?: "ok") } }.start()
                "present" -> Thread { val r = com.spacez.unlock_stage1.extras.PcRemote.present(applicationContext, call.argument<Int>("cmd") ?: 0); runOnUiThread { result.success(r ?: "ok") } }.start()
                "remoteWatch" -> Thread { val r = com.spacez.unlock_stage1.extras.PcRemote.watch(applicationContext); runOnUiThread { result.success(r ?: "ok") } }.start()
                "remoteStop" -> { com.spacez.unlock_stage1.extras.PcRemote.unwatch(applicationContext); com.spacez.unlock_stage1.extras.PcRemote.setPresenterKeys(false); result.success(true) }
                "input" -> { com.spacez.unlock_stage1.extras.PcRemote.input(applicationContext, call.argument<String>("json") ?: "{}"); result.success(true) }
                "presenterKeys" -> { com.spacez.unlock_stage1.extras.PcRemote.setPresenterKeys(call.argument<Boolean>("on") == true); result.success(true) }
                "setAllowRing" -> { com.spacez.unlock_stage1.extras.PhoneRinger.setAllowed(this, call.argument<Boolean>("on") == true); result.success(true) }
                "stopRinging" -> { com.spacez.unlock_stage1.extras.PhoneRinger.stop(this, "app"); result.success(true) }
                "installUpdate" -> result.success(com.spacez.unlock_stage1.update.AppUpdate.install(this))
                "openSettings" -> {
                    val action = when (call.argument<String>("page")) {
                        "bluetooth" -> android.provider.Settings.ACTION_BLUETOOTH_SETTINGS
                        else -> android.provider.Settings.ACTION_SECURITY_SETTINGS
                    }
                    result.success(runCatching { startActivity(Intent(action)); "ok" }.getOrElse { "Open Settings yourself: ${it.message}" })
                }
                else -> result.notImplemented()
            }
        }
    }

    /** Presentation remote: while its screen is open, the volume keys are Next / Previous. */
    override fun dispatchKeyEvent(event: android.view.KeyEvent): Boolean {
        if (com.spacez.unlock_stage1.extras.PcRemote.presenterKeysOn() &&
            (event.keyCode == android.view.KeyEvent.KEYCODE_VOLUME_UP || event.keyCode == android.view.KeyEvent.KEYCODE_VOLUME_DOWN)) {
            if (event.action == android.view.KeyEvent.ACTION_DOWN && event.repeatCount == 0) {
                val cmd = if (event.keyCode == android.view.KeyEvent.KEYCODE_VOLUME_DOWN) com.spacez.aura.link.PresentCmd.NEXT else com.spacez.aura.link.PresentCmd.PREV
                Feedback.play(this, "slide")
                Thread { com.spacez.unlock_stage1.extras.PcRemote.present(applicationContext, cmd) }.start()
            }
            return true
        }
        return super.dispatchKeyEvent(event)
    }
}
