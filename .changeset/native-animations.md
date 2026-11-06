---
"release": minor
---

# Animations that run on the platform

Widgets can now move, scale, and rotate, and animate changes without the app computing a single frame. Tailwind's `translate-*`, `scale-*`, `rotate-*`, `transition-*`, `duration-*`, `delay-*`, `ease-*`, and `animate-spin`/`ping`/`pulse`/`bounce` classes work, as does the `starting:` variant for the look a widget enters from. For anything else, elements take `transform`, `transition`, `keyframes`, `entering`, `exiting`, and `layout_transition` values from the new `leptoskit_animation` crate (re-exported as `leptoskit::native::animation`), with springs, timing curves, and presets modelled on React Native Reanimated.

The guest describes an animation once and the platform runs it: Core Animation in the generated iOS and macOS apps, Android's animators in the generated Android app, and CSS in the browser. The web doesn't run `exiting` or `layout_transition` yet, and Windows, Linux, the terminal, and HarmonyOS ignore transforms and animations for now.
