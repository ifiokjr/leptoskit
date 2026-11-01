---
leptoskit_windows: breaking
---

# leptoskit_windows: the Windows backend

`leptoskit_windows` renders shared leptoskit elements as Win32 windows and native controls, with every element owning a child window and interactive elements using real Windows controls.

The backend owns its HWNDs on one UI thread and consumes the existing Yoga mutation stream; it never embeds a browser or creates a second layout tree. The executable includes the provided manifest for version 6 common controls and layered-window support.
