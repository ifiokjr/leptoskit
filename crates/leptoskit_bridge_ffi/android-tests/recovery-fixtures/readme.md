# Controlled JVM application recovery tests

`ServiceRecoveryTest.kt` executes production `LeptosKitApplicationView` and `LeptosKitView` methods with deterministic native service completion. Compile production Kotlin against the actual Android platform JAR and coroutines with warnings denied. Compile these four Java boundary doubles against that same Android JAR. Compile the test separately with those classes first on the classpath and the production JAR as a Kotlin friend path. Run `dev.leptoskit.mobile.ServiceRecoveryTestKt` with the boundary classes first.

The doubles replace owner-loop identity, child-removal accounting, status text, and enabled state. Unsafe allocation bypasses Android constructors. No production lifecycle method is copied. The closed update-store sentinel proves that rejection is reached after joined errors without doing native update-store work.

These are controlled JVM tests. They do not establish APK execution, real Android Looper dispatch, rendering, accessibility, update-store persistence, or a real Rust AppService journey. Those require generated application integration tests.
