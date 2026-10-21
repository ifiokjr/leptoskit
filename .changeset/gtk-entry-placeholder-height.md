---
"release": patch
---

# Keep GTK text input placeholders inside their frames

GTK text inputs no longer clip their text or placeholder vertically. Yoga sizes an input from its text, but the GTK theme gave every `GtkEntry` a 34px minimum height and the frame placed that taller entry from its top edge, so the centered placeholder was pushed down and cut off at the bottom of the frame, as in the gallery's settings search field. Entries now drop the theme's minimum height, and a native control taller than its frame is centered on it, so clipping removes chrome rather than text.
