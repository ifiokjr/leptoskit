# leptoskit_gtk

Real GTK4 Linux widgets consuming leptoskit's shared native mutation stream.

Initialize GTK on the application thread, construct `GtkBackend`, and use it in `NativeSurface`. `open_window` hosts a committed root; `run_application` runs the GTK event loop until the windows close. Controls emit typed events through `set_event_listener`. Application state owns values, including rejected input proposals. Updates preserve native widget identity and editor focus; replacing the root rebinds the existing window.

Retain the surface in `Rc<RefCell<_>>` for an interactive application. Install an event listener holding a `Weak` surface reference, dispatch the event, and reconcile the next `Element` tree. `set_resize_listener` receives logical pixel dimensions from the event loop after native allocation; its callback can safely call `NativeSurface::set_viewport`. Hosts embedding GLib may drive its main context instead of `run_application`.

For a guest that has not committed its first tree yet, `create_window` shows an empty native host. The next committed root attaches to that host automatically. Mount and appearance changes suppress programmatic input notifications, so host setup can safely retain its normal surface borrow.

Every node has a zero-minimum outer GTK frame with exact Yoga geometry. Native interiors preserve their font and toolkit chrome sizes; deliberately tiny outer boxes clip the interior instead of scaling text or expanding the layout.

The backend renders containers, labels and text runs, images, scroll documents, buttons and rich pressables, editable and secure fields, switches, checkboxes, grouped radios, sliders, progress bars, and activity indicators as GTK widgets. Pango provides native text shaping and measurement, including declared writing direction. `GtkTextMeasurer` copies the font-metric hinting GTK applies on the host display; after a window moves to a display with a different scale, call `NativeSurface::refresh_text_measurer`. Images load local file paths and file URLs. The host's explicit color scheme updates GTK's display-wide native theme preference.

GTK sliders use a normalized numeric accessibility range of `0..1`, matching GTK4's native accessibility setters; their accessible value text and spoken description expose the shared numeric value and bounds. The description also reaches older GTK4 AT-SPI implementations that omit value text from their numeric interface. Typed events always use the shared `f64` range and canonical step grid. Radios publish group position relations and a spoken position description because GTK4.8 does not export those relations as AT-SPI attributes. Pressable descendants remain decorative in the accessibility tree.

Requires GTK 4.6 or newer; verified on GTK 4.8.3. On Debian/Ubuntu, install `libgtk-4-dev`, `pkg-config`, `xvfb`, `xauth`, `dbus-x11`, `at-spi2-core`, `fonts-dejavu-core`, and `xdotool`. Run the native harness with:

```sh
GDK_BACKEND=x11 GSK_RENDERER=cairo dbus-run-session -- xvfb-run -a \
  cargo test -p leptoskit_gtk --test widgets
```

The harness opens actual GTK windows, sends pointer and keyboard input, queries and sets AT-SPI values, and compares allocations, text metrics, focus, scrolling, and rendered pixels. Generated fixture images and snapshots go under `target/native-widgets/gtk/<process-id>/`; set `LEPTOSKIT_GTK_ARTIFACTS` to change the parent directory. Each run includes a readable `catalog.png` plus geometry, clipping, paint, rounded-control, and writing-direction artifacts.
