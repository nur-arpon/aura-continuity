package com.spacez.unlock_stage1.buds

import com.spacez.unlock_stage1.BuildConfig

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.ServiceConnection
import android.content.pm.PackageManager
import android.media.AudioManager
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.os.Parcel
import android.util.Log
import com.spacez.aura.link.Message
import com.spacez.unlock_stage1.AuraPhone
import com.spacez.unlock_stage1.UnlockState
import rikka.shizuku.Shizuku
import java.util.concurrent.atomic.AtomicBoolean

/**
 * Galaxy Buds Core hand-off, phone side. The Buds keep ONE audio link; the phone must let go before the
 * PC can take them, and connect them again when the PC gives them back.
 *
 * The DECISION (the device that most recently STARTED playing gets the Buds, calls win, short sounds are
 * ignored) is made on the PC, see aura-pc buds.rs. This side reports and obeys.
 *
 * PC -> phone (Aura TEXT messages, prefix "aura:buds "):
 *   "take"      the PC wants the Buds: disconnect the Buds' A2DP + HFP here (those two only)
 *   "pc 0|1"    the PC is playing (1) / idle (0)
 *   "give"      the PC let go: connect them here now (also clears the "Buds on your laptop" notification)
 *   "?"         the PC wants the current media / call state now
 *   "moved pc"  the PC took the Buds on its own: show "Buds on your laptop - Bring here"
 * Phone -> PC: "phone <playing 0|1> <call 0|1>" whenever either changes (and on "?"), "want" = Bring here.
 * The privileged call runs as the adb shell user through Shizuku ([BudsShell]); the owner grants the app
 * Shizuku permission once (Shizuku must be running: start it from wireless debugging).
 */
object BudsHandoff {
    private const val TAG = "BudsHandoff"
    private val main = Handler(Looper.getMainLooper())
    @Volatile private var svc: IBinder? = null
    private val binding = AtomicBoolean(false)
    @Volatile private var pcPlaying = false
    @Volatile private var released = false   // we let go of the Buds for the PC and have not taken them back

    fun shizukuReady(): Boolean = !BuildConfig.PLAY && runCatching { Shizuku.pingBinder() && Shizuku.checkSelfPermission() == PackageManager.PERMISSION_GRANTED }.getOrDefault(false)

    /** Ask Shizuku for permission (shows its own dialog). Call from a setup screen / first use. */
    fun requestPermission() {
        if (BuildConfig.PLAY) return
        // Shizuku hands over its binder a moment after the app starts: asking before that did nothing, so ask when it arrives
        runCatching {
            Shizuku.addBinderReceivedListenerSticky(object : Shizuku.OnBinderReceivedListener {
                override fun onBinderReceived() {
                    Shizuku.removeBinderReceivedListener(this)
                    runCatching { if (!shizukuReady()) { Log.i(TAG, "asking Shizuku permission"); Shizuku.requestPermission(4412) } }
                }
            })
        }.onFailure { Log.w(TAG, "Shizuku: ${it.message}") }
    }

    /** Entry point from AuraPhone.onMessage; [arg] is the text after "aura:buds ". */
    fun onPcMessage(ctx: Context, arg: String) {
        start(ctx)
        when {
            arg == "take" -> { released = true; pcPlaying = true; act(ctx, "disconnect") }
            arg == "give" -> { released = false; cancelNote(ctx); act(ctx, "connect") }
            arg == "?" -> report(ctx, true)
            arg == "moved pc" -> note(ctx)
            arg.startsWith("pc ") -> pcPlaying = arg.endsWith("1")
        }
    }

    // ---- what the phone tells the PC: media playing / call, so the PC can follow the most recent start ----

    @Volatile private var lastSent: String? = null
    private var started = false

    /** Idempotent: listen to the audio mode (ringing / call) so a call reaches the PC at once. */
    @Synchronized
    fun start(ctx: Context) {
        if (started) return
        started = true
        val app = ctx.applicationContext
        appCtx = app
        if (android.os.Build.VERSION.SDK_INT >= 31) runCatching {
            val am = app.getSystemService(Context.AUDIO_SERVICE) as AudioManager
            am.addOnModeChangedListener(app.mainExecutor) { _ -> report(app, false) }
        }.onFailure { Log.w(TAG, "mode listener: ${it.message}") }
        runCatching {
            val am = app.getSystemService(Context.AUDIO_SERVICE) as AudioManager
            // fires whenever any app starts / stops an audio player; isMusicActive is read a moment later, once it has settled
            am.registerAudioPlaybackCallback(object : AudioManager.AudioPlaybackCallback() {
                override fun onPlaybackConfigChanged(configs: MutableList<android.media.AudioPlaybackConfiguration>?) {
                    main.removeCallbacks(recheck); main.postDelayed(recheck, 400)
                }
            }, main)
        }.onFailure { Log.w(TAG, "playback callback: ${it.message}") }
        report(app, true)
    }

    private var appCtx: Context? = null
    private val recheck = Runnable { appCtx?.let { report(it, false) } }

    private fun inCall(am: AudioManager) = am.mode == AudioManager.MODE_RINGTONE || am.mode == AudioManager.MODE_IN_CALL || am.mode == AudioManager.MODE_IN_COMMUNICATION

    /** Called by the media listener on any playback change, by the mode listener and on "?". Sends only on change unless [force]. */
    fun report(ctx: Context, force: Boolean) {
        val am = ctx.getSystemService(Context.AUDIO_SERVICE) as AudioManager
        // real audio only: a muted autoplay video (Facebook feed) keeps a media session in PLAYING but makes no sound
        val playing = am.isMusicActive
        val text = "aura:buds phone ${if (playing) 1 else 0} ${if (inCall(am)) 1 else 0}"
        if (!force && text == lastSent) return
        lastSent = text
        val l = AuraPhone.link ?: return
        Thread {
            runCatching { l.send(Message.text(text)) }.onFailure { Log.w(TAG, "send: ${it.message}") }
            Log.i(TAG, "told the PC: $text")
        }.start()
    }

    // ---- Samsung-style quick override: "Buds on your laptop - Bring here" ----

    private const val CH = "aura_buds"
    private const val NOTE_ID = 4413
    const val ACTION_WANT = "com.spacez.unlock_stage1.BUDS_WANT"

    private fun note(ctx: Context) {
        val nm = ctx.getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(NotificationChannel(CH, "Galaxy Buds hand-off", NotificationManager.IMPORTANCE_LOW).apply { setShowBadge(false) })
        val pi = PendingIntent.getBroadcast(ctx, 0, Intent(ctx, BudsActionReceiver::class.java).setAction(ACTION_WANT), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        val n = Notification.Builder(ctx, CH).setSmallIcon(com.spacez.unlock_stage1.R.drawable.ic_aura_glyph)
            .setContentTitle("Buds on your laptop").setContentText("They followed what you started there.")
            .setTimeoutAfter(10 * 60_000L).setAutoCancel(true)
            .addAction(Notification.Action.Builder(null, "Bring here", pi).build()).build()
        runCatching { nm.notify(NOTE_ID, n) }
    }

    private fun cancelNote(ctx: Context) {
        runCatching { ctx.getSystemService(NotificationManager::class.java).cancel(NOTE_ID) }
    }

    /** The notification button: ask the PC to let go and hand the Buds back. */
    fun want(ctx: Context) {
        cancelNote(ctx)
        val l = AuraPhone.link ?: run { UnlockState.log("buds: Bring here pressed but the PC link is down"); return }
        Thread {
            runCatching { l.send(Message.text("aura:buds want")) }
            UnlockState.log("buds: asked the PC to give the Buds back")
        }.start()
    }

    private val conn = object : ServiceConnection {
        override fun onServiceConnected(n: ComponentName?, b: IBinder?) { svc = b }
        override fun onServiceDisconnected(n: ComponentName?) { svc = null }
    }

    private fun bind(ctx: Context) {
        if (svc?.pingBinder() == true || !binding.compareAndSet(false, true)) return
        try {
            val args = Shizuku.UserServiceArgs(ComponentName(ctx.packageName, BudsShell.Service::class.java.name))
                .daemon(false).processNameSuffix("buds").version(1)
            Shizuku.bindUserService(args, conn)
        } finally { main.postDelayed({ binding.set(false) }, 3000) }
    }

    /** Is a Bluetooth headset/earbud audio output connected to this phone right now (no privileges needed)? */
    private fun budsAudioHere(ctx: Context): Boolean = runCatching {
        (ctx.getSystemService(Context.AUDIO_SERVICE) as AudioManager).getDevices(AudioManager.GET_DEVICES_OUTPUTS)
            .any { it.type == android.media.AudioDeviceInfo.TYPE_BLUETOOTH_A2DP }
    }.getOrDefault(true)

    /** Run one action as shell; the result goes to the app log. Off the main thread. */
    private fun act(ctx: Context, action: String) {
        Thread {
            val out = try {
                if (!shizukuReady()) "err Shizuku is not running / not granted"
                else {
                    bind(ctx)
                    var t = 0
                    while (svc?.pingBinder() != true && t++ < 30) Thread.sleep(100)
                    val b = svc
                    if (b == null) "err Shizuku service did not start"
                    else {
                        val d = Parcel.obtain(); val r = Parcel.obtain()
                        try { d.writeString(action); b.transact(1, d, r, 0); r.readException(); r.readString() ?: "err empty" }
                        finally { d.recycle(); r.recycle() }
                    }
                }
            } catch (e: Throwable) { "err ${e.javaClass.simpleName}: ${e.message}" }
            Log.i(TAG, "buds $action: $out")
            runCatching { UnlockState.log("buds $action: $out") }
            // taking needs nothing from the phone (the laptop simply wins the Buds); getting them BACK does: without
            // Shizuku the phone cannot connect them, and the laptop then tries adb. If they are still not here 8 s later,
            // offer one tap that opens Bluetooth settings.
            if (action == "connect" && out.startsWith("err")) main.postDelayed({ if (!budsAudioHere(ctx)) runCatching {
                val nm = ctx.getSystemService(NotificationManager::class.java)
                nm.createNotificationChannel(NotificationChannel(CH, "Galaxy Buds hand-off", NotificationManager.IMPORTANCE_LOW).apply { setShowBadge(false) })
                val pi = PendingIntent.getActivity(ctx, 1, Intent(android.provider.Settings.ACTION_BLUETOOTH_SETTINGS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
                val b = Notification.Builder(ctx, CH).setSmallIcon(com.spacez.unlock_stage1.R.drawable.ic_aura_glyph)
                    .setContentTitle("Connect your Buds")
                    .setContentText("Your laptop let go of them. Tap to connect them here (start Shizuku to make this automatic).")
                    .setContentIntent(pi).setAutoCancel(true).setTimeoutAfter(10 * 60_000L)
                    .addAction(Notification.Action.Builder(null, "Bluetooth", pi).build())
                nm.notify(NOTE_ID + 1, b.build())
            } }, 8_000)
        }.start()
    }
}

/** Notification button "Bring here". */
class BudsActionReceiver : BroadcastReceiver() {
    override fun onReceive(c: Context, i: Intent) { BudsHandoff.want(c) }
}
