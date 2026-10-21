//! Generating `routes.rs` from route trees on disk.

use std::path::Path;
use std::path::PathBuf;

use leptoskit_router_codegen::ExportStyle;
use leptoskit_router_codegen::RouteKind;
use leptoskit_router_codegen::Status;
use leptoskit_router_codegen::WriteError;

/// A routes directory in a temporary project.
struct Tree {
	project: tempfile::TempDir,
}

impl Tree {
	fn new(files: &[(&str, &str)]) -> Self {
		let project = tempfile::tempdir().expect("temp dir");
		for (path, source) in files {
			let path = project.path().join("routes").join(path);
			std::fs::create_dir_all(path.parent().expect("a parent")).expect("create dirs");
			std::fs::write(path, source).expect("write route file");
		}
		Self { project }
	}

	fn dir(&self) -> PathBuf {
		self.project.path().join("routes")
	}

	fn output(&self) -> PathBuf {
		leptoskit_router_codegen::output_path(&self.dir())
	}
}

const PAGE: &str = "pub fn page() {}";

#[test]
fn generates_the_module_and_route_table() {
	let tree = Tree::new(&[
		(
			"index.rs",
			"#[derive(Route, Clone, Debug, PartialEq)]\npub struct Today;\npub fn page() {}",
		),
		("_layout.rs", "pub fn layout() {}"),
		("+not-found.rs", PAGE),
		("notes/index.rs", PAGE),
		("notes/_card.rs", "pub fn card() {}"),
		(
			"notes/[note].rs",
			"#[derive(Route, Clone, Debug, PartialEq)]\npub struct Note {\n\tpub note: \
			 u64,\n\tpub highlight: Option<u32>,\n}\n\n#[component]\npub fn Page() \
			 {}\n\n#[component]\npub fn Loading() {}",
		),
		("(auth)/sign-in.rs", PAGE),
		("README.md", "Only Rust files are routes."),
	]);
	let generated = leptoskit_router_codegen::generate(&tree.dir()).expect("valid tree");
	insta::assert_snapshot!(generated.source);

	let table: Vec<_> = generated
		.routes
		.iter()
		.map(|route| {
			(
				route.pattern.as_str(),
				route.name.as_str(),
				route.file.to_string_lossy().into_owned(),
				route.kind,
				route.page,
			)
		})
		.collect();
	assert_eq!(
		table,
		[
			(
				"/",
				"Today",
				"index.rs".to_owned(),
				RouteKind::Page,
				ExportStyle::Function
			),
			(
				"/notes",
				"Notes",
				"notes/index.rs".to_owned(),
				RouteKind::Page,
				ExportStyle::Function
			),
			(
				"/sign-in",
				"SignIn",
				"(auth)/sign-in.rs".to_owned(),
				RouteKind::Page,
				ExportStyle::Function
			),
			(
				"/notes/:note",
				"Note",
				"notes/[note].rs".to_owned(),
				RouteKind::Page,
				ExportStyle::Component
			),
			(
				"/*path?",
				"NotFound",
				"+not-found.rs".to_owned(),
				RouteKind::NotFound,
				ExportStyle::Function
			),
		]
	);
}

#[test]
fn reports_every_problem_with_its_file_and_line() {
	let tree = Tree::new(&[
		("index.rs", PAGE),
		// A dynamic route needs a struct.
		("[slug].rs", PAGE),
		// The struct must name every path parameter, as the right kind.
		(
			"users/[id]/posts/[...rest].rs",
			"#[derive(Route)]\npub struct Post {\n\tpub id: Option<u64>,\n}\npub fn page() {}",
		),
		// Capitalised exports need `#[component]`; one spelling only.
		("about.rs", "pub fn Page() {}"),
		(
			"contact.rs",
			"pub fn page() {}\n\n#[component]\npub fn Page() {}",
		),
		// A route file without a page, and a layout with one.
		("empty.rs", "pub fn helper() {}"),
		("(shop)/_layout.rs", "pub fn page() {}\npub fn layout() {}"),
		// Two files for the same URL.
		("(shop)/cart.rs", PAGE),
		("(store)/cart.rs", PAGE),
		// Names that are not routes.
		("_shared/util.rs", PAGE),
		("feed.web.rs", PAGE),
		("+layout.rs", PAGE),
		("news.rs", PAGE),
		("news/index.rs", PAGE),
		("broken.rs", "pub fn page( {"),
	]);
	let diagnostics = leptoskit_router_codegen::generate(&tree.dir()).expect_err("broken tree");
	let lines: Vec<String> = diagnostics
		.0
		.iter()
		.map(|diagnostic| {
			let file = diagnostic
				.file
				.strip_prefix(tree.project.path())
				.expect("inside the project");
			match diagnostic.line {
				Some(line) => format!("{}:{line}: {}", file.display(), diagnostic.message),
				None => format!("{}: {}", file.display(), diagnostic.message),
			}
		})
		.collect();
	insta::assert_snapshot!(lines.join("\n"));
}

#[test]
fn status_ignores_formatting_and_item_order() {
	let tree = Tree::new(&[
		("index.rs", PAGE),
		("notes/index.rs", PAGE),
		("about.rs", PAGE),
	]);
	let generated = leptoskit_router_codegen::generate(&tree.dir()).expect("valid tree");
	let output = tree.output();
	assert_eq!(
		leptoskit_router_codegen::status(&output, &generated).expect("status"),
		Status::Missing
	);
	assert!(leptoskit_router_codegen::write(&output, &generated).expect("write"));
	assert!(!leptoskit_router_codegen::write(&output, &generated).expect("write"));

	// rustfmt with hard tabs, and module declarations in another order.
	let mut reformatted = generated.source.replace("    ", "\t");
	let first = "#[path = \"routes/about.rs\"]\npub mod about;\n";
	assert!(reformatted.contains(first), "{reformatted}");
	reformatted = reformatted.replacen(first, "", 1);
	reformatted.push_str(first);
	std::fs::write(&output, &reformatted).expect("reformat");
	assert_eq!(
		leptoskit_router_codegen::status(&output, &generated).expect("status"),
		Status::Current
	);

	let edited = reformatted.replace("Notes", "Changed");
	std::fs::write(&output, edited).expect("edit");
	assert_eq!(
		leptoskit_router_codegen::status(&output, &generated).expect("status"),
		Status::Stale
	);
	assert!(leptoskit_router_codegen::write(&output, &generated).expect("write"));
	assert_eq!(
		std::fs::read_to_string(&output).expect("read"),
		generated.source
	);
}

#[test]
fn never_overwrites_a_hand_written_module() {
	let tree = Tree::new(&[("index.rs", PAGE)]);
	let generated = leptoskit_router_codegen::generate(&tree.dir()).expect("valid tree");
	let output = tree.output();
	std::fs::write(&output, "// my routes\npub mod index;\n").expect("hand-written");
	let error = leptoskit_router_codegen::write(&output, &generated).expect_err("refuses");
	assert!(matches!(error, WriteError::NotGenerated(_)), "{error:?}");
	assert_eq!(
		std::fs::read_to_string(&output).expect("read"),
		"// my routes\npub mod index;\n"
	);
}

#[test]
fn an_empty_or_missing_directory_is_reported() {
	let tree = Tree::new(&[("notes.txt", "")]);
	let error = leptoskit_router_codegen::generate(&tree.dir()).expect_err("no routes");
	assert!(error.to_string().contains("no route files"), "{error}");
	let missing = tree.project.path().join("missing");
	let error = leptoskit_router_codegen::generate(&missing).expect_err("missing");
	assert!(error.to_string().contains("does not exist"), "{error}");
}

/// The example's committed `routes.rs` must match its route files, the same
/// check `leptoskit routes --check` runs.
#[test]
fn the_file_routes_example_is_current() {
	let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/file_routes/src/routes");
	let generated = leptoskit_router_codegen::generate(&dir).expect("the example is valid");
	assert_eq!(
		leptoskit_router_codegen::status(&leptoskit_router_codegen::output_path(&dir), &generated)
			.expect("status"),
		Status::Current,
		"run `leptoskit routes` in examples/file_routes"
	);
}
