// Build-check only. These sources are meant to be COPIED into the phone app
// (unlock-stage1/phone/android/app); see ../README.md "Phone side".
plugins {
    id("com.android.library")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "com.spacez.unlock_stage1.watchlink"
    compileSdk = 36
    defaultConfig { minSdk = 26 } // same as the phone app
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

kotlin { compilerOptions { jvmTarget = org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17 } }

dependencies {
    implementation(project(":shared"))
    implementation("com.google.android.gms:play-services-wearable:20.0.1")
}
