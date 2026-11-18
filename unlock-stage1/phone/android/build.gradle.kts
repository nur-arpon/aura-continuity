allprojects {
    repositories {
        google()
        mavenCentral()
    }
}

val newBuildDir: Directory =
    rootProject.layout.buildDirectory
        .dir("../../build")
        .get()
rootProject.layout.buildDirectory.value(newBuildDir)

subprojects {
    // Plugins in the pub cache can live on another drive (C:) than this project (D:); Gradle cannot
    // relate paths across drives, so only redirect the ones on this project's drive (item 25).
    if (project.projectDir.toPath().root == rootProject.projectDir.toPath().root) {
        val newSubprojectBuildDir: Directory = newBuildDir.dir(project.name)
        project.layout.buildDirectory.value(newSubprojectBuildDir)
    }
}
subprojects {
    project.evaluationDependsOn(":app")
}

tasks.register<Delete>("clean") {
    delete(rootProject.layout.buildDirectory)
}

// Windows: a stray quote in the user's PATH (e.g. `"C:\Program Files\PowerShell\7"`) leaks into the test JVM's
// -Djava.library.path and breaks the forked command line ("Could not find or load main class VS").
// AGP appends its jniLibs dirs to the daemon's own java.library.path, so clean that property itself.
System.setProperty("java.library.path", (System.getProperty("java.library.path") ?: "").replace("\"", ""))
