plugins {
    id("org.jetbrains.kotlin.jvm") version "2.4.20"
    id("org.jetbrains.intellij.platform") version "2.19.0"
}

group = "dev.cranpose"
version = providers.gradleProperty("pluginVersion").get()

kotlin {
    jvmToolchain(providers.gradleProperty("jvmToolchain").get().toInt())
}

repositories {
    mavenCentral()
    intellijPlatform {
        defaultRepositories()
    }
}

dependencies {
    intellijPlatform {
        val localIde = providers.gradleProperty("platformLocalPath").orNull
        if (localIde != null) local(localIde) else intellijIdea(providers.gradleProperty("platformVersion"))
    }
    testImplementation("junit:junit:4.13.2")
}

intellijPlatform {
    buildSearchableOptions = false
    pluginConfiguration {
        ideaVersion {
            sinceBuild = providers.gradleProperty("pluginSinceBuild")
            untilBuild = provider { null }
        }
    }
    signing {
        certificateChain = providers.environmentVariable("CERTIFICATE_CHAIN")
        privateKey = providers.environmentVariable("PRIVATE_KEY")
        password = providers.environmentVariable("PRIVATE_KEY_PASSWORD")
    }
    publishing {
        token = providers.environmentVariable("PUBLISH_TOKEN")
    }
}

val uiManifest = layout.projectDirectory.file("../ui/Cargo.toml")
val cargoTargetDir: Directory = layout.projectDirectory.dir(providers.gradleProperty("cranposeCargoTargetDir").get())
val nativeResources = layout.buildDirectory.dir("native-resources")

val osName: String = System.getProperty("os.name")
val hostOs = when {
    osName.startsWith("Mac") -> "macos"
    osName.startsWith("Windows") -> "windows"
    else -> "linux"
}
val hostArch = when (System.getProperty("os.arch")) {
    "aarch64", "arm64" -> "aarch64"
    else -> "x86_64"
}
val executable = if (hostOs == "windows") "cranpose-intellij-ui.exe" else "cranpose-intellij-ui"
val hostBinary = cargoTargetDir.file("release/$executable")

val cargoBuild = tasks.register<Exec>("cargoBuild") {
    description = "Builds the Cranpose UI for this machine, without the desktop window code."
    group = "build"
    workingDir = layout.projectDirectory.asFile
    commandLine(
        "cargo", "build", "--release", "--no-default-features",
        "--manifest-path", uiManifest.asFile.path,
        "--target-dir", cargoTargetDir.asFile.path,
    )
    outputs.upToDateWhen { false }
}

val bundleNative = tasks.register<Sync>("bundleNative") {
    description = "Puts the UI binaries into the plugin's resources under native/<os>-<arch>/."
    val prebuilt = providers.gradleProperty("cranposeNativeDir").orNull
    if (prebuilt != null) {
        from(prebuilt) { into("native") }
    } else {
        dependsOn(cargoBuild)
        from(hostBinary) { into("native/$hostOs-$hostArch") }
    }
    into(nativeResources)
}

sourceSets.main {
    resources.srcDir(nativeResources)
}

tasks.processResources {
    dependsOn(bundleNative)
}

tasks.test {
    dependsOn(cargoBuild)
    systemProperty("java.awt.headless", "true")
    systemProperty("cranpose.ui.binary", hostBinary.asFile.path)
    systemProperty("cranpose.test.output", layout.buildDirectory.dir("test-frames").get().asFile.path)
}

tasks.runIde {
    providers.gradleProperty("cranposeUiBinary").orNull?.let { environment("CRANPOSE_UI_BINARY", it) }
    providers.gradleProperty("runIdeProject").orNull?.let { args(it) }
    jvmArgs("-Ddisable.android.first.run=true")
}
