---
"leptoskit_plugin_local_authentication": minor
"leptoskit_plugin_location": minor
"leptoskit_plugin_notifications": minor
"leptoskit_cli": minor
---

# Add foreground native services and readable Android chrome

Add independent local device-owner authentication, foreground location and immediate local notification crates. Selected services register through Swift/Kotlin metadata. Permission prompts remain explicit, purpose strings remain app-owned, and cancellation/close release native work. Browser adapters expose supported location and notification operations.

Authentication does not identify an account or authorize a server request. Notification scheduling, push registration and action routing, background location, and Windows/Linux local authentication remain unimplemented. Generated Android applications use an app-owned light theme with dark system-bar icons.
