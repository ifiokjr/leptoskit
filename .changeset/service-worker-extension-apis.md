---
"leptoskit": minor
"leptoskit_extension": minor
"leptoskit_service_worker": minor
"leptoskit_cli": patch
---

# Service workers and stronger extension messaging

Web apps can enable the `service-worker` feature to register a worker script, exchange messages, describe caching strategies, open Cache Storage, and manage Web Push subscriptions. Calls return `Unavailable` outside a secure browser context.

Browser extensions gain long-lived `runtime::connect` ports (with `Background::on_connect`), storage `clear` / `keys` / `get_bytes_in_use`, and permissions `get_all` / `on_added` / `on_removed`. The generated background worker keeps ports alive so the wasm side can post and disconnect by id.
