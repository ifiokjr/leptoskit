---
"leptoskit_plugin_picture_in_picture": minor
---

# Mobile picture-in-picture and system overlays

Apps can keep a call or video-like surface visible while another app is open. The new `picture-in-picture` runtime plugin reports capabilities and the active session, enters Android activity picture-in-picture as a cancellable lease, and — after the user grants `SYSTEM_ALERT_WINDOW` in system settings — holds an Android system overlay window. Overlay status and `request_overlay` open the grant screen without inventing a permission. iOS reports whether system PiP is supported but does not yet enter it without an attached video player, and has no draw-over-apps overlay. Desktop and the browser return `Unavailable`.
