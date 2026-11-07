---
leptoskit_plugin_home_widgets: minor
---

# Home-screen and desktop widgets

`leptoskit_plugin_home_widgets` publishes bounded text snapshots for iOS and macOS WidgetKit and Android App Widgets, then asks the OS to reload them. Guests call `guest::HomeWidgets` for capability, set/get/clear, reload, and installed queries. Apps still own their WidgetKit extensions and App Widget providers; Apple needs an App Group for a shared suite, and Windows, Linux, and the browser remain unavailable.
