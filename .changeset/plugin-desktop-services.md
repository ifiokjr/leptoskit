---
"leptoskit_plugin_global_shortcut": minor
"leptoskit_plugin_window_state": minor
---

# Add owned desktop shortcuts and window placement

Add independent global-shortcut and window-state crates. Shortcuts require explicit event-loop ownership, bound registrations and event queues, and release native registrations on teardown. Window placement validates portable geometry; macOS capture/restore operates on an explicitly owned window, with persistence controlled by the application.

Wayland shortcuts and Windows/Linux native window capture/restore remain unimplemented. Extend the generated Android service fixture with logging/crypto worker assertions, scoped service checks and system-bar verification. The catalog records all eight service crates and the crypto extension with operation-level platform and registration limits.
