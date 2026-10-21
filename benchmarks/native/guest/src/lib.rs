//! The wasm guest the native benchmark drives through the real rendering ABI.
//!
//! It is the workload every layer under test runs — a 1,000-row list, a
//! single-label update per event, and a full re-render — written as a plain
//! Rust `cdylib` compiled to `wasm32-unknown-unknown`, so the wasm layer is
//! measured executing Rust codegen the way a shipped bundle would, not a
//! hand-written module. The JavaScript guest in the benchmark binary mirrors
//! this file call for call.
//!
//! Wire ids: the root is `1`, row `i` is `2 + i`, and its label is
//! `LABEL_BASE + i`. A press on the root updates one label; a press on the
//! first row re-renders every label.
#![cfg(target_arch = "wasm32")]
#![allow(unsafe_code)] // Exported C names form the interpreter's entry points.

use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering;

#[link(wasm_import_module = "leptoskit")]
unsafe extern "C" {
	fn node_create(id: i64, kind: i32);
	fn node_set_props(id: i64, ptr: i32, len: i32);
	fn node_insert(parent: i64, child: i64, index: i32);
	fn commit();
}

const ROWS: i64 = 1000;
const ROOT: i64 = 1;
const ROW_BASE: i64 = 2;
const LABEL_BASE: i64 = 2 + ROWS;
const KIND_VIEW: i32 = 0;
const KIND_TEXT: i32 = 1;
const PRESS: i32 = 0;

/// Row style as the host's serde shape: enum variants by name, dimensions
/// externally tagged.
const ROW_PROPS: &str = concat!(
	r#"{"style":{"flex_direction":"Row","padding":{"#,
	r#""top":{"Points":4.0},"right":{"Points":8.0},"#,
	r#""bottom":{"Points":4.0},"left":{"Points":8.0}}}}"#
);

static COUNT: AtomicU32 = AtomicU32::new(0);
static GENERATION: AtomicU32 = AtomicU32::new(0);

fn set_props(id: i64, json: &str) {
	// The host copies the bytes during the call, so a borrowed slice is enough.
	unsafe {
		node_set_props(id, json.as_ptr() as i32, json.len() as i32);
	}
}

fn label_json(row: i64, suffix: &str) -> String {
	format!(r#"{{"text":"Row {row}{suffix}","font_size":14}}"#)
}

/// Builds the initial list and commits it.
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_main() {
	unsafe {
		node_create(ROOT, KIND_VIEW);
	}
	set_props(ROOT, r#"{"style":{"flex_direction":"Column"}}"#);
	for row in 0..ROWS {
		let row_id = ROW_BASE + row;
		let label_id = LABEL_BASE + row;
		unsafe {
			node_create(row_id, KIND_VIEW);
			node_create(label_id, KIND_TEXT);
		}
		set_props(row_id, ROW_PROPS);
		set_props(label_id, &label_json(row, ""));
		unsafe {
			node_insert(row_id, label_id, 0);
			node_insert(ROOT, row_id, row as i32);
		}
	}
	unsafe {
		commit();
	}
}

/// Handles an event: a press on the root bumps one label, a press on the
/// first row re-renders every label. Returns `1` when handled.
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_event(id: i64, kind: i32, _ptr: i32, _len: i32) -> i32 {
	if kind != PRESS {
		return 0;
	}
	if id == ROOT {
		let count = COUNT.fetch_add(1, Ordering::Relaxed) + 1;
		set_props(LABEL_BASE, &label_json(0, &format!(" · {count}")));
		unsafe {
			commit();
		}
		return 1;
	}
	if id == ROW_BASE {
		let generation = GENERATION.fetch_add(1, Ordering::Relaxed) + 1;
		let suffix = format!(" · gen {generation}");
		for row in 0..ROWS {
			set_props(LABEL_BASE + row, &label_json(row, &suffix));
		}
		unsafe {
			commit();
		}
		return 1;
	}
	0
}

/// Hands the host a buffer for event payloads; the host never frees it.
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_alloc(len: i32) -> i32 {
	let buffer = vec![0_u8; usize::try_from(len).unwrap_or(0).max(1)];
	Box::leak(buffer.into_boxed_slice()).as_mut_ptr() as i32
}

/// The ABI this guest speaks.
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_abi_version() -> i32 {
	1
}
