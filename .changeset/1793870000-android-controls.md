---
"release": patch
---

# Render Android form controls and dismiss the keyboard

Generated Android apps now render Switch, Checkbox, Radio, Slider, and progress indicators using native Android widgets. The shared native event bridge accepts checked-state and numeric-value events. Tapping outside an editor clears focus and hides the keyboard in both Android renderers. Generated apps move between inputs with the keyboard Next action and dismiss the keyboard on Done.
