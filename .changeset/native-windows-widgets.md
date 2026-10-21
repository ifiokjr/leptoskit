---
"release": minor
---

# Render shared primitives with Win32 controls on Windows

Add `leptoskit_windows` and the `leptoskit` facade's `windows` feature. The backend renders every existing native primitive with real Win32 windows and standard controls, exact Yoga frames, grouped opacity, per-monitor DPI, controlled inputs, GDI text measurement, MSAA and UI Automation accessibility, scrolling, and lifecycle support. Executables must embed the crate's `app.manifest` to enable version 6 common controls.
