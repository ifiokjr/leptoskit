<h1 align="center">leptoskit_windows</h1>

<p align="center">
  <strong>leptoskit_windows</strong> renders shared leptoskit elements as Win32 windows and native controls, with every element owning a child window and interactive elements using real Windows controls.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_windows"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_windows"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_windows.svg"></a>
  <a href="https://docs.rs/leptoskit_windows"><img alt="docs.rs" src="https://docs.rs/leptoskit_windows/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

Renders shared leptoskit elements as Win32 windows and native controls. The backend owns its HWNDs on one UI thread and consumes the existing Yoga mutation stream; it does not embed a browser or create a second layout tree.

The executable must include the provided `app.manifest` to enable version 6 common controls and layered child windows on Windows 10 and later. The crate embeds this manifest in its own MSVC integration tests and examples. An application must embed it in its executable too. The MSVC linker arguments are `/MANIFEST:EMBED` and `/MANIFESTINPUT:<absolute path to app.manifest>`.

`examples/probe.rs` contains a retained application host. Create `WindowsWindow` first, set the backend DPI to `window.dpi()`, and then construct `NativeSurface`. The resize listener receives logical width, logical height, and monitor DPI. When the DPI changes, call `backend_mut().set_dpi(dpi)` followed by `surface.refresh_text_measurer()` before updating the viewport. GDI font metrics are immutable within each measurement cache generation. The host handles native pixel allocation and the system's suggested DPI rectangle.

Every element has a Yoga-sized frame inside a composition window. The composition window encloses visible overflow and applies subtree opacity once, including overlapping children. Explicit clipping, rounded corners, and scroll viewports constrain the native child hierarchy. A transparent, unclipped container draws nothing of its own, so overlapping siblings beneath it stay visible and receive input. A translucent background on a childless, borderless container, such as a modal scrim, blends over whatever lies beneath. Classic child windows cannot blend their own content per pixel, so a translucent background behind text, controls, or children is flattened over the nearest ancestor background. Logical ownership and event ancestry remain the shared element tree.

Editors use standard `EDIT` controls. Buttons, switches, checkboxes, radios, and Pressables use standard `BUTTON` controls. Sliders use a standard trackbar whose pointer coordinates map into the shared `f64` domain. Its UI Automation RangeValue provider exposes the actual bounds, value, and step. Progress bars use common controls, and activity indicators use a UI-thread timer. All programmatic commits suppress input callbacks, and native proposals restore application-owned values after callbacks.

Images load ordinary local paths and file URLs through Windows path decoding. PNG, JPEG, GIF, WebP, BMP, and ICO files are supported. Rendering preserves image alpha. Missing or invalid sources produce an `image_error` diagnostic without exposing the source path. The backend does not fetch remote URLs.

Run the native checks on Windows with an interactive desktop.

The probe writes `widgets.png` with only its own client area and `report.json` with the renderer and DPI. Screenshot capture requires the probe window to be visible and unobscured. The tests exercise actual control classes and allocation, accepted and rejected input, queued keyboard handling, MSAA and UI Automation, secure editing, overflow hit testing, grouped opacity pixels, alpha image pixels, both-axis scrolling, DPI, animation shutdown, and host teardown.

Native execution is required to validate this backend. A cross-target Cargo check is supplemental evidence. On a machine without Windows, the renderer verdict remains **NOT VERIFIED** until the Windows tests and screenshot inspection succeed.

## How it fits

The backend owns its HWNDs on one UI thread and consumes the existing Yoga mutation stream; it never embeds a browser or creates a second layout tree. The executable includes the provided manifest for version 6 common controls and layered-window support.

## Use it

```powershell
cargo test -p leptoskit_windows --test widgets -- --test-threads=1
cargo run -p leptoskit_windows --example probe -- native-artifacts/windows
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_windows)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Windows](https://ifiokjr.github.io/leptoskit/platforms/windows-linux.html)
