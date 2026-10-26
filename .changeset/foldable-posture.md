---
"release": minor
---

# Know how a foldable phone is folded

The new `posture` runtime plugin, `leptoskit_plugin_posture`, reports whether a foldable is closed on its cover screen, half open, or open flat, with every hinge that crosses the app's window: its axis, its position as fractions of the window, its angle, and whether it hides content. `Posture::panes` splits the window around the folds for percentage layouts, and `is_tabletop` and `is_book` name the two half-open postures. `leptoskit_use::use_posture` turns a posture signal into memoized queries.

iOS 27.1 reads the iPhone Duo's hinge through `UIHingeInteraction` and the fold's position through the view's division reserved region. Android reads Jetpack WindowManager's folding features and the hinge angle sensor, so book folds, flips, and trifolds report each fold. Browsers read the Device Posture and Viewport Segments APIs.

Generated Android apps now also handle `screenLayout` and `smallestScreenSize` configuration changes. Before, folding or unfolding a foldable recreated the activity and restarted the guest, losing the app's state. Apps generated earlier should add `screenLayout|smallestScreenSize` to `android:configChanges` in `android/app/src/main/AndroidManifest.xml`.

A guest that fails to start on iOS or macOS now shows its error. The message label was sized from the view's bounds at the moment of failure, which are still empty during launch, so the error was laid out off screen and the app showed a blank window.
