---
"release": patch
---

# Fix release Android apps crashing at launch

Release APKs built with `leptoskit native build android --release` crashed on launch with `NoSuchMethodError` for `NativeServiceOwner.wake()`: R8 renamed the Kotlin callbacks the native library looks up by name. The CLI now writes keep rules for exactly those members to `.leptoskit/android/proguard-rules.pro` on every Android build, so they always match the native library they guard, and new projects pass that file to R8.

Apps created before this release need one line in `android/app/build.gradle.kts`. Add the generated file to the release `proguardFiles` list, before your own rules:

```kotlin
proguardFiles(
    getDefaultProguardFile("proguard-android-optimize.txt"),
    "../../.leptoskit/android/proguard-rules.pro",
    "proguard-rules.pro",
)
```

Release builds warn until the line is there. Your own `proguard-rules.pro` can drop its `-keep class dev.leptoskit.bridge.** { *; }` line, which the generated file now carries.
