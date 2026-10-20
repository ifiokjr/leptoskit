//! Code splitting tests: spotting `#[lazy]` code in a compiled client, and
//! reading which chunks load on demand.

use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use leptoskit_cli::split;

struct Scratch {
	path: PathBuf,
}

impl Scratch {
	fn new(tag: &str) -> Self {
		static COUNTER: AtomicU64 = AtomicU64::new(0);
		let unique = COUNTER.fetch_add(1, Ordering::SeqCst);
		let path = std::env::temp_dir().join(format!(
			"leptoskit_split_test_{tag}_{}_{}",
			std::process::id(),
			unique
		));
		std::fs::create_dir_all(&path).expect("scratch");
		Self { path }
	}
}

impl Drop for Scratch {
	fn drop(&mut self) {
		let _ = std::fs::remove_dir_all(&self.path);
	}
}

/// A wasm section: its id, then its length-prefixed body. Every body here is
/// shorter than 128 bytes, so each length is one LEB128 byte.
fn section(id: u8, body: &[u8]) -> Vec<u8> {
	let mut bytes = vec![id, u8::try_from(body.len()).expect("short section")];
	bytes.extend_from_slice(body);
	bytes
}

fn name(text: &str) -> Vec<u8> {
	let mut bytes = vec![u8::try_from(text.len()).expect("short name")];
	bytes.extend_from_slice(text.as_bytes());
	bytes
}

/// A module importing one `() -> ()` function from each of `modules`.
fn module_importing(modules: &[&str]) -> Vec<u8> {
	let mut wasm = b"\0asm\x01\0\0\0".to_vec();
	wasm.extend(section(1, &[1, 0x60, 0, 0]));
	if !modules.is_empty() {
		let mut imports = vec![u8::try_from(modules.len()).expect("few imports")];
		for module in modules {
			imports.extend(name(module));
			imports.extend(name("f"));
			imports.extend([0x00, 0x00]);
		}
		wasm.extend(section(2, &imports));
	}
	wasm
}

#[test]
fn lazy_code_shows_as_an_import_from_the_split_placeholder() {
	assert!(
		split::uses_lazy_loading(&module_importing(&["wbg", split::PLACEHOLDER_MODULE]))
			.expect("parse")
	);
	assert!(!split::uses_lazy_loading(&module_importing(&["wbg", "env"])).expect("parse"));
	assert!(!split::uses_lazy_loading(&module_importing(&[])).expect("parse"));
	assert!(split::uses_lazy_loading(b"not wasm").is_err());
}

#[test]
fn lazy_files_come_from_the_split_manifest() {
	let scratch = Scratch::new("manifest");
	let pkg = &scratch.path;
	assert!(
		split::lazy_files(pkg).expect("no manifest").is_empty(),
		"an unsplit build has no lazy files"
	);

	std::fs::write(
		pkg.join(split::MANIFEST),
		r#"{"mixer_loader_1": ["chunk_0", "split_mixer_loader_1"], "chart_2": ["chunk_0"]}"#,
	)
	.expect("manifest");
	let lazy: Vec<PathBuf> = split::lazy_files(pkg)
		.expect("manifest")
		.into_iter()
		.collect();
	assert_eq!(
		lazy,
		[
			pkg.join("chunk_0.wasm"),
			pkg.join("split_mixer_loader_1.wasm")
		]
	);

	std::fs::write(pkg.join(split::MANIFEST), "not json").expect("manifest");
	assert!(split::lazy_files(pkg).is_err());
}
