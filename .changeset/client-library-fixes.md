---
"release": patch
---

# Fix a query deadlock, stale async results, and routing

Fixes in the client libraries:

- `leptoskit_query`: a watcher that reacts to a query starting to fetch, by cancelling, refetching, or invalidating it, no longer freezes the app. The fetch used to hold a lock while it notified watchers.
- `leptoskit_use`: `use_async_state` keeps the newest result. When two runs overlap and the older one finishes last, its result no longer replaces the newer one.
- `leptoskit_router`: a route under a catch-all, such as `[...path]/edit.rs`, is now reachable. `[...path]/index.rs` used to match `/a/edit` first.
- `leptoskit_l10n`: `negotiate` keeps looking through every following preference in the same language before settling for a language-only match. `[en-GB, en-AU, en-US]` now gets `en-US` from `[en, en-US]` instead of `en`.
- `leptoskit_virtual`: measuring an item at its estimated size no longer recomputes the position of every item after it. Measuring the rows that scroll into view in the middle of a 100,000-item list used to recompute 50,000 positions; now it recomputes none.
- `leptoskit_aria`: `Field::wire` replaces the field's own help and error references instead of adding them again, so wiring an input twice no longer makes a screen reader read its help twice, and an error the field no longer shows is no longer referenced.
