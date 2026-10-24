# Native widget paint tests

This Android application exercises the shipping Kotlin renderer on a real Android runtime. Gradle copies the two renderer sources directly from their owners before compilation.

Run `gradle :app:connectedDebugAndroidTest` with the project Android SDK and Java 17. Select the intended test device through `ANDROID_SERIAL`.

The regression case uses the observed shared 320 by 16 layout-unit frame and 14 sp label. It compares actual native draw pixels with a taller frame, preserves button state and dimensions, checks local font scaling, and injects a real screen touch. PNGs remain in the test application's files directory. The comparison test records padding-only and font-padding alternatives without changing the shipping control.

This fixture does not run the Solana SDK application journey or establish its lifecycle acceptance.
