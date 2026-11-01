---
"release": minor
---

# Scale live servers: a relay, roles, read replicas

Live queries and channels now scale past one database reader per server.

- **The live relay.** `rpc::server::Relay` (the `relay` feature, `rpc-relay` on the facade) connects to NATS JetStream. `relay.publish(&db)` publishes the change log from one server at a time, with a lease that another server takes over within five seconds, and `Live::relay(&relay)` makes a server take commits from the stream instead of reading the database. Commits arrive in order and once each, gaps are read from the database, and a server reads the database itself while no one publishes. Channel signals and presence cross between servers. `[rpc.live] relay_url_env` names the URL. `Live::start()` starts a server before its first connection, and `LiveStats` gains `relay_fills`.
- **Roles.** `rpc::server::Roles` reads `--role` or `LEPTOSKIT_ROLE` (`web`, `live`, `worker`, `capture`, any combination, every one by default). The scaffold's database servers serve their pages only with `web`. `leptoskit generate deploy --target kubernetes --split-roles` writes one Deployment per role on Postgres, and the Kubernetes manifests' `PodDisruptionBudget` kind is valid YAML again.
- **Read replicas.** `Db::with_replica(url)` and `[db] replica_url_envs` add Postgres standbys. A live query re-runs on one only once it has replayed past the commits that caused the re-run, and on the primary otherwise, so a screen never shows an older state; `LiveStats` gains `replica_reruns`. Other reads stay on the primary unless the app asks with `db.on_replica()`, whose `SELECT`s go to a replica while writes stay on the primary. Every change log `Commit` now carries `lsn`, the position a replica must reach to show it, and commits serialize.
- **`leptoskit_db`.** `Db::change_log_after(v)` reads one page of the change log without listening.
