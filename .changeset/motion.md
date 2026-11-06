---
"release": minor
---

# Motion for the shared component tree

Elements can move. `motion(Motion)` describes the poses an element moves between, as opacity, offset, scale, rotation, clipping, and blur, and what drives the movement: appearing (`Motion::enter`), scrolling into view (`in_view`), or a named container scrolling past (`timeline` on the container and `follow` on the elements inside it). `stagger` on a container starts its children's motions one after another. Timing comes from a duration and an `Easing`, or from a `Spring`, and a new `key` plays an enter motion again.

The web renders motion as CSS animations and scroll-driven animations, with springs as `linear()` easing, so nothing runs in script. People who ask for less motion, and browsers without scroll timelines, see every element at rest at once. Native platforms show the resting pose too: motion is web-only for now, and native apps never receive it, so adding it changes nothing they build.

The leptoskit.dev landing page is redesigned around it: one component tree becomes each platform's own widgets as you scroll, with every movement written as `Motion` and the hand-written animation script removed.
