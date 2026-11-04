---
"release": patch
---

# Refuse older code push releases after losing the newest

In the generated iOS and Android apps, a device remembers the newest update it has confirmed in a file of its own. Before this change, a damaged update file or a disk error made the device forget it had ever updated, so a stale or hostile `index.json` could install an older release that was still validly signed. The device now refuses anything at or below the newest release it confirmed, until a new store build of the app ships its own bundle.
