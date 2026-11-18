package com.spacez.unlock_stage1.away

import android.app.Activity
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.hardware.biometrics.BiometricManager
import android.hardware.biometrics.BiometricPrompt
import android.os.Build
import android.os.Bundle
import android.os.CancellationSignal
import android.service.quicksettings.Tile
import android.service.quicksettings.TileService
import android.widget.Toast
import com.spacez.unlock_stage1.Feedback
import com.spacez.unlock_stage1.FeedbackPlan.Cue
import com.spacez.unlock_stage1.PcState
import com.spacez.unlock_stage1.UnlockCrypto
import com.spacez.unlock_stage1.UnlockState
import kotlin.concurrent.thread

/**
 * Remote "Unlock PC" (item 17): the ONLY way the app signs a remote unlock. A strong-biometric
 * BiometricPrompt every time; its CryptoObject releases the Keystore FINGERPRINT key for exactly
 * one MAC, which signs the frame. Started only by a tap on Unlock PC (app, widget); never by a
 * message, a timer or the PC. No UI of its own besides the system prompt and a toast.
 */
class RemoteUnlockActivity : Activity() {
    companion object {
        /** true while a signed unlock is on its way (the app shows "Unlocking…"). */
        @Volatile private var busySince = 0L
        /** Never stuck: a signed unlock resolves within ~40 s; after 45 s the button resets regardless. */
        var busy: Boolean
            get() = busySince > 0 && android.os.SystemClock.elapsedRealtime() - busySince < 45_000
            set(v) { busySince = if (v) android.os.SystemClock.elapsedRealtime() else 0L }
    }
    private val cancel = CancellationSignal()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val mac = UnlockCrypto.fingerprintMacUninit()
        if (UnlockCrypto.pcId(this) == null || mac == null) {
            Feedback.play(this, Cue.ERROR)
            toast(if (mac == null) "Unlock PC needs a fingerprint set up on this phone" else "Pair with your PC first")
            finish(); return
        }
        AwayPhone.userAction(90_000) // lets the link warm up (Bluetooth or tailnet) while the finger is on the sensor
        BiometricPrompt.Builder(this)
            .setTitle("Unlock your PC?")
            .setSubtitle("Approves the PC's sign-in screen once. Works from anywhere.")
            .setAllowedAuthenticators(BiometricManager.Authenticators.BIOMETRIC_STRONG)
            .setNegativeButton("Cancel", mainExecutor) { _, _ -> finish() }
            .build()
            .authenticate(BiometricPrompt.CryptoObject(mac), cancel, mainExecutor, object : BiometricPrompt.AuthenticationCallback() {
                override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
                    val m = result.cryptoObject?.mac ?: return finish()
                    Feedback.play(this@RemoteUnlockActivity, Cue.UNLOCK)
                    toast("Unlocking your PC…")
                    val app = applicationContext
                    busy = true
                    thread(isDaemon = true) {
                        val r = try { AwayPhone.unlockPc(app, m) } finally { busy = false }
                        if (AwayPhone.lastResultOk()) Feedback.play(app, Cue.SUCCESS)
                        else {
                            // strong error: two error cues, so it is felt in a pocket too
                            Feedback.play(app, Cue.ERROR)
                            android.os.Handler(app.mainLooper).postDelayed({ Feedback.play(app, Cue.ERROR) }, 350)
                        }
                        android.os.Handler(app.mainLooper).post { Toast.makeText(app, r, Toast.LENGTH_LONG).show() }
                    }
                    finish()
                }
                override fun onAuthenticationError(errorCode: Int, errString: CharSequence) {
                    UnlockState.log("away: unlock prompt ended: $errString")
                    finish()
                }
            })
    }

    private fun toast(s: String) = Toast.makeText(applicationContext, s, Toast.LENGTH_SHORT).show()

    override fun onDestroy() {
        if (!cancel.isCanceled) cancel.cancel()
        super.onDestroy()
    }
}

/**
 * Widget / tile / app actions that need a short foreground moment (a network round trip):
 * "continue" (open what the PC showed, at the exact second), "lock" (remote Lock PC), "refresh".
 * Translucent, no UI besides a toast; finishes at once.
 */
class AwayActionActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val app = applicationContext
        val what = intent?.getStringExtra("do") ?: "continue"
        if (what == "unlock") { Feedback.play(app, Cue.TAP); startActivity(Intent(this, RemoteUnlockActivity::class.java)); finish(); return }
        if (what == "topc") {
            // Continue on PC needs a focused activity to read the clipboard (Android 10+): hand over to it
            startActivity(Intent(this, com.spacez.unlock_stage1.handoff.ContinueActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            finish(); return
        }
        if (UnlockCrypto.pcId(app) == null) {
            Feedback.play(app, Cue.ERROR)
            Toast.makeText(app, "Open Aura and pair with your PC first", Toast.LENGTH_SHORT).show()
            finish(); return
        }
        AwayPhone.userAction()
        // instant feedback on the tap itself; the result follows when the PC answers
        when (what) {
            "lock" -> { Feedback.play(app, Cue.LOCK); Toast.makeText(app, "Locking your PC…", Toast.LENGTH_SHORT).show() }
            "wake" -> { Feedback.play(app, Cue.TAP); Toast.makeText(app, "Waking ${com.spacez.unlock_stage1.extras.PcWake.pcName(app)}…", Toast.LENGTH_SHORT).show() }
            "continue" -> { Feedback.play(app, Cue.TAP); Toast.makeText(app, "Asking your PC…", Toast.LENGTH_SHORT).show() }
            else -> Feedback.play(app, Cue.TICK)
        }
        thread(isDaemon = true) {
            val r = when (what) {
                "lock" -> AwayPhone.lockPc(app)
                "wake" -> com.spacez.unlock_stage1.extras.PcWake.wake(app)
                "refresh" -> AwayPhone.fetch(app, "widget refresh").let { if (it == "ok") "Updated" else it }
                else -> AwayPhone.continueFromLaptop(app)
            }
            val ok = r.startsWith("Lock sent") || r.startsWith("Opening") || r == "Updated" || r == "PC is awake"
            if (!ok) Feedback.play(app, Cue.ERROR) else if (what == "lock") Feedback.play(app, Cue.APPROVED)
            AwayWidget.updateAll(app)
            android.os.Handler(app.mainLooper).post { Toast.makeText(app, r, Toast.LENGTH_SHORT).show() }
        }
        finish()
    }

    companion object {
        fun intent(c: Context, what: String): Intent = Intent(c, AwayActionActivity::class.java).putExtra("do", what)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_NO_HISTORY)
        fun pending(c: Context, what: String, req: Int): PendingIntent =
            PendingIntent.getActivity(c, req, intent(c, what), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
    }
}

/**
 * Quick-settings tile "Continue from laptop". Opening the panel shows the last known activity and
 * asks the PC once (at most once a minute, only while the panel is open = a user action).
 */
class ContinueFromLaptopTile : TileService() {
    override fun onStartListening() {
        val l = AwayPhone.last(this)
        qsTile?.apply {
            state = if (UnlockCrypto.pcId(this@ContinueFromLaptopTile) == null) Tile.STATE_UNAVAILABLE else Tile.STATE_INACTIVE
            label = "From laptop"
            if (Build.VERSION.SDK_INT >= 29) subtitle = l?.title()?.take(22) ?: "Tap to continue"
            updateTile()
        }
        AwayPhone.autoFetch(this, "tile opened")
    }

    override fun onClick() {
        val go = {
            val i = AwayActionActivity.intent(this, "continue")
            if (Build.VERSION.SDK_INT >= 34) startActivityAndCollapse(PendingIntent.getActivity(this, 7, i, PendingIntent.FLAG_IMMUTABLE))
            else @Suppress("DEPRECATION") startActivityAndCollapse(i)
        }
        if (isLocked) unlockAndRun(go) else go()
    }
}

/** Starts [AwayActionActivity] from a tile, unlocking the keyguard first when needed. */
private fun TileService.runAction(what: String, req: Int) {
    val go = {
        val i = AwayActionActivity.intent(this, what)
        if (Build.VERSION.SDK_INT >= 34) startActivityAndCollapse(PendingIntent.getActivity(this, req, i, PendingIntent.FLAG_IMMUTABLE))
        else @Suppress("DEPRECATION") startActivityAndCollapse(i)
    }
    if (isLocked) unlockAndRun(go) else go()
}

/**
 * Quick-settings tile "Lock PC" (item 13). One tap = the same signed lock event as the app and
 * the widget (Bluetooth, else the tailnet). Shows the last known PC state as its subtitle.
 */
class LockPcTile : TileService() {
    override fun onStartListening() {
        val paired = UnlockCrypto.pcId(this) != null
        val locked = PcState.locked(this) == true
        qsTile?.apply {
            state = if (!paired) Tile.STATE_UNAVAILABLE else if (locked) Tile.STATE_ACTIVE else Tile.STATE_INACTIVE
            label = "Lock PC"
            if (Build.VERSION.SDK_INT >= 29) subtitle = when { !paired -> "Not paired"; locked -> "Locked"; else -> PcState.name(this@LockPcTile) ?: "Your PC" }
            updateTile()
        }
    }
    override fun onClick() = runAction("lock", 21)
}

/** Quick-settings tile "Unlock PC": fingerprint every time (RemoteUnlockActivity), never automatic. */
class UnlockPcTile : TileService() {
    override fun onStartListening() {
        val paired = UnlockCrypto.pcId(this) != null
        qsTile?.apply {
            state = if (!paired) Tile.STATE_UNAVAILABLE else Tile.STATE_INACTIVE
            label = "Unlock PC"
            if (Build.VERSION.SDK_INT >= 29) subtitle = if (!paired) "Not paired" else "With fingerprint"
            updateTile()
        }
    }
    override fun onClick() = runAction("unlock", 22)
}

/** Quick-settings tile "Wake PC": Wake-on-LAN packets (Wi-Fi broadcast + tailnet relay), then watches for the link. */
class WakePcTile : TileService() {
    override fun onStartListening() {
        val paired = UnlockCrypto.pcId(this) != null
        val known = com.spacez.unlock_stage1.extras.PcWake.macs(this).isNotEmpty()
        qsTile?.apply {
            state = if (!paired) Tile.STATE_UNAVAILABLE else Tile.STATE_INACTIVE
            label = "Wake PC"
            if (Build.VERSION.SDK_INT >= 29) subtitle = when { !paired -> "Not paired"; !known -> "Open Aura near PC"; else -> com.spacez.unlock_stage1.extras.PcWake.pcName(this@WakePcTile) }
            updateTile()
        }
    }
    override fun onClick() = runAction("wake", 23)
}
