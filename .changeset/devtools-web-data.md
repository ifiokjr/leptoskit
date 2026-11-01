---
"release": minor
---

# Routes, queries, forms, and named state in the DevTools

A browser app under `leptoskit dev` now hands the DevTools what only it knows, through hooks in `leptoskit::devtools` that do nothing in any other build:

- **`track_router(&router)`** reports the current route as its href the moment the app navigates, and the route panel's **Navigate** parses an href and navigates the app, as a link would.
- **`expose("cart.count", signal)`** shows a signal as JSON with its Rust type, and the state panel edits it. Only state named this way appears.
- **`track_form("signup", &state)`**, with the `form` feature, shows a `leptoskit_form` form's values, errors, touched fields, and whether a submit was attempted. Forms are read-only.
- **`track_queries(&client)`**, with the `query` feature, shows every query with its status, fetch status, staleness, observers, timestamps, and data, and every mutation's status. The queries panel refetches, invalidates, and replaces data. Data goes through the scope's codec (`with_ssr` or `with_codec`); a query without one is read-only, its value says so, and the panel offers no **Edit data** for it.

On the app's path a hook only notes that something changed: the agent serializes values in its debounced flush and sends only the ones that changed, with a full snapshot for each new session and whenever a value goes away. In native guests the hooks compile and do nothing yet.

`leptoskit_query`'s new `devtools` feature adds `QueryClient::inspect_queries` and `inspect_mutations`, type-erased access to every cached query and mutation. `leptoskit_form`'s new `devtools` feature makes `FormState` serializable. `leptoskit/devtools` turns both on when the app has `query` or `form`.
