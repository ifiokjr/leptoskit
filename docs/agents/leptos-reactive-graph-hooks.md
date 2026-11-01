# Proposal: observer hooks in `reactive_graph`

Status: **draft for upstream**, to be posted on [leptos#3466](https://github.com/leptos-rs/leptos/issues/3466). This is phase 6 item 2 of the [devtools plan](devtools-plan.md). leptoskit does not fork `reactive_graph`: the signal graph view in DevTools waits for hooks like these to land upstream.

## The problem

A developer debugging a Leptos app wants to see why the screen changed: which signal was set, which memos re-ran, which effects fired, and which nodes they read. Every other framework's DevTools answers that (React's profiler, Solid's [solid-devtools](https://github.com/thetarnav/solid-devtools), Vue's timeline). Leptos can't today. `reactive_graph` keeps its sources and subscribers private, and the only outside view is the `tracing` feature's spans, which name functions but not the graph.

leptoskit already shows everything it owns (the element tree, styles, routes, queries, forms, named state, plugin and server calls) without touching `reactive_graph`. Signals an app names with `leptoskit::devtools::expose` are shown and editable. What no library on top of `reactive_graph` can show is the unnamed rest of the graph and the edges between its nodes.

## The proposal

A cargo feature, `observer`, off by default, that compiles to nothing without it. With it, `reactive_graph` calls one process-wide observer at each graph event:

```rust
#[cfg(feature = "observer")]
pub mod observer {
	use core::panic::Location;

	/// A node in the graph, stable for its lifetime.
	#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
	pub struct NodeId(pub u64);

	#[derive(Clone, Copy, Debug, PartialEq, Eq)]
	pub enum NodeKind {
		Signal,
		Memo,
		Effect,
		RenderEffect,
		Resource,
		Owner,
	}

	pub trait Observer: Send + Sync + 'static {
		/// A node was created, with the caller location `#[track_caller]`
		/// already records for its `defined_at`.
		fn created(&self, node: NodeId, kind: NodeKind, defined_at: Option<&'static Location<'static>>, owner: Option<NodeId>);
		/// A node was disposed.
		fn disposed(&self, node: NodeId);
		/// `subscriber` started or stopped depending on `source`.
		fn subscribed(&self, source: NodeId, subscriber: NodeId);
		fn unsubscribed(&self, source: NodeId, subscriber: NodeId);
		/// A signal's value was replaced or updated in place.
		fn set(&self, signal: NodeId);
		/// A memo or effect started and finished running.
		fn run_started(&self, node: NodeId);
		fn run_finished(&self, node: NodeId, changed: bool);
	}

	/// Installs the observer. A second call replaces the first.
	pub fn set_observer(observer: Box<dyn Observer>);
}
```

Design choices, each open to the maintainers' taste:

- **Ids, not values.** The observer sees node ids and kinds, never the values, so the hooks impose no bounds on signal types. A tool that wants a value reads it through the app, as leptoskit's `expose` does.
- **One global observer.** DevTools is one consumer per process. A list of observers can come later without changing call sites.
- **Off the hot path.** Each call site is one branch on an atomic "observer installed" flag when the feature is on, and nothing when it is off. Apps build with the feature only in development, as leptoskit's `devtools` feature already works.
- **Locations are free.** `reactive_graph` already captures `defined_at` under `debug_assertions`; the observer receives the same `Location`, so a tool can link each node to its source line.

## What leptoskit would do with it

- A **Signals** panel listing each live signal, memo, and effect with its kind, owner, and source line, linked to the element tree by the render effects that write each element.
- A **why did this render** view: for a commit, the chain from the signal that was set, through the memos that re-ran, to the render effects whose elements changed. The commit observer and the element tree already exist; the hooks supply the edges.
- **Leptos component boundaries** for `leptos::component` functions, from their owners, beside the `#[component]` boundaries leptoskit records itself.

The agent would keep the budget the plan sets: bounded buffers, serialization off the render path, and nothing at all in release builds.

## Alternatives considered

- **The `tracing` feature.** It names component and effect functions as spans, but carries no graph edges, costs a `tracing` subscriber in every development build, and has no id to tie a span to a signal. Phase 6 item 1 uses leptoskit's own `#[component]` instead (see the devtools plan's Progress).
- **A fork of `reactive_graph`.** Rejected: leptoskit apps depend on the same `reactive_graph` as every Leptos crate they use, and a fork would split the graph.
- **Wrapping signal constructors in leptoskit.** Only covers signals created through leptoskit, misses every Leptos internal, and still can't see subscriptions.
