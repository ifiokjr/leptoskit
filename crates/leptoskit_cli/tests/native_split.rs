//! Feasibility probe: real Rust split output running in wasmi without
//! JavaScript.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;

use leptoskit_cli::split;
use wasmi::Caller;
use wasmi::Engine;
use wasmi::Func;
use wasmi::Instance;
use wasmi::Linker;
use wasmi::Module;
use wasmi::Store;
use wasmi::Val;

#[derive(Default)]
struct Requests {
	callbacks: Vec<(u32, i32)>,
	loads: usize,
}

#[test]
#[ignore = "compiles a Rust wasm guest; needs the wasm32 target and registry access"]
fn rust_chunks_share_memory_and_load_on_demand_without_javascript() {
	let scratch = scratch_directory();
	let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/split_guest");
	let output = Command::new("cargo")
		.current_dir(&fixture)
		.args([
			"build",
			"--locked",
			"--release",
			"--target",
			"wasm32-unknown-unknown",
		])
		.env("CARGO_TARGET_DIR", &scratch)
		.env("RUSTFLAGS", split::EMIT_RELOCS)
		.env_remove("CARGO_ENCODED_RUSTFLAGS")
		.output()
		.expect("compile guest");
	assert!(
		output.status.success(),
		"{}",
		String::from_utf8_lossy(&output.stderr)
	);
	let compiled = scratch.join("wasm32-unknown-unknown/release/split_guest.wasm");
	let bytes = std::fs::read(&compiled).expect("guest");
	assert!(split::uses_lazy_loading(&bytes).expect("imports"));
	let pkg = scratch.join("pkg");
	std::fs::create_dir_all(&pkg).expect("pkg");
	let split = split::split(&bytes, &compiled, &pkg, "split_guest").expect("split Rust guest");
	let manifest: BTreeMap<String, Vec<String>> =
		serde_json::from_slice(&std::fs::read(pkg.join(split::MANIFEST)).expect("manifest"))
			.expect("manifest JSON");
	let chunks = manifest.get("details").expect("details split point");
	assert!(!chunks.is_empty());

	let engine = Engine::default();
	let main = Module::new(&engine, std::fs::read(&split.main).expect("main"))
		.expect("wasmi accepts split main");
	let mut store = Store::new(&engine, Requests::default());
	let mut linker = Linker::new(&engine);
	linker
		.func_wrap(
			&format!("./{}", split::LINK_MODULE),
			"__wasm_split_load_details",
			|mut caller: Caller<'_, Requests>, callback: u32, data: i32| {
				caller.data_mut().callbacks.push((callback, data));
				caller.data_mut().loads += 1;
			},
		)
		.expect("native loader import");
	let instance = linker
		.instantiate_and_start(&mut store, &main)
		.expect("instantiate main without JavaScript");
	let begin = instance
		.get_typed_func::<(), ()>(&store, "begin")
		.expect("begin");
	let poll = instance
		.get_typed_func::<(), i32>(&store, "poll")
		.expect("poll");
	assert_eq!(store.data().loads, 0, "startup loads no chunks");
	begin.call(&mut store, ()).expect("start lazy call");
	assert_eq!(poll.call(&mut store, ()).expect("first poll"), 0);
	assert_eq!(store.data().loads, 1);
	assert_eq!(store.data().callbacks.len(), 1);
	complete_load(instance, &mut store, false);
	assert_eq!(
		poll.call(&mut store, ())
			.expect("failed load is recoverable"),
		2
	);
	begin.call(&mut store, ()).expect("retry lazy call");
	assert_eq!(poll.call(&mut store, ()).expect("retry requests a load"), 0);
	assert_eq!(store.data().loads, 2);

	let mut chunk_linker = Linker::new(&engine);
	chunk_linker
		.instance(&mut store, "__wasm_split", instance)
		.expect("shared exports");

	for stem in chunks {
		let chunk = Module::new(
			&engine,
			std::fs::read(pkg.join(format!("{stem}.wasm"))).expect("chunk"),
		)
		.expect("wasmi accepts chunk");
		let loaded = chunk_linker
			.instantiate_and_start(&mut store, &chunk)
			.expect("link chunk to main memory, table and globals");

		for export in loaded.exports(&store) {
			chunk_linker
				.define("__wasm_split", export.name(), export.into_extern())
				.expect("chunk export");
		}
	}

	complete_load(instance, &mut store, true);
	assert_eq!(poll.call(&mut store, ()).expect("resume lazy call"), 1);
	begin.call(&mut store, ()).expect("second lazy call");
	assert_eq!(poll.call(&mut store, ()).expect("already loaded"), 1);
	assert_eq!(store.data().loads, 2, "successful loads are reused");
	std::fs::remove_dir_all(scratch).expect("remove probe artifacts");
}

fn complete_load(instance: Instance, store: &mut Store<Requests>, success: bool) {
	let table = instance
		.get_table(&*store, "__indirect_function_table")
		.expect("callback table");
	let callbacks = std::mem::take(&mut store.data_mut().callbacks);

	for (callback, data) in callbacks {
		let function = table
			.get(&*store, u64::from(callback))
			.expect("callback slot");
		let function: &Func = function
			.as_func()
			.and_then(Option::from)
			.expect("callback function");
		function
			.call(
				&mut *store,
				&[Val::I32(data), Val::I32(i32::from(success))],
				&mut [],
			)
			.expect("complete load");
	}
}

fn scratch_directory() -> PathBuf {
	let scratch =
		std::env::temp_dir().join(format!("leptoskit-native-split-{}", std::process::id()));
	std::fs::create_dir_all(&scratch).expect("scratch directory");
	scratch
}
