import org.gradle.api.DefaultTask
import org.gradle.api.file.DirectoryProperty
import org.gradle.api.file.RegularFileProperty
import org.gradle.api.tasks.InputFile
import org.gradle.api.tasks.OutputDirectory
import org.gradle.api.tasks.PathSensitive
import org.gradle.api.tasks.PathSensitivity
import org.gradle.api.tasks.TaskAction

plugins {
	id("com.android.application")
}

// The host is shared by the scenario app and the size-tier apps: a tier build
// passes its own bundle, application id, and build directory.
//   -ParenaBundle=<path to main.lynx.bundle>   (default: ../../dist/main.lynx.bundle)
//   -ParenaApplicationId=<id>                  (default: dev.leptoskit.arena.lynx)
//   -ParenaBuildDir=<dir>                      (default: app/build)
val lynxVersion: String by project
val arenaBundle = providers.gradleProperty("arenaBundle")
	.orElse(rootProject.layout.projectDirectory.file("../dist/main.lynx.bundle").asFile.path)
val arenaApplicationId = providers.gradleProperty("arenaApplicationId").orElse("dev.leptoskit.arena.lynx")
providers.gradleProperty("arenaBuildDir").orNull?.let { layout.buildDirectory.set(file(it)) }

/** Copies the Lynx bundle into the APK's assets as `main.lynx.bundle`. */
abstract class LynxBundleAssets : DefaultTask() {
	@get:InputFile
	@get:PathSensitive(PathSensitivity.NONE)
	abstract val bundle: RegularFileProperty

	@get:OutputDirectory
	abstract val outputDirectory: DirectoryProperty

	@TaskAction
	fun copy() {
		val target = outputDirectory.get().asFile
		target.deleteRecursively()
		target.mkdirs()
		bundle.get().asFile.copyTo(target.resolve("main.lynx.bundle"))
	}
}

val lynxBundleAssets = tasks.register<LynxBundleAssets>("lynxBundleAssets") {
	bundle.set(file(arenaBundle.get()))
}

android {
	namespace = "dev.leptoskit.arena.lynx"
	compileSdk = 36

	defaultConfig {
		applicationId = arenaApplicationId.get()
		minSdk = 24
		targetSdk = 36
		versionCode = 1
		versionName = "1.0"
		ndk {
			abiFilters += "arm64-v8a"
		}
		// The engine's own version string (LynxEnv.getLynxVersion) reads
		// 0.0.1 in the Maven release, so reports carry the linked version.
		buildConfigField("String", "LYNX_VERSION", "\"$lynxVersion\"")
	}

	buildFeatures {
		buildConfig = true
	}

	buildTypes {
		release {
			isMinifyEnabled = true
			isShrinkResources = true
			proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
			// Release, signed with the debug key so it installs anywhere.
			signingConfig = signingConfigs.getByName("debug")
		}
	}

	compileOptions {
		sourceCompatibility = JavaVersion.VERSION_17
		targetCompatibility = JavaVersion.VERSION_17
	}

	packaging {
		jniLibs {
			// Keep native libraries uncompressed and page-aligned, as the Play
			// Store ships them, so the size benchmark sees installed bytes.
			useLegacyPackaging = false
		}
	}
}

androidComponents {
	onVariants { variant ->
		variant.sources.assets?.addGeneratedSourceDirectory(lynxBundleAssets, LynxBundleAssets::outputDirectory)
	}
}

dependencies {
	implementation("org.lynxsdk.lynx:lynx:$lynxVersion")
	// `<input>` (the size-tier form screens) is an XElement. The `xelement`
	// artifact holds the generated registries and depends on every XElement
	// module; only `xelement-input` is needed (see InputBehavior.kt).
	implementation("org.lynxsdk.lynx:xelement:$lynxVersion") { isTransitive = false }
	implementation("org.lynxsdk.lynx:xelement-input:$lynxVersion")
	constraints {
		// xelement-input pulls appcompat 1.0.0, whose vectordrawable-animated
		// 1.0.0 shares a namespace with vectordrawable, which AGP 9 rejects.
		implementation("androidx.vectordrawable:vectordrawable-animated:1.2.0")
	}
}
