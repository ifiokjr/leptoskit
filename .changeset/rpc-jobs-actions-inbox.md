---
"release": minor
---

# Add job steps, cron, actions, and an inbox

Background jobs gain everything slow, unreliable work needs: `cx.step(…)` saves each step's result so a retry resumes where the last attempt stopped, `cx.key()` gives outside calls an idempotency key that is the same on every attempt, `cx.mutate(…)` applies a mutation once per job and step, and `rpc::Error::retry` and `rpc::Error::permanent` say whether to try again. `#[rpc::job]` takes `queue`, `retries`, `backoff = exponential(…)`, `deadline`, `timeout`, `serialize_by`, `visible_to`, and `on_failure`; queues have a concurrency that holds across servers, configured in `[rpc.jobs]`. `#[rpc::cron("…")]` enqueues a job once per tick however many servers run. Jobs report progress, can be cancelled, are visible only to whoever enqueued them, and wake workers the moment they are enqueued. `use_job` follows a job live over the socket.

`#[rpc::action]` declares work someone waits for that may call anything: it runs as the caller, outside any transaction, is never retried, and is called over `POST /api/<name>`, from generated clients, and with `use_action`.

`cx.notify(account, notice)` puts a notice in a person's inbox from a mutation, a job, or an action, and `use_inbox()` shows it on every device they have open, with an unread count, `mark_read`, and `mark_all_read`.

`leptoskit jobs list`, `retry`, and `cancel` operate on any deployment's jobs, and `rpc::testing::TestJobs` runs jobs in tests on a clock the test moves.
