---
"release": patch
---

# Entrances let go once they have played

On the web, a motion that plays from a pose to the element's own style, whether it enters, scrolls into view (`in_view`), or follows a timeline, now lets go of the element once it has played instead of holding its last keyframe. A reveal that clips from an inset therefore no longer keeps clipping what the element paints outside its box, such as a heading's descenders. Keyframes that end on a pose of their own, at 1.0, still hold it.

The leptoskit.dev landing page and documentation get the same care:

- Section titles show their descenders, set display headings at a looser leading from shared tokens, and wrap each line of a title evenly.
- The hero's fan of screens no longer covers the copy on tablets and small laptops.
- The widget map fits a phone with reduced motion.
- The documentation's hero mark lands smoothly, and stays still for people who ask for less motion.
- The documentation's page titles no longer widen the page on a tablet, and its hero title keeps each sentence whole on a phone.
- Small coral text and the "Get started" button meet WCAG AA contrast in light and dark.
