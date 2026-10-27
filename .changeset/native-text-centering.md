---
"release": patch
---

# Centre text and buttons on iOS and Android

Screens laid out with `items-center` now centre their content on the screen in the apps `leptoskit new` generates. Three things kept them off centre:

- The generated iOS and Android apps measured text with a fixed-width approximation, so a label's box rarely matched the text the platform drew in it. They now measure with the platform's own fonts: Core Text on iOS, and the typeface and scaled size Android's text views use.
- A screen's root `View` or `ScrollView` shrank to its content instead of filling the window's width as its `<div>` does in a browser. It now fills the width, less its margins and within its max width; text and control roots keep their intrinsic width.
- Android buttons left-aligned their label and set it in capitals. They now centre it unless the element sets `text-left` or `text-right`, and show the text as written, like iOS and the web.

Apps generated earlier pick up the measurement and alignment changes from the CLI. Their release builds also need `-keepclassmembers class dev.leptoskit.mobile.NativeTextMeasurer { float[] measure(byte[], float, int, float, boolean); }` in `android/app/proguard-rules.pro`.
