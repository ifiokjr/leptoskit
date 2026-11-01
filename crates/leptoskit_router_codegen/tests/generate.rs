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
			"#[derive(Route, Clone, Debug, PartialEq)]\npub struct Today;\npub fn page(route: \
			 &Today) {}",
		),
		(
			"_layout.rs",
			"pub fn layout(children: Children, route: &Route) {}",
		),
		("+html.rs", "pub fn html() {}"),
		(
			"+not-found.rs",
			"pub fn load() {}\npub fn page(route: &NotFound, navigation: &Navigation) {}",
		),
		(
			"welcome.rs",
			"#[derive(Route, Clone, Debug, PartialEq)]\npub struct Welcome;\npub fn page(route: \
			 Welcome) {}",
		),
		(".editor-backup.rs", "not Rust at all {"),
		(
			"notes/index.rs",
			"pub fn load() {}\npub fn page(navigation: Navigation) {}",
		),
		(
			"notes/_layout.rs",
			"#[component]\npub fn Layout(navigation: &Navigation, route: Route, children: \
			 Children) {}",
		),
		("notes/_card.rs", "pub fn card() {}"),
		(
			"notes/[note].rs",
			"#[derive(Route, Clone, Debug, PartialEq)]\npub struct Note {\n\tpub note: \
			 u64,\n\tpub highlight: Option<u32>,\n}\n\n#[component]\npub fn Page(route: Note, \
			 navigation: &Navigation) {}\n\n#[component]\npub fn Loading() {}",
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
				route.file.clone(),
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
				"/welcome",
				"Welcome",
				"welcome.rs".to_owned(),
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
		("home.ios.rs", PAGE),
		("[self].rs", PAGE),
		("+layout.rs", PAGE),
		("news.rs", PAGE),
		("news/index.rs", PAGE),
		("broken.rs", "pub fn page( {"),
		// Exports in the wrong kind of file.
		(
			"misplaced.rs",
			"pub fn page() {}\npub fn layout() {}\npub fn navigator() {}",
		),
		(
			"(shop)/deep/_layout.rs",
			"#[derive(Route)]\npub struct Shop;\npub fn prerender() {}\n#[component]\npub fn \
			 Html() {}",
		),
		("(shop)/deep/+html.rs", "pub fn page() {}"),
		// Not-found pages under a parameter, with a struct, or a navigator.
		("teams/[team]/+not-found.rs", PAGE),
		(
			"help/+not-found.rs",
			"#[derive(Route)]\npub struct Missing;\npub fn navigator() {}\npub fn page() {}",
		),
		// Repeated parameters, a struct called `Route`, wrong field kinds.
		(
			"[id]/[id].rs",
			"#[derive(Route)]\npub struct Twice {\n\tpub id: u64,\n}\npub fn page() {}",
		),
		(
			"me.rs",
			"#[derive(Route)]\npub struct Route;\npub fn page() {}",
		),
		(
			"[[lang]]/about.rs",
			"#[derive(Route)]\npub struct About {\n\tpub lang: String,\n}\npub fn page() {}",
		),
		(
			"docs/[...path].rs",
			"#[derive(Route)]\npub struct Docs {\n\tpub path: String,\n}\npub fn page() {}",
		),
		// Pages and layouts ask only for props the router passes.
		("profile.rs", "pub fn page(user: &User) {}"),
		("plain.rs", "pub fn page(route: &Plain) {}"),
		(
			"(admin)/_layout.rs",
			"pub fn layout(children: &Children, theme: Theme) {}",
		),
		("(admin)/panel.rs", PAGE),
		(
			"nav.rs",
			"#[derive(Route)]\npub struct Navigation;\npub fn page() {}",
		),
		// A layout file without its layout, or with a private one.
		("account/_layout.rs", "pub fn loading() {}"),
		("account/index.rs", PAGE),
		("settings/_layout.rs", "fn layout(children: Children) {}"),
		("settings/index.rs", "fn page() {}"),
	]);
	let diagnostics = leptoskit_router_codegen::generate(&tree.dir()).expect_err("broken tree");
	let lines: Vec<String> = diagnostics
		.0
		.iter()
		.map(|diagnostic| diagnostic.relative_to(tree.project.path()))
		.collect();
	insta::assert_snapshot!(lines.join("\n"));
	// Display shows the full path, with the line when there is one.
	let display = diagnostics.to_string();
	assert!(
		display.contains("about.rs:1: `Page` is capitalised"),
		"{display}"
	);
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

	// rustfmt's braces around a closure body and a match arm.
	let braced = reformatted
		.replacen(
			"|_, _| ::core::option::Option::Some(Route::Notes)",
			"|_, _| { ::core::option::Option::Some(Route::Notes) }",
			1,
		)
		.replacen(
			"Self::Notes => PATTERNS[2].static_href(),",
			"Self::Notes => { PATTERNS[2].static_href() }",
			1,
		);
	assert_ne!(braced, reformatted, "the braces went in");
	std::fs::write(&output, &braced).expect("brace");
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

#[test]
fn names_that_are_not_identifiers_become_valid_rust() {
	let tree = Tree::new(&[
		("index.rs", PAGE),
		("404.rs", PAGE),
		("self.rs", PAGE),
		("gen.rs", PAGE),
		("2fa/index.rs", PAGE),
		(
			"crate/[type].rs",
			"#[derive(Route)]\npub struct Kind {\n\tpub r#type: String,\n}\npub fn page() {}",
		),
	]);
	let generated = leptoskit_router_codegen::generate(&tree.dir()).expect("every name is Rust");
	syn::parse_file(&generated.source).expect("routes.rs parses");
	for module in [
		"pub mod _404;",
		"pub mod self_;",
		"pub mod gen_;",
		"pub mod _2fa {",
		"pub mod crate_ {",
		"pub mod type_;",
	] {
		assert!(
			generated.source.contains(module),
			"{module}\n{}",
			generated.source
		);
	}
	let names: Vec<_> = generated
		.routes
		.iter()
		.map(|route| route.name.as_str())
		.collect();
	assert_eq!(names, ["Index", "_2fa", "_404", "Gen", "Self_", "Kind"]);
}

#[test]
fn private_helpers_with_export_names_are_ignored() {
	let tree = Tree::new(&[(
		"index.rs",
		"fn error() {}\nfn load() {}\nfn head() {}\nfn loading() {}\nfn layout() {}\npub fn \
		 page() {}",
	)]);
	leptoskit_router_codegen::generate(&tree.dir()).expect("the helpers are the file's own");
}

#[test]
fn routes_that_share_any_shape_are_reported_once() {
	// Both match `/x` and `/x/y`, once their optional segments are expanded.
	let tree = Tree::new(&[
		(
			"[[a]]/[b].rs",
			"#[derive(Route)]\npub struct A {\n\tpub a: Option<String>,\n\tpub b: String,\n}\npub \
			 fn page() {}",
		),
		(
			"[c]/[[d]].rs",
			"#[derive(Route)]\npub struct C {\n\tpub c: String,\n\tpub d: Option<String>,\n}\npub \
			 fn page() {}",
		),
	]);
	let diagnostics = leptoskit_router_codegen::generate(&tree.dir()).expect_err("they compete");
	let lines: Vec<String> = diagnostics
		.0
		.iter()
		.map(|diagnostic| diagnostic.relative_to(&tree.dir()))
		.collect();
	assert_eq!(
		lines,
		["[c]/[[d]].rs: this route and `[[a]]/[b].rs` match the same URLs, `/:b`"]
	);
}

#[test]
fn a_route_has_at_most_eight_optional_segments() {
	let names = ["a", "b", "c", "d", "e", "f", "g", "h", "i"];
	let path = names.map(|name| format!("[[{name}]]/")).concat() + "x.rs";
	let fields = names
		.map(|name| format!("\tpub {name}: Option<u8>,\n"))
		.concat();
	let source = format!("#[derive(Route)]\npub struct Deep {{\n{fields}}}\npub fn page() {{}}");
	let tree = Tree::new(&[(path.as_str(), source.as_str())]);
	let error = leptoskit_router_codegen::generate(&tree.dir()).expect_err("too many");
	assert!(
		error
			.to_string()
			.contains("this route has 9 optional segments"),
		"{error}"
	);
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

#[test]
fn reports_files_it_cannot_read_or_write() {
	let tree = Tree::new(&[("index.rs", PAGE)]);
	let generated = leptoskit_router_codegen::generate(&tree.dir()).expect("valid tree");

	// A directory where the module should be cannot be read as one.
	let output = tree.output();
	std::fs::create_dir(&output).expect("a directory in the way");
	let error = leptoskit_router_codegen::status(&output, &generated).expect_err("unreadable");
	assert!(matches!(error, WriteError::Io { .. }), "{error:?}");

	let missing = tree.project.path().join("missing/routes.rs");
	let error = leptoskit_router_codegen::write(&missing, &generated).expect_err("no directory");
	assert!(matches!(error, WriteError::Io { .. }), "{error:?}");
	assert!(error.to_string().contains("could not access"), "{error}");
}

#[test]
fn a_directory_without_a_name_is_reported() {
	let error = leptoskit_router_codegen::generate(Path::new("/")).expect_err("no name");
	assert!(error.to_string().contains("needs a name"), "{error}");
}

#[cfg(unix)]
#[test]
fn unreadable_files_and_directories_are_reported() {
	use std::os::unix::fs::PermissionsExt;

	let tree = Tree::new(&[
		("index.rs", PAGE),
		("locked.rs", PAGE),
		("vault/index.rs", PAGE),
	]);
	let locked = tree.dir().join("locked.rs");
	let vault = tree.dir().join("vault");
	let lock = |path: &Path, mode: u32| {
		std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).expect("chmod");
	};
	lock(&locked, 0o000);
	lock(&vault, 0o000);
	let result = leptoskit_router_codegen::generate(&tree.dir());
	lock(&locked, 0o644);

	lock(&vault, 0o755);
	let messages: Vec<String> = result
		.expect_err("unreadable")
		.0
		.iter()
		.map(|diagnostic| diagnostic.relative_to(&tree.dir()))
		.collect();
	assert!(
		messages
			.iter()
			.any(|message| message.starts_with("locked.rs: could not read")),
		"{messages:?}"
	);
	assert!(
		messages
			.iter()
			.any(|message| message.starts_with("vault: could not read")),
		"{messages:?}"
	);
}

#[cfg(target_os = "linux")]
#[test]
fn names_that_are_not_utf8_are_reported() {
	use std::ffi::OsStr;
	use std::os::unix::ffi::OsStrExt;

	let tree = Tree::new(&[("index.rs", PAGE)]);
	std::fs::write(tree.dir().join(OsStr::from_bytes(b"caf\xe9.rs")), PAGE).expect("latin-1 name");
	let error = leptoskit_router_codegen::generate(&tree.dir()).expect_err("not UTF-8");
	assert!(error.to_string().contains("must be UTF-8"), "{error}");
}
