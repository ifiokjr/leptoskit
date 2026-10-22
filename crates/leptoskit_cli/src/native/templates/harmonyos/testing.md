# Test this HarmonyOS app

Open this folder in DevEco Studio, run `ohpm install`, and start a compatible local emulator from **Tools → Device Manager → Local Emulator**, or connect a HarmonyOS device with debugging enabled. Use a full emulator/device for the Rust native library; ArkUI Previewer alone does not test the bridge.

From the application root, run `leptoskit native build harmonyos` for arm64 or `leptoskit native build harmonyos-x86_64` for an x86-64 target. Match the emulator's ABI, not just the development computer's CPU. Configure debug signing in DevEco and run **entry** once on that target.

Stop the app before each test run so its counter starts at zero. Right-click `entry/src/ohosTest/ets/test/Counter.test.ets` and run it as an **Instrument Test** on the same target. It starts `__BUNDLE_NAME__/EntryAbility`, finds the real native controls, taps Increment three times, and checks the displayed count after every tap. Assertions wait for components with a deadline, with no fixed sleeps. Update this starter journey when you replace the counter.

For command-line installation, instrumentation, failure artifacts, SDK-free tests, and a manual device checklist, see the [HarmonyOS testing guide](https://ifiokjr.github.io/leptoskit/platforms/harmonyos.html#testing).

The test sources and this file belong to your app and are preserved by repeated prepares. `leptoskit test run` currently supports desktop, iOS, and Android executables; it does not install or run HarmonyOS test HAPs. Use DevEco Instrument Test or `hdc` for this platform.
