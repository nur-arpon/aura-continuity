package com.spacez.unlock_stage1.handoff

import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri

/** Item 27: the apps on this phone that can open a youtube.com/watch link (YouTube, Morphe / ReVanced, browsers...). */
object YtHandlers {
    private fun watch() = Intent(Intent.ACTION_VIEW, Uri.parse("https://www.youtube.com/watch?v=dQw4w9WgXcQ")).addCategory(Intent.CATEGORY_BROWSABLE)

    fun list(c: Context): List<LinkLogic.Handler> {
        val pm = c.packageManager
        val browsers = runCatching {
            pm.queryIntentActivities(Intent(Intent.ACTION_VIEW, Uri.parse("https://example.com/")).addCategory(Intent.CATEGORY_BROWSABLE), PackageManager.MATCH_ALL)
                .map { it.activityInfo.packageName }.toSet()
        }.getOrDefault(emptySet())
        return runCatching { pm.queryIntentActivities(watch(), PackageManager.MATCH_ALL) }.getOrDefault(emptyList())
            .map { it.activityInfo.packageName }.distinct()
            .map { LinkLogic.Handler(it, runCatching { pm.getApplicationLabel(pm.getApplicationInfo(it, 0)).toString() }.getOrDefault(it), it in browsers) }
    }

    /** The package a YouTube link opens in, or null for a plain VIEW. */
    fun pick(c: Context): String? {
        val def = runCatching { c.packageManager.resolveActivity(watch(), PackageManager.MATCH_DEFAULT_ONLY)?.activityInfo?.packageName }.getOrNull()
        return LinkLogic.pickYouTube(list(c), def?.takeIf { it != "android" }, LinkPrefs.ytChosen(c))
    }

    fun effective(c: Context): String = pick(c)?.let { p -> list(c).firstOrNull { it.pkg == p }?.label } ?: "Browser"
}
