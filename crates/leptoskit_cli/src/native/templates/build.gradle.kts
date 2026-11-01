import java.util.Properties

plugins { id("com.android.application") }

// The app's version, from app.version in leptoskit.toml, which every build
// writes here. `leptoskit publish play` numbers each upload with the
// leptoskit.versionCode property.
val leptoskitVersion =
    Properties().apply {
        file("../../../.leptoskit/android/version.properties").inputStream().use { load(it) }
    }

android {
    namespace = "__PACKAGE__"
    ndkPath = file("../../../.leptoskit/android/ndk-path.txt").readText().trim()
    ndkVersion =
        Properties()
            .apply {
                file("$ndkPath/source.properties").inputStream().use { load(it) }
            }.getProperty("Pkg.Revision") ?: error("Selected Android NDK is missing Pkg.Revision")
    compileSdk = 36
    defaultConfig {
        applicationId = "__PACKAGE__"
        minSdk = 26
        // Google Play takes new apps and updates only when they target
        // API 36 or later.
        targetSdk = 36
        versionCode =
            (
                findProperty("leptoskit.versionCode") as String?
                    ?: leptoskitVersion.getProperty("versionCode")
            ).toInt()
        versionName = leptoskitVersion.getProperty("versionName")
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        ndk { abiFilters += file("../../../.leptoskit/android/abi.txt").readText().trim() }
    }
    sourceSets["main"].apply {
        kotlin.directories += "../../../.leptoskit/android/sources"
        assets.directories += "../../../.leptoskit/mobile"
        // The launcher icons leptoskit makes from app.icon.
        res.directories += "../../../.leptoskit/android/res"
        manifest.srcFile("../../../.leptoskit/android/AndroidManifest.xml")
    }
    externalNativeBuild { cmake { path = file("../../../.leptoskit/android/CMakeLists.txt") } }

    signingConfigs {
        // The upload key, from the environment, so the keystore's path and
        // passwords stay out of the project. Without them, the release is
        // unsigned. Google Play re-signs what it installs with the app
        // signing key it holds.
        val keystore = providers.environmentVariable("LEPTOSKIT_ANDROID_KEYSTORE").orNull

        if (keystore != null) {
            create("release") {
                storeFile = file(keystore)
                storePassword = providers.environmentVariable("LEPTOSKIT_ANDROID_KEYSTORE_PASSWORD").orNull
                keyAlias = providers.environmentVariable("LEPTOSKIT_ANDROID_KEY_ALIAS").orNull
                keyPassword = providers.environmentVariable("LEPTOSKIT_ANDROID_KEY_PASSWORD").orNull
            }
        }
    }
    buildTypes {
        // Release shrinks to the code the app reaches: the unminified dex of
        // the Kotlin stdlib and coroutines alone is megabytes. The CLI writes
        // keep rules for the members its JNI library looks up by name to
        // .leptoskit/android on every build; proguard-rules.pro next to this
        // file is for the app's own rules.
        release {
            signingConfig = signingConfigs.findByName("release")
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "../../../.leptoskit/android/proguard-rules.pro",
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
    file("../../../.leptoskit/android/plugin-dependencies.txt").readLines().forEach {
        implementation(it)
    }
    androidTestImplementation("androidx.test:runner:1.6.2")
    androidTestImplementation("androidx.test.ext:junit:1.2.1")
    androidTestImplementation("androidx.test.uiautomator:uiautomator:2.3.0")
}
