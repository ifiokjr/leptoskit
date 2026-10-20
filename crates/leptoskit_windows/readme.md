# leptoskit_windows

Renders shared leptoskit elements as Win32 windows and native controls. The backend owns its HWNDs on one UI thread and consumes the existing Yoga mutation stream; it does not embed a browser or create a second layout tree.

The executable must include the provided `app.manifest` to enable version 6 common controls and layered child windows on Windows 10 and later. The crate embeds this manifest in its own MSVC integration tests and examples. An application must embed it in its executable too. The MSVC linker arguments are `/MANIFEST:EMBED` and `/MANIFESTINPUT:<absolute path to app.manifest>`.

`examples/probe.rs` contains a retained application host. Create `WindowsWindow` first, set the backend DPI to `window.dpi()`, and then construct `NativeSurface`. The resize listener receives logical width, logical height, and monitor DPI. When the DPI changes, call `backend_mut().set_dpi(dpi)` followed by `surface.refresh_text_measurer()` before updating the viewport. GDI font metrics are immutable within each measurement cache generation. The host handles native pixel allocation and the system's suggested DPI rectangle.

Every element has a Yoga-sized frame inside a composition window. The composition window encloses visible overflow and applies subtree opacity once, including overlapping children. Explicit clipping, rounded corners, and scroll viewports constrain the native child hierarchy. Logical ownership and event ancestry remain the shared element tree.

Editors use standard `EDIT` controls. Buttons, switches, checkboxes, radios, and Pressables use standard `BUTTON` controls. Sliders use a standard trackbar whose pointer coordinates map into the shared `f64` domain. Its UI Automation RangeValue provider exposes the actual bounds, value, and step. Progress bars use common controls, and activity indicators use a UI-thread timer. All programmatic commits suppress input callbacks, and native proposals restore application-owned values after callbacks.

Images load ordinary local paths and file URLs through Windows path decoding. PNG, JPEG, GIF, WebP, BMP, and ICO files are supported. Rendering preserves image alpha. Missing or invalid sources produce an `image_error` diagnostic without exposing the source path. The backend does not fetch remote URLs.

Run the native checks on Windows with an interactive desktop:

```powershell
cargo test -p leptoskit_windows --test widgets -- --test-threads=1
cargo run -p leptoskit_windows --example probe -- native-artifacts/windows
```

The probe writes `widgets.png` with only its own client area and `report.json` with the renderer and DPI. Screenshot capture requires the probe window to be visible and unobscured. The tests exercise actual control classes and allocation, accepted and rejected input, queued keyboard handling, MSAA and UI Automation, secure editing, overflow hit testing, grouped opacity pixels, alpha image pixels, both-axis scrolling, DPI, animation shutdown, and host teardown.

Native execution is required to validate this backend. A cross-target Cargo check is supplemental evidence. On a machine without Windows, the renderer verdict remains **NOT VERIFIED** until the Windows tests and screenshot inspection succeed.
