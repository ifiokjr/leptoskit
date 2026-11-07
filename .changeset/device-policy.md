---
"leptoskit_plugin_device_policy": minor
---

# Android device-policy plugin

`leptoskit_plugin_device_policy` exposes an explicit, least-privilege Device Admin / Device Policy Manager surface for Android: status without prompting, user-driven activation, lock, password quality and minimum length, camera disable, confirmed wipe, and admin removal. Non-Android platforms return `Unavailable`. Apps own the manifest receiver and the `device_admin.xml` policy file so they declare only the policies they need. Wipe requires `WipeRequest::factory_reset()`; empty payloads cannot factory-reset a device. Google Play and enterprise-policy constraints still apply and are documented with the API.
