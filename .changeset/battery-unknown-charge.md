---
"release": patch
---

# Battery readings that aren't a charge read as unknown

On desktops, a power source whose charge gauge reports a value that isn't a fraction of full, such as a UPS or an uncalibrated battery, made `leptoskit_plugin_battery`'s snapshot fail with `InvalidArgument`. That reading's charge is now `None`, and a gauge slightly past full reads as full.
