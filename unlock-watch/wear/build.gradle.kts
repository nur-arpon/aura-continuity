plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
}

android {
    namespace = "com.spacez.unlock_stage1.wear"
    compileSdk = 36

    defaultConfig {
        // MUST equal the phone app's applicationId: the Wearable Data Layer only delivers
        // messages between apps with the same package name AND the same signing certificate.
        applicationId = "com.spacez.unlock_stage1"
        minSdk = 30
        targetSdk = 36
        versionCode = 16
        versionName = "1.0.0"
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            // Same as the phone app (unlock-stage1/phone/android/app/build.gradle.kts): the
            // release build is signed with this PC's ~/.android/debug.keystore. If the phone app
            // ever gets a real release key, change BOTH apps together.
            signingConfig = signingConfigs.getByName("debug")
        }
    }

    buildFeatures { compose = true }

    // Screenshots without a watch: Robolectric + Roborazzi render the real screens on the JVM
    // (gradlew :wear:testDebugUnitTest --tests *ScreenshotTest*) into ../screenshots/render/.
    testOptions { unitTests { isIncludeAndroidResources = true; all { it.systemProperty("roborazzi.test.record", "true") } } }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

kotlin { compilerOptions { jvmTarget = org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17 } }

dependencies {
    implementation(project(":shared"))
    implementation("com.google.android.gms:play-services-wearable:20.0.1")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-play-services:1.11.0")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-guava:1.11.0")
    implementation("androidx.core:core-ktx:1.16.0")
    implementation("androidx.activity:activity-compose:1.13.0")

    // Compose for Wear OS, Material 3 (rotary + haptics are built into TransformingLazyColumn)
    implementation("androidx.wear.compose:compose-material3:1.6.2")
    implementation("androidx.wear.compose:compose-foundation:1.6.2")
    implementation("androidx.wear.compose:compose-navigation:1.6.2")
    implementation("androidx.wear:wear-input:1.1.0")
    implementation("androidx.wear:wear-tooling-preview:1.0.0")

    // Tile: Material 3 ProtoLayout + Horologist SuspendingTileService (same tiles/protolayout versions)
    implementation("androidx.wear.tiles:tiles:1.6.2")
    implementation("androidx.wear.protolayout:protolayout:1.4.2")
    implementation("androidx.wear.protolayout:protolayout-expression:1.4.2")
    implementation("androidx.wear.protolayout:protolayout-material3:1.4.2")
    implementation("com.google.android.horologist:horologist-tiles:0.8.4-alpha")

    // Complication (SuspendingComplicationDataSourceService)
    implementation("androidx.wear.watchface:watchface-complications-data-source-ktx:1.3.0")


    testImplementation("junit:junit:4.13.2")
    debugImplementation("androidx.wear.tiles:tiles-renderer:1.6.2") // tile screenshots in unit tests only
    testImplementation("org.robolectric:robolectric:4.17")
    testImplementation("io.github.takahirom.roborazzi:roborazzi:1.76.0")
    testImplementation("io.github.takahirom.roborazzi:roborazzi-compose:1.76.0")
    testImplementation("androidx.compose.ui:ui-test-junit4:1.9.2")
    debugImplementation("androidx.compose.ui:ui-test-manifest:1.9.2")
}
