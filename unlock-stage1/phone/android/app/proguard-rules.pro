# Aura release shrinking rules (the Flutter Gradle plugin picks this file up by name).

# Buds hand-off: BudsShell is started by class name, as the adb shell user, through Shizuku's user service
# and through `app_process` - R8 renamed it, so neither could find it.
-keep class com.spacez.unlock_stage1.buds.BudsShell { *; }
-keep class com.spacez.unlock_stage1.buds.** { *; }
