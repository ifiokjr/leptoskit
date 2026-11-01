# leptoskit_devtools

The DevTools agents. The `host` agent reports a native host to the local hub: a `Reporter` opens a session, mirrors each guest's tree from the mutations its renderer applies, sends snapshots, patches, and commits from a thread of its own, and long-polls for the commands a front end sends back. The `web` agent reports a browser page. The `guest` agent runs in a native Wasm guest and sends the hooks' values through the host's `DevTools` plugin (`leptoskit_plugin_devtools`); `Reporter::with_guests` and `Source::guest` merge them into the host's session and pass the hub's commands for them back.

The hooks hand over what only the app knows: `track_router` for the route, `expose` for named state, `track_form` for a `leptoskit_form` form (the `form` feature), and `track_queries` for a `leptoskit_query` cache (the `query` feature). Outside a `leptoskit dev` build they do nothing.
