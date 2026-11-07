---
"release": minor
---

# Play the examples on the docs site

The screens on leptoskit.dev's examples page and home page were pictures. Each one is now the example's real web build, running in a sandboxed frame on the page: press, type, scroll, and navigate in it. The gallery's seven screens share one module, chosen with `?screen=`, so the browser downloads it once; Observatory and Postcard gained browser clients, and Field, the file routes example, plays too, with a back button that walks its own history. A player loads as it nears the screen, keeps the page from moving while it does, and switches to the screen's AppKit render, so the native views are one press away.

The gallery's dashboard range picker now chooses what the revenue chart covers: seven days, thirty days, or twelve months. The profile's Follow button follows and unfollows, and counts the follower.
