---
"release": minor
---

# Add generated artwork to the documentation book

`leptoskit_docs::Site::generated_asset` adds a file your build generates, such as SVG artwork or a stylesheet, to the book under `_leptoskit/generated/`, without writing it into the book's sources. The file is kept through development-server rebuilds, and an invalid file name fails the build before any output changes.

The LeptosKit website and book use it for their hand-drawn look: borders, hatching, and artwork drawn at build time with skribble's rough engine, in a paper-and-ink palette for light and dark modes, with locally served fonts.
