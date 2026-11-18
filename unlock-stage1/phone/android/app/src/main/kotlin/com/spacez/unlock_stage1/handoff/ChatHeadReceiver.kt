package com.spacez.unlock_stage1.handoff

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent

/** The "Show bubble" action of the ongoing "Aura chat head" notification. */
class ChatHeadReceiver : BroadcastReceiver() {
    override fun onReceive(c: Context, i: Intent) { ChatHead.summon(c) }
}
