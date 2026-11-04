//! The public site's content uses the same component builders as the apps.
use leptoskit::native::Role;
use leptoskit::prelude::*;
use reactive_graph::prelude::*;

pub const REPOSITORY: &str = "https://github.com/ifiokjr/leptoskit";

fn text(id: &str, content: &str) -> Element {
	Text(content).id(id)
}

fn link(id: &str, label: &str, href: &str) -> Element {
	Pressable(label).id(id).href(href).child(Text(label))
}

fn intro(id: &str, eyebrow: &str, title: &str, description: &str) -> Element {
	View()
		.id(id)
		.class("gap-5")
		.child(text(&format!("{id}-eyebrow"), eyebrow))
		.child(Heading(2, title).id(format!("{id}-title")))
		.child(text(&format!("{id}-description"), description))
}

pub fn screen() -> Element {
	View()
		.id("site")
		.child(link("skip", "Skip to content", "#main"))
		.child(header())
		.child(
			View()
				.role(Role::Main)
				.id("main")
				.child(hero())
				.child(platforms())
				.child(studio())
				.child(workflow())
				.child(capabilities())
				.child(cloud())
				.child(start()),
		)
		.child(footer())
}

fn header() -> Element {
	View()
		.role(Role::Banner)
		.id("header")
		.child(
			Pressable("LeptosKit home")
				.href("/")
				.id("brand")
				.child(
					Image("/docs/assets/mark.svg")
						.class("w-9 h-9")
						.aria_hidden(true),
				)
				.child(Text("leptoskit").class("font-semibold")),
		)
		.child(
			View()
				.role(Role::Navigation)
				.accessibility_label("Main navigation")
				.id("nav")
				.child(link("nav-toolkit", "The toolkit", "#toolkit"))
				.child(link("nav-cloud", "Your cloud", "#cloud"))
				.child(link("nav-docs", "Docs ↗", "/docs/")),
		)
		.child(link("nav-console", "Open console ↗", "/console/"))
}

fn hero() -> Element {
	View()
		.id("hero")
		.child(
			View()
				.id("hero-copy")
				.child(text(
					"hero-eyebrow",
					"A LITTLE LESS GLUE. A LOT MORE POSSIBILITY.",
				))
				.child(Heading(1, "One tree.\nEvery screen.\nYour cloud.").id("hero-title"))
				.child(text(
					"hero-description",
					"Build your interface, backend, and deployment in one Rust codebase. From the \
					 first idea to the app in someone's pocket — keep it yours.",
				))
				.child(
					View()
						.id("hero-actions")
						.child(link(
							"hero-primary",
							"Start building  ↗",
							"/docs/guide/01-installation.html",
						))
						.child(link("hero-secondary", "Explore the toolkit  ↓", "#toolkit")),
				)
				.child(text(
					"hero-note",
					"Open source. Real native widgets. No mandatory cloud.",
				)),
		)
		.child(
			View()
				.id("hero-art")
				.aria_hidden(true)
				.child(View().id("art-grid"))
				.child(
					View()
						.id("art-l")
						.child(View().id("art-tall"))
						.child(View().id("art-square")),
				)
				.child(View().id("art-mint"))
				.child(text("art-label-one", "ONE COMPONENT TREE"))
				.child(text("art-label-two", "READY FOR THE NEXT IDEA"))
				.child(text(
					"art-code",
					"View()\n  .class(\"gap-4 p-6\")\n  .child(Text(\"Hello, everywhere.\"))",
				)),
		)
}

fn platforms() -> Element {
	View()
		.id("platforms")
		.child(text("platforms-label", "AT HOME ON"))
		.children(
			["Web", "iOS", "Android", "macOS", "Windows", "Linux"]
				.into_iter()
				.map(|name| Text(name)),
		)
		.child(link(
			"platforms-more",
			"Platform details ↗",
			"/docs/platforms/index.html",
		))
}

fn studio() -> Element {
	View()
		.id("toolkit")
		.child(intro(
			"studio-intro",
			"01 / THE INTERFACE",
			"Same tree.\nDifferent worlds.",
			"Leptos in the browser. Platform widgets in native apps. Tailwind classes travel with \
			 the component tree.",
		))
		.child(
			View()
				.id("studio-board")
				.child(
					View()
						.id("source-panel")
						.child(text("source-caption", "ONE RUST COMPONENT"))
						.child(text(
							"source-code",
							"let count = RwSignal::new(0);\n\nView()\n  .class(\"gap-4 rounded-xl \
							 p-6\")\n  .child(Heading(2, \"Little wins\"))\n  .child(Text(move \
							 ||\n    format!(\"{} ideas shipped\", count.get())\n  ))\n  \
							 .child(\n    Button(\"One more idea\")\n      .on_press(move || \
							 count.update(|n| *n += 1))\n  )",
						))
						.child(link(
							"source-guide",
							"Learn the component model ↗",
							"/docs/guide/07-shared-components.html",
						)),
				)
				.child(
					View()
						.id("preview-panel")
						.child(text(
							"preview-caption",
							"A REAL LEPTOSKIT COMPONENT · TRY IT",
						))
						.child(View().id("live-demo").child(demo(RwSignal::new(0))))
						.child(text(
							"preview-note",
							"This interactive browser example changes local state. Native builds \
							 use the same builders with their platform's widgets.",
						)),
				),
		)
}

/// A small, working example mounted by the public page's browser client.
pub fn demo(count: RwSignal<u32>) -> Element {
	View()
		.id("demo-card")
		.class("gap-4 rounded-2xl p-6")
		.child(crate::motion::mark().id("demo-mark"))
		.child(Heading(3, "Little wins").id("demo-title"))
		.child(Text(move || format!("{} ideas shipped", count.get())).id("demo-count"))
		.child(Text("Big things start with one small, well-made thing.").id("demo-description"))
		.child(
			Button("One more idea  +")
				.id("demo-increment")
				.test_id("demo-increment")
				.on_press(move || count.update(|value| *value = value.saturating_add(1))),
		)
}

fn workflow() -> Element {
	let mut steps = View().id("workflow-steps");
	for (number, title, description, href) in [
		(
			"01",
			"Build something real.",
			"Components, typed RPC, database schemas, and sign-in. Rust checks how the pieces fit.",
			"/docs/guide/07-shared-components.html",
		),
		(
			"02",
			"Give it a place to live.",
			"Describe compute, storage, live services, and backups in TOML. Review the OpenTofu \
			 plan before applying it.",
			"/docs/guide/24-deploying.html",
		),
		(
			"03",
			"Keep making it better.",
			"Roll out container releases and signed native code pushes. Keep migration history \
			 and a route back.",
			"/docs/guide/05-code-push.html",
		),
	] {
		steps = steps.child(
			View()
				.id(format!("step-{number}"))
				.class("gap-4")
				.child(Text(number).class("text-sm"))
				.child(Heading(3, title))
				.child(Text(description))
				.child(link(
					&format!("step-{number}-link"),
					"See how it works ↗",
					href,
				)),
		);
	}
	View()
		.id("workflow")
		.child(intro(
			"workflow-intro",
			"02 / THE WHOLE JOURNEY",
			"From a blank file\nto a living app.",
			"A toolkit for the parts you want to build — and the parts you shouldn't have to \
			 assemble again.",
		))
		.child(steps)
}

fn capabilities() -> Element {
	let mut grid = View().id("capability-grid");
	for (id, number, title, description, href) in [
		(
			"data",
			"{ }",
			"Data with a point of view.",
			"Typed schemas, migrations, row policies, and client-safe projections. Give every \
			 caller exactly the view they need.",
			"/docs/guide/22-database.html",
		),
		(
			"live",
			"↔",
			"A little more alive.",
			"Typed requests, live queries, and relay-backed services that can stay connected. \
			 Build collaboration around real application state.",
			"/docs/guide/24-real-time.html",
		),
		(
			"native",
			"⌘",
			"Native means native.",
			"System widgets, runtime plugins, secure storage, and signed updates. A Wasm \
			 interface inside an installed native host.",
			"/docs/platforms/index.html",
		),
		(
			"identity",
			"✳",
			"Identity, built in.",
			"Sign-in, passkeys, sessions, and an identity provider. Keep authorization close to \
			 the data and operations it protects.",
			"/docs/guide/23-sign-in.html",
		),
		(
			"motion",
			"◌",
			"Make it feel like yours.",
			"Vector graphics, Rive animation, themes, and localization. Give a shared interface \
			 its own character.",
			"/docs/guide/20-rive.html",
		),
		(
			"confidence",
			"✓",
			"Room to move with confidence.",
			"Accessible controls, component tests, device journeys, and agent-readable analytics. \
			 See what the app is actually doing.",
			"/docs/guide/27-analytics.html",
		),
	] {
		grid = grid.child(
			View()
				.id(format!("feature-{id}"))
				.class("gap-4")
				.child(
					Text(number)
						.id(format!("feature-{id}-icon"))
						.aria_hidden(true),
				)
				.child(Heading(3, title))
				.child(Text(description))
				.child(link(
					&format!("feature-{id}-link"),
					"Read the guide ↗",
					href,
				)),
		);
	}
	View()
		.id("capabilities")
		.child(intro(
			"capabilities-intro",
			"03 / GOOD PIECES, ALREADY CONNECTED",
			"Bring the idea.\nThe foundations are here.",
			"Explore the toolkit, choose the features your app needs, and leave the rest out.",
		))
		.child(grid)
}

fn cloud() -> Element {
	View()
		.id("cloud")
		.child(intro(
			"cloud-intro",
			"04 / YOUR NEXT PLACE TO LAUNCH",
			"Your app.\nYour choice of cloud.",
			"Run the deployment engine yourself, or connect a dedicated worker to the hosted \
			 console. Your infrastructure stays explicit.",
		))
		.child(
			View()
				.id("ownership")
				.child(
					View()
						.id("ownership-customer")
						.class("gap-5")
						.child(text("ownership-customer-label", "BRING YOUR OWN"))
						.child(Heading(3, "Your account.\nYour infrastructure."))
						.child(Text(
							"Connect your cloud credentials to a dedicated worker. Use the \
							 console for reviewed plans, releases, encrypted state, and \
							 deployment history.",
						))
						.child(link(
							"ownership-customer-link",
							"Connect your infrastructure ↗",
							"/docs/guide/29-hosted-deployments.html",
						)),
				)
				.child(
					View()
						.id("ownership-managed")
						.class("gap-5")
						.child(text("ownership-managed-label", "LET US HELP YOU LAUNCH"))
						.child(Heading(3, "A dedicated place\nto get started."))
						.child(Text(
							"LeptosKit-managed DigitalOcean capacity starts with an \
							 operator-approved quote and an isolated worker. Cloud usage is \
							 separate from the management subscription.",
						))
						.child(link(
							"ownership-managed-link",
							"Read about the hosted pilot ↗",
							"/docs/guide/29-hosted-deployments.html",
						)),
				),
		)
		.child(text(
			"cloud-note",
			"Hosted access is an operator-enrolled pilot. Contact your service operator for \
			 availability, pricing, and a reviewed infrastructure quote.",
		))
}

fn start() -> Element {
	View()
		.id("start")
		.child(text("start-eyebrow", "WHAT WILL YOU MAKE?"))
		.child(Heading(2, "One good idea.\nEverywhere it belongs.").id("start-title"))
		.child(
			View()
				.id("start-actions")
				.child(link(
					"start-primary",
					"Build your first app  ↗",
					"/docs/guide/01-installation.html",
				))
				.child(link("start-source", "Explore the source ↗", REPOSITORY)),
		)
}

fn footer() -> Element {
	View()
		.role(Role::ContentInfo)
		.id("footer")
		.child(text("footer-brand", "leptoskit"))
		.child(Text("Made of Rust. Open by design."))
		.child(
			View()
				.role(Role::Navigation)
				.accessibility_label("Resources")
				.id("footer-links")
				.child(link("footer-docs", "Documentation", "/docs/"))
				.child(link("footer-examples", "Examples", "/docs/examples.html"))
				.child(link("footer-github", "GitHub", REPOSITORY))
				.child(link("footer-console", "Console", "/console/")),
		)
}
