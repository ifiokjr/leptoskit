import java.util.Properties

plugins { id("com.android.application") }

android {
    namespace = "com.leptoskit.commonplace"
    ndkPath = file("../../.leptoskit/android/ndk-path.txt").readText().trim()
    ndkVersion =
        Properties()
            .apply {
                file("$ndkPath/source.properties").inputStream().use { load(it) }
            }.getProperty("Pkg.Revision") ?: error("Selected Android NDK is missing Pkg.Revision")
    compileSdk = 35
    defaultConfig {
        applicationId = "com.leptoskit.commonplace"
        minSdk = 26
        targetSdk = 35
        versionCode = 1
        versionName = "0.1.0"
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        ndk { abiFilters += file("../../.leptoskit/android/abi.txt").readText().trim() }
    }
    sourceSets["main"].apply {
        kotlin.directories += "../../.leptoskit/android/sources"
        assets.directories += "../../.leptoskit/android/mobile"
        manifest.srcFile("../../.leptoskit/android/AndroidManifest.xml")
    }
    externalNativeBuild { cmake { path = file("../../.leptoskit/android/CMakeLists.txt") } }
    buildTypes {
        // Release shrinks to the code the app reaches: the unminified dex of
        // the Kotlin stdlib and coroutines alone is megabytes. The CLI writes
        // keep rules for the members its JNI library looks up by name to
        // .leptoskit/android on every build; proguard-rules.pro next to this
        // file is for the app's own rules.
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "../../.leptoskit/android/proguard-rules.pro",
                "proguard-rules.pro",
            )
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

dependencies {
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.10.2")
    file("../../.leptoskit/android/plugin-dependencies.txt").readLines().forEach {
        implementation(it)
    }
    androidTestImplementation("androidx.test:runner:1.6.2")
    androidTestImplementation("androidx.test.ext:junit:1.2.1")
    androidTestImplementation("androidx.test.uiautomator:uiautomator:2.3.0")
}
