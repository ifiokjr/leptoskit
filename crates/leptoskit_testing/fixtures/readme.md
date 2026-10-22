# Native driver conformance

These small applications use UIKit, Android Views, AppKit, Windows Forms, and Qt Widgets. They test the native automation boundary independently of Leptoskit's rendering backends. They are not substitutes for end-to-end tests of a production Leptoskit app.

Each journey enters text, clicks buttons, checks the displayed result, opens a real native dialog, captures it, responds, and checks the result returned by the OS. Mobile runs deny and allow notification permission in separate installations. Desktop runs cancel and complete file selection through the native file chooser; on macOS that is an open panel presented as a sheet, which preselects `evidence.txt` beside the fixture bundle. Screenshot assertions reject invalid, tiny, or flat placeholder images, while the HTML reports display original PNGs and accessibility trees.

Build the Rust controller once:

```sh
cargo build --locked -p leptoskit_testing --features native-ui --example native_conformance
npm ci --prefix setup/native-ui
```

Run iOS on macOS with Xcode and an installed simulator runtime:

```sh
bash setup/scripts/native_mobile_run.sh ios SIMULATOR_UDID
```

Run Android with SDK platform 35, build tools 35.0.0, Java, and an emulator/device:

```sh
export ANDROID_HOME=/path/to/android-sdk
bash setup/scripts/native_mobile_run.sh android emulator-5554
```

The mobile helper listens on loopback port 14731 (`NATIVE_DRIVER_PORT` overrides it). It resets **only** `dev.leptoskit.capturelab` between permission outcomes. The SDK and emulator are prerequisites; the CI Android job provisions and boots its emulator. The iOS driver can boot the selected simulator.

Run macOS in an interactive session with Xcode. The script also runs the dialog plugin's fixture, so build it and the journeys first:

```sh
cargo build --locked -p leptoskit_testing --features native-ui --example native_conformance --example dialog_plugin_journey
cargo build --locked -p leptoskit_plugin_dialog --example native_fixture
sudo automationmodetool enable-automationmode-without-authentication
bash setup/scripts/native_macos_run.sh
```

The Mac2 driver moves the real pointer and types into the focused window, so leave the machine alone while it runs, or use a dedicated macOS VM.

Run Windows in an interactive desktop, including a Windows VM:

```powershell
./setup/scripts/native_windows_run.ps1
```

Run Linux with Docker/Podman on a Linux host. Build the controller there so its binary matches the container architecture. KWin's screenshot API needs an OpenGL compositor and an accessible DRM buffer device, even with Mesa software rendering. On a dedicated headless test host whose kernel provides `vgem`, load it with `sudo modprobe vgem` and grant the test user access to its `/dev/dri` nodes. Some Ubuntu kernels package it in `linux-modules-extra-$(uname -r)`; Azure 6.17 omits it entirely. Pass any required host device groups with Docker's `--group-add`.

```sh
docker build --build-arg NATIVE_UID="$(id -u)" --build-arg NATIVE_GID="$(id -g)" -t leptoskit-native-linux -f setup/native-ui/Dockerfile .
docker run --rm --device /dev/dri --user "$(id -u):$(id -g)" -e HOME=/tmp/native-home -v "$PWD:/workspace" leptoskit-native-linux
```

The Linux container starts a private D-Bus session and nested software-rendered KWin compositor. Windows and Linux use desktop-root sessions so dialogs can be found outside the original app window. Apps, drivers, and environments are cleaned up by the wrappers or their dedicated CI runner.

Linux CI pins the Ubuntu 22.04 host for its Azure 6.8 kernel's `vgem` support; the tested desktop is Ubuntu 26.04 inside the container. GitHub retires that host image on [17 April 2027](https://github.com/actions/runner-images/issues/14254). Before then, move the job to a runner that supplies a usable DRM device or a kernel with `vgem`; changing only the runner label is insufficient.

The native CI workflow provisions all five environments and retains `target/native-ui/reports` and driver logs even after failures. Download the artifact and open its `index.html` with the sibling PNG/XML files present.

Appium and its four drivers are pinned in `setup/native-ui/package-lock.json`. Overrides update independently resolvable vulnerable dependencies; some upstream driver bundles/shrinkwraps retain audit findings that npm overrides cannot replace. These development-only servers bind to loopback and run trusted fixture applications on isolated test machines. Do not expose them as public services. KDE's AT-SPI driver is installed separately at a pinned upstream revision; its implementation is not copied into this repository.
