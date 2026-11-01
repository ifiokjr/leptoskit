# leptoskit_devtools

The DevTools agents. The `host` agent reports a native surface to the local hub: it opens a session, sends snapshots and commits, and long-polls for the commands a front end sends back. The `web` agent does the same for a browser page.

The hooks hand over what only the app knows: `track_router` for the route, `expose` for named state, `track_form` for a `leptoskit_form` form (the `form` feature), and `track_queries` for a `leptoskit_query` cache (the `query` feature). Outside a `leptoskit dev` build they do nothing.
