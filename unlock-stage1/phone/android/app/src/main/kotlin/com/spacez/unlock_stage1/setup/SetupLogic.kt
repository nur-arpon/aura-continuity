package com.spacez.unlock_stage1.setup

/** Pure logic of the first-run "Allow everything" walk and the watch step (no Android types, JVM-tested). */
object SetupLogic {
    /** The order permissions are asked in: cheap system dialogs first, then the Settings pages. */
    val ORDER = listOf("notifications", "bluetooth", "location", "usage", "listener", "accessibility", "battery", "overlay", "bubbles")

    /** The next step that still needs the user, or null when everything is granted. [skipped] steps
     *  the user already declined this run are not asked again, so the walk always ends. */
    fun next(granted: Map<String, Boolean>, skipped: Set<String> = emptySet(), available: Collection<String> = ORDER): String? =
        ORDER.firstOrNull { it in available && granted[it] != true && it !in skipped }

    fun missing(granted: Map<String, Boolean>, available: Collection<String> = ORDER): List<String> =
        ORDER.filter { it in available && granted[it] != true }

    /** "none" no Wear watch on this phone | "app-missing" watch there, Aura not installed on it
     *  | "unpaired" Aura on the watch but not pinned by the PC | "away" paired, not reachable now
     *  | "connected". */
    fun watchState(connectedNodes: Int, nodesWithApp: Int, pinned: Boolean, pinnedReachable: Boolean): String = when {
        connectedNodes == 0 && nodesWithApp == 0 && !pinned -> "none"
        pinnedReachable -> "connected"
        pinned -> "away"
        nodesWithApp == 0 -> "app-missing"
        else -> "unpaired"
    }
}
