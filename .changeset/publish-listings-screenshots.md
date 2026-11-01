---
"release": minor
---

# Listings from `store/`, and `leptoskit publish screenshots`

Each `leptoskit publish` upload now sends the listing text the stores' APIs take, from the project's `store/` folder: the App Store's description, keywords, promotional text, addresses, and privacy policy; Google Play's name and short and full descriptions; and addons.mozilla.org's name, summary, and description. `store/listing.toml` gains `name` and `keywords`, and `store/en-US/summary.md` holds the short description. Text over a store's limit fails before anything is built, and the steps after each run ask only for what is left.

`leptoskit publish screenshots` captures every store's screenshots into `store/screenshots/<size>/`, from `store/screenshots/screenshots.toml`: browser sizes open a route in headless Chrome at an exact viewport, and simulator and Android sizes drive the installed app with the same steps `leptoskit device` runs.
