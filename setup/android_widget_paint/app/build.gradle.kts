plugins { id("com.android.application") }

android {
    namespace = "dev.leptoskit.mobile.paint"
    compileSdk = 35
    defaultConfig {
        applicationId = providers.gradleProperty("paintApplicationId").getOrElse("dev.leptoskit.mobile.paint")
        minSdk = 26
        targetSdk = 35
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
    }
    sourceSets["main"].kotlin.directories +=
        layout.buildDirectory
            .dir("widget-sources")
            .get()
            .asFile.path
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

val nativeWidgetSources by tasks.registering(Sync::class) {
    from("../../../crates/leptoskit_bridge_ffi/mobile") {
        include("NativeWidgetRenderer.kt", "NativeWidgetProps.kt")
    }
    into(layout.buildDirectory.dir("widget-sources"))
}
tasks.named("preBuild") { dependsOn(nativeWidgetSources) }

dependencies {
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.10.2")
    androidTestImplementation("androidx.test:runner:1.6.2")
    androidTestImplementation("androidx.test.ext:junit:1.2.1")
}
