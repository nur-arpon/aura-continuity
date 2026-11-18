plugins { id("org.jetbrains.kotlin.jvm") }

java { toolchain { languageVersion.set(JavaLanguageVersion.of(17)) } }
kotlin { jvmToolchain(17) }

// Aura Link (item 11): the shared Kotlin core lives in ../../aura-link/kotlin and is compiled into
// this pure-JVM module, so the watch app (and phone-patch) get it and its tests run on the JVM.
sourceSets {
    main { kotlin.srcDir("../../aura-link/kotlin/src/main/kotlin") }
    test { kotlin.srcDir("../../aura-link/kotlin/src/test/kotlin") }
}

tasks.withType<Test> {
    systemProperty("aura.vectors", file("../../aura-link/test-vectors/aura-link-v1.txt").absolutePath)
}

dependencies {
    testImplementation("junit:junit:4.13.2")
}
