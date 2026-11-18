package com.spacez.unlock_stage1.update

import com.spacez.unlock_stage1.BuildConfig

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.provider.MediaStore
import android.provider.Settings
import com.spacez.aura.link.FileOffer
import com.spacez.unlock_stage1.MainActivity
import com.spacez.unlock_stage1.UnlockState
import java.io.File
import kotlin.concurrent.thread

/** Pure rules of the in-app updater (JVM-tested). */
object UpdateLogic {
    data class Name(val version: String, val build: Long)

    private val NAME = Regex("^Aura-(\\d+(?:\\.\\d+)*)\\+(\\d+)\\.apk$")

    /** "Aura-1.0.0+20.apk" -> (1.0.0, 20); anything else is not an Aura update. */
    fun parseName(name: String): Name? {
        val m = NAME.matchEntire(name) ?: return null
        return Name(m.groupValues[1], m.groupValues[2].toLongOrNull() ?: return null)
    }

    /** null = fine to offer: a strictly higher build whose package and signing certificates are ours. */
    fun verdict(pkg: String, runningPkg: String, build: Long, runningBuild: Long, signers: List<ByteArray>, runningSigners: List<ByteArray>): String? = when {
        pkg != runningPkg -> "wrong package $pkg"
        build <= runningBuild -> "build $build is not newer than $runningBuild"
        !sameSigners(signers, runningSigners) -> "signing certificate differs"
        else -> null
    }

    fun sameSigners(a: List<ByteArray>, b: List<ByteArray>): Boolean {
        val x = a.map { it.toList() }.toSet()
        val y = b.map { it.toList() }.toSet()
        return x.isNotEmpty() && x == y
    }
}

/**
 * In-app updater, phone side. A newer Aura APK (sent by the paired PC over Aura Link, or already
 * sitting in Download/Aura) is verified, kept in the app's private storage and offered with a
 * notification + a row on Home / Settings; one tap opens the system package installer.
 */
object AppUpdate {
    private const val CH = "aura_update"
    private const val N_ID = 4210
    const val EXTRA = "aura_update"

    private fun prefs(c: Context) = c.getSharedPreferences("aura_update", 0)
    private fun dir(c: Context) = File(c.filesDir, "update").apply { mkdirs() }

    fun isUpdateName(name: String) = !BuildConfig.PLAY && UpdateLogic.parseName(name) != null

    private fun running(c: Context): Long = runCatching { c.packageManager.getPackageInfo(c.packageName, 0).longVersionCode }.getOrDefault(0L)

    private fun signersOf(info: android.content.pm.PackageInfo?): List<ByteArray> {
        val s = info?.signingInfo ?: return emptyList()
        val arr = if (s.hasMultipleSigners()) s.apkContentsSigners else s.signingCertificateHistory
        return arr?.map { it.toByteArray() } ?: emptyList()
    }

    /** (build, null) = fine to offer, else (build, why not). */
    private fun check(c: Context, apk: File): Pair<Long, String?> {
        val pm = c.packageManager
        val info = pm.getPackageArchiveInfo(apk.path, PackageManager.GET_SIGNING_CERTIFICATES) ?: return 0L to "not a readable APK"
        val mine = runCatching { pm.getPackageInfo(c.packageName, PackageManager.GET_SIGNING_CERTIFICATES) }.getOrNull()
        val build = info.longVersionCode
        return build to UpdateLogic.verdict(info.packageName ?: "", c.packageName, build, running(c), signersOf(info), signersOf(mine))
    }

    /** A file from the paired PC (Aura Link only delivers authenticated peers). Never lands in Downloads. */
    fun onReceived(c: Context, file: File, offer: FileOffer) {
        val n = UpdateLogic.parseName(offer.name)
        if (n == null) { file.delete(); return }
        val dst = File(dir(c), offer.name)
        val moved = runCatching { file.copyTo(dst, true); file.delete(); true }.getOrDefault(false)
        if (!moved) { UnlockState.log("app update ${offer.name}: could not store it"); return }
        accept(c, dst, n, "from your PC")
    }

    private fun accept(c: Context, apk: File, n: UpdateLogic.Name, source: String) {
        val (build, why) = check(c, apk)
        if (why != null) {
            UnlockState.log("app update ${apk.name} refused: $why")
            apk.delete()
            markRejected(c, apk.name)
            return
        }
        // keep only the newest offer
        dir(c).listFiles()?.filter { it.name != apk.name }?.forEach { it.delete() }
        prefs(c).edit().putString("file", apk.name).putLong("build", build).putString("version", n.version).apply()
        UnlockState.log("app update ready: ${n.version} build $build ($source)")
        notifyReady(c, build)
    }

    private fun markRejected(c: Context, name: String) {
        val s = prefs(c).getStringSet("rejected", emptySet())!!.toMutableSet().apply { add(name) }
        prefs(c).edit().putStringSet("rejected", s).apply()
    }

    /** The verified update waiting to be installed, if it is still newer than what runs. */
    fun pending(c: Context): Triple<File, Long, String>? {
        val p = prefs(c)
        val name = p.getString("file", null) ?: return null
        val f = File(dir(c), name)
        val build = p.getLong("build", 0)
        if (!f.isFile || build <= running(c)) {
            // installed (or gone): tidy up
            runCatching { dir(c).listFiles()?.forEach { it.delete() } }
            p.edit().remove("file").remove("build").remove("version").apply()
            runCatching { c.getSystemService(NotificationManager::class.java).cancel(N_ID) }
            return null
        }
        return Triple(f, build, p.getString("version", "") ?: "")
    }

    fun info(c: Context): Map<String, Any> = if (BuildConfig.PLAY) emptyMap() else pending(c)?.let { mapOf("updateBuild" to it.second.toInt(), "updateVersion" to it.third) }
        ?: mapOf("updateBuild" to 0, "updateVersion" to "")

    /** An Aura APK that arrived in Download/Aura before this updater existed. Background thread. */
    fun scanDownloads(c: Context) {
        if (BuildConfig.PLAY) return
        thread(isDaemon = true) {
            runCatching {
                val floor = maxOf(pending(c)?.second ?: 0L, running(c))
                val skip = prefs(c).getStringSet("rejected", emptySet())!!
                var bestUri: Uri? = null
                var bestName: UpdateLogic.Name? = null
                var bestFile = ""
                fun consider(uri: Uri, name: String) {
                    val n = UpdateLogic.parseName(name) ?: return
                    if (name in skip || n.build <= floor || (bestName != null && bestName!!.build >= n.build)) return
                    bestUri = uri; bestName = n; bestFile = name
                }
                if (Build.VERSION.SDK_INT >= 29) {
                    c.contentResolver.query(MediaStore.Downloads.EXTERNAL_CONTENT_URI,
                        arrayOf(MediaStore.MediaColumns._ID, MediaStore.MediaColumns.DISPLAY_NAME),
                        MediaStore.MediaColumns.RELATIVE_PATH + " LIKE ? AND " + MediaStore.MediaColumns.DISPLAY_NAME + " LIKE ?",
                        arrayOf("Download/Aura/%", "Aura-%.apk"), null)?.use { q ->
                        while (q.moveToNext()) consider(android.content.ContentUris.withAppendedId(MediaStore.Downloads.EXTERNAL_CONTENT_URI, q.getLong(0)), q.getString(1))
                    }
                } else {
                    File(Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOWNLOADS), "Aura").listFiles()?.forEach { consider(Uri.fromFile(it), it.name) }
                }
                val uri = bestUri ?: return@runCatching
                val n = bestName ?: return@runCatching
                val dst = File(dir(c), bestFile)
                c.contentResolver.openInputStream(uri)?.use { i -> dst.outputStream().use { o -> i.copyTo(o, 256 * 1024) } } ?: return@runCatching
                accept(c, dst, n, "found in Download/Aura")
            }.onFailure { UnlockState.log("app update scan failed: ${it.message}") }
        }
    }

    private fun notifyReady(c: Context, build: Long) {
        val nm = c.getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(NotificationChannel(CH, "App updates", NotificationManager.IMPORTANCE_DEFAULT).apply { setShowBadge(false) })
        val i = Intent(c, MainActivity::class.java).putExtra(EXTRA, true).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP)
        val pi = PendingIntent.getActivity(c, N_ID, i, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        val n = Notification.Builder(c, CH).setSmallIcon(com.spacez.unlock_stage1.R.drawable.ic_aura_glyph)
            .setContentTitle("Aura update ready - tap to update").setContentText("Build $build from your PC")
            .setContentIntent(pi).setAutoCancel(true).build()
        runCatching { nm.notify(N_ID, n) }
    }

    /** Tap to update. Returns "ok" (installer opened), "settings" (asked to allow installs first) or a reason. */
    fun install(c: Context): String {
        if (BuildConfig.PLAY) return "Updates come from Google Play"
        val p = pending(c) ?: return "No update waiting"
        if (!c.packageManager.canRequestPackageInstalls()) {
            prefs(c).edit().putBoolean("after_settings", true).apply()
            val s = Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:" + c.packageName)).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            return if (runCatching { c.startActivity(s) }.isSuccess) "settings" else "Allow Aura to install apps in Settings, then tap Update again"
        }
        prefs(c).edit().putBoolean("after_settings", false).apply()
        val uri = Uri.parse("content://" + c.packageName + ".aura.clip/update/" + p.first.name)
        val v = Intent(Intent.ACTION_VIEW).setDataAndType(uri, "application/vnd.android.package-archive")
            .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
        return if (runCatching { c.startActivity(v) }.isSuccess) "ok" else "Could not open the installer"
    }

    /** Back from the "install unknown apps" page: if allowed now, carry on to the installer. */
    fun continueAfterSettings(c: Context) {
        if (BuildConfig.PLAY) return
        if (prefs(c).getBoolean("after_settings", false) && c.packageManager.canRequestPackageInstalls()) install(c)
    }
}
