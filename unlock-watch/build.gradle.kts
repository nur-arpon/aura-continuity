// Same AGP / Kotlin as the phone app (unlock-stage1/phone/android) so the patch drops in.
plugins {
    id("com.android.application") version "9.1.0" apply false
    id("com.android.library") version "9.1.0" apply false
    id("org.jetbrains.kotlin.android") version "2.4.0" apply false
    id("org.jetbrains.kotlin.jvm") version "2.4.0" apply false
    id("org.jetbrains.kotlin.plugin.compose") version "2.4.0" apply false
}

// Windows: a stray quote in the user's PATH (e.g. `"C:\Program Files\PowerShell\7"`) leaks into the test JVM's
// -Djava.library.path and breaks the forked command line ("Could not find or load main class VS").
// AGP appends its jniLibs dirs to the daemon's own java.library.path, so clean that property itself.
System.setProperty("java.library.path", (System.getProperty("java.library.path") ?: "").replace("\"", ""))
