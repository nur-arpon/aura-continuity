import java.util.Properties

plugins {
    id("com.android.application")
    // The Flutter Gradle Plugin must be applied after the Android and Kotlin Gradle plugins.
    id("dev.flutter.flutter-gradle-plugin")
}

android {
    namespace = "com.spacez.unlock_stage1"
    compileSdk = flutter.compileSdkVersion
    ndkVersion = flutter.ndkVersion

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    defaultConfig {
        // Stage-1 test app id (not a store app).
        applicationId = "com.spacez.unlock_stage1"
        // You can update the following values to match your application needs.
        // For more information, see: https://flutter.dev/to/review-gradle-config.
        minSdk = 26
        targetSdk = flutter.targetSdkVersion
        // Uses the version code from pubspec.yaml. When using split APKs, 1000 * ABI_VERSION
        // is added automatically by Flutter. (https://developer.android.com/studio/build/configure-apk-splits#configure-APK-versions)
        // You can force using the value of versionCode by specifying the `-P force-version-code-ignoring-abi=true`
        // flag during build.
        versionCode = flutter.versionCode
        versionName = flutter.versionName
    }

    // "pro" = the sideload build (his own devices, unchanged id + debug key); "play" = the Play Store build
    flavorDimensions += "dist"
    productFlavors {
        create("pro") {
            dimension = "dist"
            buildConfigField("boolean", "PLAY", "false")
        }
        create("play") {
            dimension = "dist"
            applicationId = "com.spacez.aura"
            buildConfigField("boolean", "PLAY", "true")
        }
    }
    buildFeatures { buildConfig = true }

    // Play upload key (publish/keys, git-ignored). Only the play flavor uses it; pro keeps the debug key.
    val keyProps = Properties().apply {
        val kf = rootProject.file("../../../publish/keys/key.properties")
        if (kf.exists()) kf.inputStream().use { load(it) }
    }
    signingConfigs {
        if (keyProps.isNotEmpty()) create("upload") {
            storeFile = file(keyProps["storeFile"] as String)
            storePassword = keyProps["storePassword"] as String
            keyAlias = keyProps["keyAlias"] as String
            keyPassword = keyProps["keyPassword"] as String
        }
    }

    // item 13: Robolectric renders the real home-screen widget (RemoteViews) to PNG on the JVM
    testOptions { unitTests { isIncludeAndroidResources = true } }

    buildTypes {
        release {
            // signing comes from the flavor (a build type setting would override it): pro = debug key, play = upload key
        }
    }
    productFlavors.getByName("pro").signingConfig = signingConfigs.getByName("debug")
    productFlavors.getByName("play").signingConfig = signingConfigs.findByName("upload") ?: signingConfigs.getByName("debug")
}

// Aura Link (item 11): the shared Kotlin core + its tests and the cross-language vectors.
android.sourceSets["main"].java.srcDir("../../../../aura-link/kotlin/src/main/kotlin")
android.sourceSets["test"].java.srcDir("../../../../aura-link/kotlin/src/test/kotlin")
tasks.withType<Test> {
    systemProperty("aura.vectors", file("../../../../aura-link/test-vectors/aura-link-v1.txt").absolutePath)
}

kotlin {
    compilerOptions {
        jvmTarget = org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17
    }
}

flutter {
    source = "../.."
}

dependencies {
    // Wear OS Data Layer (watch <-> phone). Reused Google library, same version as unlock-watch.
    implementation("com.google.android.gms:play-services-wearable:20.0.1")
    // Galaxy Buds hand-off (buds/BudsHandoff.kt): run the Bluetooth profile calls as the adb shell user
    implementation("dev.rikka.shizuku:api:13.1.5")
    implementation("dev.rikka.shizuku:provider:13.1.5")
    testImplementation("junit:junit:4.13.2")
    // same Robolectric as unlock-watch (already in the Gradle cache), only for the widget screenshots
    testImplementation("org.robolectric:robolectric:4.17")
}

// Robolectric resources (item 13): the host-test package reads the merged assets, which the
// Flutter plugin's copy task also writes; declare the order so Gradle's validation is happy.
tasks.matching { it.name == "packageDebugUnitTestForUnitTest" }.configureEach {
    dependsOn(tasks.matching { it.name == "copyFlutterAssetsDebug" })
}
tasks.matching { it.name == "packageReleaseUnitTestForUnitTest" }.configureEach {
    dependsOn(tasks.matching { it.name == "copyFlutterAssetsRelease" })
}
// Same ordering for the flavored variants (Pro / Play): every host-test task that reads the merged assets runs after the Flutter copy.
listOf("Pro", "Play").forEach { f ->
    listOf("Debug", "Release").forEach { b ->
        tasks.matching { it.name == "package${f}${b}UnitTestForUnitTest" || it.name == "generate${f}${b}UnitTestConfig" }.configureEach {
            dependsOn(tasks.matching { it.name == "copyFlutterAssets${f}${b}" })
        }
    }
}
