//! Harbour, a ferry departures board: the worked example for `leptoskit`'s
//! `query`, `use`, and `rpc` features together.
//!
//! - [`api`] is the harbour's RPC: three leptos server functions published with
//!   `#[api]`, so the spec, the OpenAPI document, and the generated clients in
//!   `clients/` all describe them.
//! - [`board`] reads them through `leptoskit_query`: a cached query per day
//!   that keeps the previous day's board while the next loads, a booking
//!   mutation that drops the seat count before the server answers and puts it
//!   back when the server refuses, and the harbour master's log as an infinite
//!   query.
//! - The board's controls are `leptoskit_use` hooks: a day picker that cycles,
//!   a search that waits for typing to pause, a harbour clock that ticks, a
//!   toggle for departed sailings, and a pinned route kept in storage.
//!
//! [`harbor`] is the in-memory harbour the server functions read and write.

pub mod api;
pub mod board;
pub mod harbor;
