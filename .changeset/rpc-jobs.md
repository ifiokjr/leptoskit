---
"release": minor
---

# Background jobs

`leptoskit_rpc` gains `#[rpc::job]` and the `rpc.jobs` schema pack.

- **`#[rpc::job]`** declares an operation that runs in the background with `rpc::JobCx`: its own database handle, its id, and which attempt this is.
- **Enqueueing** happens in the caller's transaction with `cx.enqueue(job)` from a mutation, so a job exists exactly when the mutation commits. Server code uses `rpc::enqueue(&db, job)` with `.after(…)`, `.at(…)`, `.key(…)`, `.priority(…)`, and `.max_attempts(…)`; a key makes a repeat return the same job.
- **`rpc::server::Jobs`** runs the workers: a claim moves a job from `pending` to `running` under a lease, two workers never run the same job, a dead worker's lease expires, failures retry with exponential backoff, and the last error and result are kept.
- **`use_job`** polls a job's state every second while a component is mounted, and only the caller who enqueued it can read it.
- **Docs.** A background-jobs section in "Real-time screens".
