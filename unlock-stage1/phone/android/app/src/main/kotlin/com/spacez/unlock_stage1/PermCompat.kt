package com.spacez.unlock_stage1

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build

/**
 * Runtime permissions that only EXIST on newer Android. Asking the system about a permission it does not
 * know (BLUETOOTH_CONNECT before Android 12, POST_NOTIFICATIONS before 13) always answers "denied", which
 * made every Bluetooth call fail with "no Bluetooth permission" on Android 11 and the setup checklist
 * impossible to finish. Before those versions the need is simply met (Bluetooth is an install-time permission,
 * notifications need no ask).
 */
object PermCompat {
    private val btNew = setOf(Manifest.permission.BLUETOOTH_CONNECT, Manifest.permission.BLUETOOTH_ADVERTISE, Manifest.permission.BLUETOOTH_SCAN)

    /** Does this Android version have [p] at all? */
    fun exists(p: String): Boolean = when (p) {
        in btNew -> Build.VERSION.SDK_INT >= 31
        Manifest.permission.POST_NOTIFICATIONS -> Build.VERSION.SDK_INT >= 33
        else -> true
    }

    fun has(c: Context, p: String): Boolean = !exists(p) || c.checkSelfPermission(p) == PackageManager.PERMISSION_GRANTED

    /** Only the permissions this Android version can actually ask for. */
    fun askable(ps: Array<String>): Array<String> = ps.filter { exists(it) }.toTypedArray()

    fun btConnect(c: Context) = has(c, Manifest.permission.BLUETOOTH_CONNECT)
}
