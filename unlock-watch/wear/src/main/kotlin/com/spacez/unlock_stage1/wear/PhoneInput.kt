package com.spacez.unlock_stage1.wear

import android.content.Context
import com.google.android.gms.wearable.Wearable
import com.spacez.unlock_stage1.watchlink.WatchProtocol
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.tasks.await

/** Where the touchpad / Air mouse send their INPUT events: the laptop (Aura Link) or the phone (Data Layer). */
object InputTarget {
    private fun p(c: Context) = c.getSharedPreferences("cu_air", Context.MODE_PRIVATE)
    fun phone(c: Context) = p(c).getBoolean("target_phone", false)
    fun set(c: Context, phone: Boolean) { p(c).edit().putBoolean("target_phone", phone).apply(); if (phone) PhoneInput.warm(c) }
    fun label(phone: Boolean) = if (phone) "Phone" else "Laptop"
}

/**
 * Watch -> phone mouse / keyboard (FUTURE-PLANS item 7). Same JSON bodies as the PC's INPUT message
 * ({"t":"move","dx","dy"} / click / scroll / key / text) sent as Wearable Data Layer messages on
 * [WatchProtocol.PATH_PHONE_INPUT]; one ordered sender so a click never overtakes the moves before it.
 * The phone answers on PATH_PHONE_INPUT_STATE ("ok" / "a11y_off") so the screen can say what to do.
 */
object PhoneInput {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val queue = Channel<String>(capacity = 64, onBufferOverflow = kotlinx.coroutines.channels.BufferOverflow.DROP_OLDEST)
    @Volatile private var node: String? = null
    @Volatile private var started = false
    private val _state = MutableStateFlow("")
    /** "" = nothing heard yet, "ok", "a11y_off" (phone: service not enabled), "no_phone" (not reachable). */
    val state: StateFlow<String> get() = _state
    fun onPhoneState(s: String) { _state.value = s.take(20) }

    @Synchronized private fun ensure(ctx: Context) {
        if (started) return
        started = true
        val app = ctx.applicationContext
        scope.launch {
            for (json in queue) {
                var id = node
                if (id == null) { id = runCatching { PhoneLink.phoneNodes(app).firstOrNull() }.getOrNull(); node = id }
                if (id == null) { _state.value = "no_phone"; continue }
                runCatching { Wearable.getMessageClient(app).sendMessage(id, WatchProtocol.PATH_PHONE_INPUT, json.toByteArray()).await() }
                    .onFailure { node = null }
            }
        }
    }

    /** Resolve the phone node and ping it, so the first tap is not the one that pays for the lookup and the state is known. */
    fun warm(ctx: Context) { ensure(ctx); queue.trySend("""{"t":"hello"}""") }

    fun send(ctx: Context, json: String) { ensure(ctx); queue.trySend(json) }
}
