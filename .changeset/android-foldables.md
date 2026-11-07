---
"leptoskit_plugin_posture": minor
"leptoskit_use": minor
---

# Dual-pane and tabletop layouts for foldables

Fold posture now carries each hinge's `separating` flag (Android `FoldingFeature.isSeparating`), so a fully open continuous foldable stays one pane. `Posture::is_dual_pane`, `layout_mode`, and `use_posture` expose dual-pane, tabletop, book, and single layouts. `DevicePosture::watch` remains how you react to fold-driven configuration changes while the Android activity stays alive. The guide and Android platform page show concrete dual-pane and tabletop examples.
