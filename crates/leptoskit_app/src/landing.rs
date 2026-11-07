//! The public site's content uses the same component builders as the apps,
//! and its motion uses the same declarative [`Motion`] they do: entrances
//! when the page loads, and choreography that follows scrolling.
use std::time::Duration;

use leptoskit::native::Role;
use leptoskit::prelude::*;
use reactive_graph::prelude::*;

pub const REPOSITORY: &str = "https://github.com/leptoskit/leptoskit";

/// Which site renders. Both share the landing page's design, sections, and
/// live component example; only console entry points and hosted-service copy
/// differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
	/// The public website: documentation and source links only. It offers no
	/// console, account, or checkout action because the public artifact has
	/// no backend to sign into.
	Public,
	/// The operator-hosted pilot, which also serves the management console
	/// and its API from the same origin.
	Hosted,
}

/// How a platform's window looks, for its drawing on the stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Form {
	Browser,
	Phone,
	Window,
	Terminal,
}

/// One platform the same component tree renders on, and the widgets it
/// renders as: the container, the heading, the text, and the button.
struct Platform {
	id: &'static str,
	name: &'static str,
	toolkit: &'static str,
	form: Form,
	widgets: [&'static str; 4],
}

/// Every platform, in the order the stage visits them. The widget names are
/// what each renderer creates for `View`, `Heading`, `Text`, and `Button`.
const PLATFORMS: [Platform; 8] = [
	Platform {
		id: "web",
		name: "Web",
		toolkit: "Leptos and the DOM",
		form: Form::Browser,
		widgets: ["<div>", "<h3>", "<span>", "<button>"],
	},
	Platform {
		id: "ios",
		name: "iOS",
		toolkit: "UIKit",
		form: Form::Phone,
		widgets: ["UIView", "UILabel", "UILabel", "UIButton"],
	},
	Platform {
		id: "android",
		name: "Android",
		toolkit: "Android views",
		form: Form::Phone,
		widgets: ["FrameLayout", "TextView", "TextView", "Button"],
	},
	Platform {
		id: "macos",
		name: "macOS",
		toolkit: "AppKit",
		form: Form::Window,
		widgets: ["NSView", "NSTextField", "NSTextField", "NSButton"],
	},
	Platform {
		id: "windows",
		name: "Windows",
		toolkit: "Win32",
		form: Form::Window,
		widgets: ["child window", "STATIC", "STATIC", "BUTTON"],
	},
	Platform {
		id: "linux",
		name: "Linux",
		toolkit: "GTK 4",
		form: Form::Window,
		widgets: ["GtkWidget", "GtkLabel", "GtkLabel", "GtkButton"],
	},
	Platform {
		id: "terminal",
		name: "Terminal",
		toolkit: "terminal cells",
		form: Form::Terminal,
		widgets: [
			"a box of cells",
			"a line of text",
			"a line of text",
			"[ One more idea ]",
		],
	},
	Platform {
		id: "harmonyos",
		name: "HarmonyOS",
		toolkit: "ArkUI, experimental",
		form: Form::Phone,
		widgets: ["Stack", "Text", "Text", "Button"],
	},
];

/// The stage's scroll timeline, which every part of the morph follows.
const STAGE: &str = "stage";

/// How much of the stage's scroll each change of platform takes.
const CHANGE: f32 = 0.04;

fn ms(millis: u64) -> Duration {
	Duration::from_millis(millis)
}

/// Where on the stage's timeline the change to platform `index` is
/// centered. The first platform is there from the start.
fn change_at(index: usize) -> f32 {
	index as f32 / PLATFORMS.len() as f32
}

/// A pose `index`'s part of the stage takes while another platform shows,
/// before (`ahead`) or after it.
fn offstage(ahead: bool) -> Pose {
	Pose::new().opacity(0.0).y(if ahead { 10.0 } else { -10.0 })
}

/// Shows something only while platform `index` is on stage: it arrives
/// with that platform's change and leaves with the next.
fn while_on_stage(index: usize) -> Motion {
	let last = PLATFORMS.len() - 1;
	let arrive = change_at(index);
	let leave = change_at(index + 1);
	let mut poses = Vec::with_capacity(6);

	if index > 0 {
		poses.extend([
			(0.0, offstage(true)),
			(arrive - CHANGE / 2.0, offstage(true)),
			(arrive + CHANGE / 2.0, Pose::REST),
		]);
	} else {
		poses.push((0.0, Pose::REST));
	}

	if index < last {
		poses.extend([
			(leave - CHANGE / 2.0, Pose::REST),
			(leave + CHANGE / 2.0, offstage(false)),
			(1.0, offstage(false)),
		]);
	} else {
		poses.push((1.0, Pose::REST));
	}

	Motion::keyframes(poses)
		.easing(Easing::Linear)
		.follow(STAGE, ViewRange::contain())
}

/// The ink under a platform's name: drawn in from the left when the
/// platform arrives, and wiped away to the right when it leaves.
fn underline_on_stage(index: usize) -> Motion {
	let last = PLATFORMS.len() - 1;
	let arrive = change_at(index);
	let leave = change_at(index + 1);
	let hidden = Pose::new().clip(Inset::from_left(0.0));
	let gone = Pose::new().clip(Inset::from_right(0.0));
	let shown = Pose::new().clip(Inset::NONE);
	let mut poses = Vec::with_capacity(6);

	if index > 0 {
		poses.extend([
			(0.0, hidden),
			(arrive - CHANGE / 2.0, hidden),
			(arrive + CHANGE / 2.0, shown),
		]);
	} else {
		poses.push((0.0, shown));
	}

	if index < last {
		poses.extend([
			(leave - CHANGE / 2.0, shown),
			(leave + CHANGE / 2.0, gone),
			(1.0, gone),
		]);
	} else {
		poses.push((1.0, shown));
	}

	Motion::keyframes(poses)
		.easing(Easing::Linear)
		.follow(STAGE, ViewRange::contain())
}

/// A platform's whole scene is drawn over the one before it, from the left,
/// as its change begins.
fn wipe_on_stage(index: usize) -> Motion {
	let at = change_at(index);
	let hidden = Pose::new().clip(Inset::from_left(0.0));
	Motion::keyframes([
		(0.0, hidden),
		(at - CHANGE / 2.0, hidden),
		(at + CHANGE / 2.0, Pose::new().clip(Inset::NONE)),
		(1.0, Pose::new().clip(Inset::NONE)),
	])
	.easing(Easing::InOut)
	.follow(STAGE, ViewRange::contain())
}

/// A part of a scene that settles into place just behind its scene's wipe,
/// `lag` of a change later.
fn settle_on_stage(index: usize, from: Pose, lag: f32) -> Motion {
	let at = change_at(index) - CHANGE / 2.0 + lag * CHANGE;
	Motion::keyframes([(0.0, from), (at, from), (at + CHANGE, Pose::REST)])
		.follow(STAGE, ViewRange::contain())
}

/// Rises into place as the page loads. Only the position moves: the copy is
/// readable from the first paint, so the largest text never waits on motion.
fn rise(delay: u64) -> Motion {
	Motion::enter(Pose::new().y(22.0))
		.spring(Spring::GENTLE)
		.delay(ms(delay))
}

/// Moves into place as it scrolls into view; `column` starts a little later
/// for things side by side.
fn arrive(from: Pose, column: usize) -> Motion {
	let start = 0.08 * column as f32;
	Motion::enter(from).in_view(ViewRange::new(
		ViewPhase::Entry,
		start,
		ViewPhase::Entry,
		(0.7 + start).min(1.0),
	))
}

fn text(id: &str, content: &str) -> Element {
	Text(content).id(id)
}

fn link(id: &str, label: &str, href: &str) -> Element {
	Pressable(label).id(id).href(href).child(Text(label))
}

/// A section's eyebrow, title, and description. Each `\n`-separated line of
/// the title is a block of its own, so a line too long for the column wraps
/// evenly rather than leaving one word behind. Its id keeps it an element of
/// its own rather than text joined into the heading's, and the space after
/// it keeps the lines apart for assistive technology.
fn intro(id: &str, eyebrow: &str, title: &str, description: &str) -> Element {
	View()
		.id(id)
		.class("gap-5")
		.child(
			text(&format!("{id}-eyebrow"), eyebrow)
				.motion(arrive(Pose::new().x(-16.0).clip(Inset::from_left(0.0)), 0)),
		)
		.child(
			Heading(2, "")
				.id(format!("{id}-title"))
				.children(
					title.split('\n').enumerate().map(|(index, line)| {
						text(&format!("{id}-title-{index}"), &format!("{line} "))
					}),
				)
				.motion(arrive(Pose::new().y(48.0).clip(Inset::from_bottom(0.0)), 1)),
		)
		.child(
			text(&format!("{id}-description"), description).motion(arrive(Pose::new().y(28.0), 2)),
		)
}

pub fn screen(mode: Mode) -> Element {
	View()
		.id("site")
		.child(link("skip", "Skip to content", "#main"))
		.child(header(mode))
		.child(
			View()
				.role(Role::Main)
				.id("main")
				.child(hero())
				.child(stage())
				.child(studio())
				.child(workflow())
				.child(capabilities())
				.child(cloud(mode))
				.child(start()),
		)
		.child(footer(mode))
}

fn header(mode: Mode) -> Element {
	View()
		.role(Role::Banner)
		.id("header")
		.child(
			Pressable("LeptosKit home")
				.href("/")
				.id("brand")
				.child(
					Image("/docs/assets/mark.svg")
						.id("brand-mark")
						.class("w-9 h-9")
						.aria_hidden(true)
						.motion(
							Motion::enter(Pose::new().rotate(-80.0).scale(0.4))
								.spring(Spring::BOUNCY)
								.delay(ms(120)),
						),
				)
				.child(Text("leptoskit").class("font-semibold")),
		)
		.child(
			View()
				.role(Role::Navigation)
				.accessibility_label("Main navigation")
				.id("nav")
				.child(link("nav-tree", "One tree", "#tree"))
				.child(link("nav-toolkit", "Try it", "#toolkit"))
				.child(link("nav-cloud", "Your cloud", "#cloud"))
				.child(link("nav-docs", "Docs ↗", "/docs/")),
		)
		.children(match mode {
			Mode::Hosted => Some(link("nav-console", "Open console ↗", "/console/")),
			Mode::Public => None,
		})
}

fn hero() -> Element {
	// The title is visible from the first paint: its lines only settle, so
	// the page's largest text never waits for motion.
	let settle = || {
		Motion::enter(Pose::new().y(18.0).rotate(-1.5))
			.spring(Spring::GENTLE)
			.delay(ms(40))
	};
	View()
		.id("hero")
		.child(
			View()
				.id("hero-copy")
				.child(
					text(
						"hero-eyebrow",
						"A LITTLE LESS GLUE. A LOT MORE POSSIBILITY.",
					)
					.motion(rise(0)),
				)
				.child(
					Heading(1, "")
						.id("hero-title")
						.stagger(ms(90))
						.child(text("hero-line-1", "One tree. ").motion(settle()))
						.child(text("hero-line-2", "Every screen. ").motion(settle()))
						.child(text("hero-line-3", "Your cloud.").motion(settle())),
				)
				.child(
					View().id("hero-underline").aria_hidden(true).motion(
						Motion::enter(Pose::new().clip(Inset::from_left(0.0)))
							.duration(ms(900))
							.delay(ms(380))
							.easing(Easing::InOut),
					),
				)
				.child(
					text(
						"hero-description",
						"Write your interface once, as one tree of components, and watch it \
						 become real native widgets on every screen — with the backend and the \
						 deployment in the same Rust codebase.",
					)
					.motion(rise(220)),
				)
				.child(
					View()
						.id("hero-actions")
						.motion(rise(320))
						.child(link(
							"hero-primary",
							"Start building  ↗",
							"/docs/guide/01-installation.html",
						))
						.child(link("hero-secondary", "Watch it render  ↓", "#tree")),
				)
				.child(
					text(
						"hero-note",
						"Open source. Real native widgets. No mandatory cloud.",
					)
					.motion(rise(420)),
				),
		)
		.child(fan())
}

/// One component tree in front, and the screens it becomes fanned out
/// behind it. Each card jumps to its platform on the stage; the stage's own
/// platform list is the keyboard's way there.
fn fan() -> Element {
	let swing = || {
		Motion::enter(Pose::new().rotate(0.0).scale(0.94))
			.spring(Spring::BOUNCY)
			.delay(ms(520))
	};
	View()
		.id("hero-fan")
		.aria_hidden(true)
		.child(
			View()
				.id("fan-cards")
				.stagger(ms(60))
				.children(PLATFORMS.iter().map(|platform| {
					Pressable(format!("See {} on the stage", platform.name))
						.id(format!("fan-{}", platform.id))
						.href(format!("#on-{}", platform.id))
						.tab_index(-1)
						.motion(swing())
						.child(device("fan", platform, None))
						.child(text(&format!("fan-{}-name", platform.id), platform.name))
				})),
		)
		.child(
			View()
				.id("fan-tree")
				.motion(
					Motion::enter(Pose::new().y(-70.0).rotate(-9.0))
						.spring(Spring::BOUNCY)
						.delay(ms(160)),
				)
				.child(text("fan-tree-label", "ONE COMPONENT TREE"))
				.child(text(
					"fan-tree-code",
					"View()\n  .child(Heading(3, \"Little wins\"))\n  .child(Text(shipped))\n  \
					 .child(Button(\"One more idea\"))",
				)),
		)
}

/// A drawing of the example component as `platform` shows it: its window
/// or device, and the heading, text, and button inside. On the stage, at
/// `turn`, the parts land one after another behind the scene's wipe, the
/// button last, with a bounce.
fn device(prefix: &str, platform: &Platform, turn: Option<usize>) -> Element {
	let id = format!("{prefix}-{}-device", platform.id);
	let part = |name: &str| format!("{id}-{name}");
	let chrome = View().id(part("chrome"));
	let chrome = match platform.form {
		Form::Browser => {
			chrome
				.child(View().id(part("dots")))
				.child(text(&part("address"), "leptoskit.dev"))
		}
		Form::Phone => {
			chrome
				.child(text(&part("time"), "9:41"))
				.child(View().id(part("camera")))
		}
		Form::Window => {
			chrome
				.child(View().id(part("controls")))
				.child(text(&part("caption"), "Little wins"))
		}
		Form::Terminal => chrome.child(text(&part("prompt"), "$ leptoskit dev --platform tui")),
	};
	let button = if platform.form == Form::Terminal {
		"[ One more idea ]"
	} else {
		"One more idea"
	};
	let lands = |element: Element, from: Pose, lag: f32, easing: Easing| {
		match turn {
			Some(turn) => element.motion(settle_on_stage(turn, from, lag).easing(easing)),
			None => element,
		}
	};
	View().id(id.clone()).child(chrome).child(
		View()
			.id(part("body"))
			.child(lands(
				text(&part("title"), "Little wins"),
				Pose::new().y(16.0).opacity(0.0),
				0.4,
				Easing::Out,
			))
			.child(lands(
				text(&part("count"), "3 ideas shipped"),
				Pose::new().y(16.0).opacity(0.0),
				0.55,
				Easing::Out,
			))
			.child(lands(
				View()
					.id(part("button"))
					.child(text(&part("label"), button)),
				Pose::new().scale(0.5).opacity(0.0),
				0.7,
				Easing::Spring(Spring::BOUNCY),
			)),
	)
}

/// The same component tree, redrawn by every platform's renderer as the
/// page scrolls past a stage. Without scroll timelines, or with reduced
/// motion, the stage lays every platform out side by side instead.
fn stage() -> Element {
	let mut stage = View().id("tree").timeline(STAGE);

	for platform in &PLATFORMS {
		stage = stage.child(View().id(format!("on-{}", platform.id)).aria_hidden(true));
	}

	stage.child(
		View()
			.id("stage")
			.child(intro(
				"stage-intro",
				"01 / ONE TREE, EVERY SCREEN",
				"Same tree.\nDifferent worlds.",
				"Scroll, and watch one component become each platform's own widgets. The code \
				 never changes; the renderer does.",
			))
			.child(widget_map())
			.child(scenes())
			.child(ticker())
			.child(
				View().id("stage-progress").aria_hidden(true).motion(
					Motion::keyframes([
						(0.0, Pose::new().clip(Inset::from_left(0.0))),
						(1.0, Pose::new().clip(Inset::NONE)),
					])
					.easing(Easing::Linear)
					.follow(STAGE, ViewRange::contain()),
				),
			),
	)
}

/// The component's tree beside what each platform makes of it, one row per
/// element. It draws what the scenes' captions say, so assistive technology
/// reads the captions instead.
fn widget_map() -> Element {
	let rows = [
		"View()",
		"  .child(Heading(3, \"Little wins\"))",
		"  .child(Text(shipped))",
		"  .child(Button(\"One more idea\"))",
	];
	let mut map = View().id("widget-map").aria_hidden(true).child(
		View()
			.id("map-head")
			.child(text("map-head-tree", "ONE RUST TREE"))
			.child(
				View()
					.id("map-head-platforms")
					.child(text("map-head-becomes", "BECOMES, ON"))
					.child(swapping("map-head", |platform| platform.name.to_owned())),
			),
	);

	for (column, code) in rows.into_iter().enumerate() {
		map = map.child(
			View()
				.id(format!("map-row-{column}"))
				.child(text(&format!("map-row-{column}-code"), code))
				.child(swapping(&format!("map-row-{column}"), |platform| {
					platform.widgets[column].to_owned()
				})),
		);
	}

	map
}

/// Every platform's version of one label, stacked in one place: each shows
/// only while its platform is on stage. Without motion they read as a list.
fn swapping(id: &str, label: impl Fn(&Platform) -> String) -> Element {
	View()
		.id(format!("{id}-swap"))
		.children(PLATFORMS.iter().enumerate().map(|(index, platform)| {
			Text(label(platform))
				.id(format!("{id}-{}", platform.id))
				.motion(while_on_stage(index))
		}))
}

/// Every platform's scene: its drawing of the component, and a caption
/// naming the toolkit and the widgets it makes. Each scene is drawn over the
/// one before as the stage scrolls; read in order, the captions say what the
/// stage shows.
fn scenes() -> Element {
	View()
		.id("scenes")
		.children(PLATFORMS.iter().enumerate().map(|(index, platform)| {
			let id = format!("scene-{}", platform.id);
			let scene = View()
				.id(id.clone())
				.child(
					device("stage", platform, Some(index))
						.aria_hidden(true)
						.motion(settle_on_stage(
							index,
							Pose::new().scale(0.92).rotate(-2.0),
							0.2,
						)),
				)
				.child(
					text(
						&format!("{id}-caption"),
						&format!("{} · {}", platform.name, platform.toolkit),
					)
					.motion(settle_on_stage(
						index,
						Pose::new().y(12.0).opacity(0.0),
						0.5,
					)),
				)
				.child(text(
					&format!("{id}-widgets"),
					&format!(
						"View, Heading, Text, and Button become {}.",
						platform.widgets.join(", ")
					),
				));

			if index > 0 {
				scene.motion(wipe_on_stage(index))
			} else {
				scene
			}
		}))
}

/// The platforms in order, each a jump to its place on the stage. The ink
/// under a name follows whichever platform is showing.
fn ticker() -> Element {
	View()
		.id("ticker")
		.role(Role::Navigation)
		.accessibility_label("Platforms on the stage")
		.children(PLATFORMS.iter().enumerate().map(|(index, platform)| {
			Pressable(platform.name)
				.id(format!("ticker-{}", platform.id))
				.href(format!("#on-{}", platform.id))
				.child(Text(platform.name))
				.child(
					View()
						.id(format!("ticker-{}-ink", platform.id))
						.aria_hidden(true)
						.motion(underline_on_stage(index)),
				)
		}))
		.child(link(
			"ticker-more",
			"Platform details ↗",
			"/docs/platforms/index.html",
		))
}

fn studio() -> Element {
	View()
		.id("toolkit")
		.child(intro(
			"studio-intro",
			"02 / NOW TRY THE REAL ONE",
			"One component.\nNo illustration.",
			"The card on the right is the real thing: leptoskit's web renderer running the code \
			 on the left. Press its button and three elements answer together.",
		))
		.child(
			View()
				.id("studio-board")
				.motion(arrive(Pose::new().y(80.0).scale(0.96), 0))
				.child(
					View()
						.id("source-panel")
						.child(text("source-caption", "ONE RUST COMPONENT"))
						.child(text(
							"source-code",
							"let count = RwSignal::new(0);\nlet shipped = \
							 count.get();\n\nView()\n  .child(Heading(3, \"Little wins\"))\n  \
							 .child(\n    Text(format!(\"{shipped} ideas shipped\"))\n      \
							 .motion(\n        Motion::enter(Pose::new().scale(1.35))\n          \
							 .spring(Spring::BOUNCY)\n          .key(shipped),\n      ),\n  )\n  \
							 .child(\n    Button(\"One more idea\")\n      .on_press(move || \
							 count.update(|n| *n += 1)),\n  )",
						))
						.child(link(
							"source-guide",
							"Learn the component model ↗",
							"/docs/guide/07-shared-components.html",
						))
						.child(link(
							"source-motion",
							"Learn about motion ↗",
							"/docs/guide/32-motion.html",
						)),
				)
				.child(
					View()
						.id("preview-panel")
						.child(text(
							"preview-caption",
							"A REAL LEPTOSKIT COMPONENT · TRY IT",
						))
						// The static preview waits for the Wasm event handlers.
						.child(
							View()
								.id("live-demo")
								.child(demo(RwSignal::new(0)).enabled(false)),
						)
						.child(text(
							"preview-note",
							"This interactive browser example changes local state. Native builds \
							 use the same builders with their platform's widgets.",
						)),
				),
		)
}

/// A small, working example mounted by the public page's browser client.
///
/// Each press answers three ways at once: the count pops, a `+1` floats
/// away, and the mark wiggles. Their motions carry the count as a replay
/// key, so every new count plays them again.
pub fn demo(count: RwSignal<u32>) -> Element {
	let shipped = count.get();
	let key = u64::from(shipped);
	let pressed = shipped > 0;

	let mut mark = crate::motion::mark().id("demo-mark");
	let mut tally = Text(format!("{shipped} ideas shipped")).id("demo-count");

	if pressed {
		mark = mark.motion(
			Motion::keyframes([
				(0.0, Pose::new().rotate(0.0)),
				(0.2, Pose::new().rotate(-14.0)),
				(0.45, Pose::new().rotate(10.0)),
				(0.7, Pose::new().rotate(-5.0)),
				(1.0, Pose::new().rotate(0.0)),
			])
			.duration(ms(700))
			.easing(Easing::InOut)
			.key(key),
		);
		tally = tally.motion(
			Motion::enter(Pose::new().scale(1.35))
				.spring(Spring::BOUNCY)
				.key(key),
		);
	}

	View()
		.id("demo-card")
		.class("gap-4 rounded-2xl p-6")
		.child(mark)
		.child(Heading(3, "Little wins").id("demo-title"))
		.child(
			View()
				.id("demo-tally")
				.child(tally)
				.children(pressed.then(|| {
					Text("+1").id("demo-plus").aria_hidden(true).motion(
						Motion::keyframes([
							(0.0, Pose::new().opacity(0.0).y(6.0).scale(0.7)),
							(0.25, Pose::new().opacity(1.0).y(-8.0).scale(1.0)),
							(1.0, Pose::new().opacity(0.0).y(-34.0).scale(1.0)),
						])
						.duration(ms(900))
						.key(key),
					)
				})),
		)
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
	for (column, (number, title, description, href)) in [
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
	]
	.into_iter()
	.enumerate()
	{
		steps = steps.child(
			View()
				.id(format!("step-{number}"))
				.class("gap-4")
				.child(
					View()
						.id(format!("step-{number}-rule"))
						.aria_hidden(true)
						.motion(arrive(Pose::new().clip(Inset::from_left(0.0)), column)),
				)
				.child(
					Text(number)
						.id(format!("step-{number}-number"))
						.motion(arrive(
							Pose::new().y(20.0).clip(Inset::from_left(0.0)),
							column,
						)),
				)
				.child(
					Heading(3, title)
						.id(format!("step-{number}-title"))
						.motion(arrive(Pose::new().y(32.0), column + 1)),
				)
				.child(
					Text(description)
						.id(format!("step-{number}-description"))
						.motion(arrive(Pose::new().y(32.0), column + 2)),
				)
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
			"03 / THE WHOLE JOURNEY",
			"From a blank file\nto a living app.",
			"A toolkit for the parts you want to build — and the parts you shouldn't have to \
			 assemble again.",
		))
		.child(steps)
}

fn capabilities() -> Element {
	let mut grid = View().id("capability-grid");
	for (index, (id, number, title, description, href)) in [
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
			"Make it move like yours.",
			"Springs, entrances, and scroll-driven choreography as typed builders, plus Rive, \
			 vector graphics, themes, and localization.",
			"/docs/guide/32-motion.html",
		),
		(
			"confidence",
			"✓",
			"Room to move with confidence.",
			"Accessible controls, component tests, device journeys, and agent-readable analytics. \
			 See what the app is actually doing.",
			"/docs/guide/27-analytics.html",
		),
	]
	.into_iter()
	.enumerate()
	{
		// Cards land like paper dropped on a desk, each at its own slight
		// angle, a column at a time.
		let tilt = [-3.0, 2.5, -2.0, 3.0, -2.5, 2.0][index];
		grid = grid.child(
			View()
				.id(format!("feature-{id}"))
				.class("gap-4")
				.motion(arrive(Pose::new().y(64.0).rotate(tilt), index % 3))
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
			"04 / GOOD PIECES, ALREADY CONNECTED",
			"Bring the idea.\nThe foundations are here.",
			"Explore the toolkit, choose the features your app needs, and leave the rest out.",
		))
		.child(grid)
}

fn cloud(mode: Mode) -> Element {
	let (description, customer, managed) = match mode {
		Mode::Public => {
			(
				"Deploy from your own machine or pipeline with the CLI. The engine plans, \
				 applies, and rolls back infrastructure described in leptoskit.toml. Manage \
				 provider credentials in your own environment.",
				(
					"The CLI plans and applies real infrastructure.",
					"Describe compute, storage, live services, and backups in TOML. Review the \
					 OpenTofu plan before anything runs, and keep the deployment history in your \
					 own hands.",
					"Read about deploying ↗",
					"/docs/guide/24-deploying.html",
				),
				(
					"Ship, then keep building.",
					"Publish container releases and signed native code pushes from your own \
					 workflow. Keep migration history and a route back when a release needs to be \
					 reversed.",
					"Explore code push ↗",
					"/docs/guide/05-code-push.html",
				),
			)
		}
		Mode::Hosted => {
			(
				"Run the deployment engine yourself, or connect a dedicated worker to the hosted \
				 console. Your infrastructure stays explicit.",
				(
					"Your account.\nYour infrastructure.",
					"Connect your cloud credentials to a dedicated worker. Use the console for \
					 reviewed plans, releases, encrypted state, and deployment history.",
					"Connect your infrastructure ↗",
					"/docs/guide/29-hosted-deployments.html",
				),
				(
					"A dedicated place\nto get started.",
					"LeptosKit-managed DigitalOcean capacity starts with an operator-approved \
					 quote and an isolated worker. Cloud usage is separate from the management \
					 subscription.",
					"Read about the hosted pilot ↗",
					"/docs/guide/29-hosted-deployments.html",
				),
			)
		}
	};
	let (customer_title, customer_body, customer_link, customer_href) = customer;
	let (managed_title, managed_body, managed_link, managed_href) = managed;
	let (managed_label, note) = match mode {
		Mode::Public => {
			(
				"RELEASE ON YOUR TERMS",
				"Use the deployment guides to choose the services and release workflow your app \
				 needs.",
			)
		}
		Mode::Hosted => {
			(
				"LET US HELP YOU LAUNCH",
				"Hosted access is an operator-enrolled pilot. Contact your service operator for \
				 availability, pricing, and a reviewed infrastructure quote.",
			)
		}
	};
	View()
		.id("cloud")
		.child(intro(
			"cloud-intro",
			"05 / YOUR NEXT PLACE TO LAUNCH",
			"Your app.\nYour choice of cloud.",
			description,
		))
		.child(
			View()
				.id("ownership")
				.child(
					View()
						.id("ownership-customer")
						.class("gap-5")
						.motion(arrive(Pose::new().x(-64.0).rotate(-1.5), 0))
						.child(text("ownership-customer-label", "BRING YOUR OWN"))
						.child(Heading(3, customer_title))
						.child(Text(customer_body))
						.child(link(
							"ownership-customer-link",
							customer_link,
							customer_href,
						)),
				)
				.child(
					View()
						.id("ownership-managed")
						.class("gap-5")
						.motion(arrive(Pose::new().x(64.0).rotate(1.5), 1))
						.child(text("ownership-managed-label", managed_label))
						.child(Heading(3, managed_title))
						.child(Text(managed_body))
						.child(link("ownership-managed-link", managed_link, managed_href)),
				),
		)
		.child(text("cloud-note", note))
}

fn start() -> Element {
	// Each line rises out of its own slot as the closing section arrives.
	let line = |id: &str, words: &str, column: usize| {
		Text("")
			.id(format!("{id}-slot"))
			.child(text(id, words).motion(arrive(Pose::new().y(90.0).rotate(2.0), column)))
	};
	View()
		.id("start")
		.child(
			text("start-eyebrow", "WHAT WILL YOU MAKE?")
				.motion(arrive(Pose::new().x(-16.0).clip(Inset::from_left(0.0)), 0)),
		)
		.child(
			Heading(2, "")
				.id("start-title")
				.child(line("start-line-1", "One good idea. ", 0))
				.child(line("start-line-2", "Everywhere it belongs.", 2)),
		)
		.child(
			View()
				.id("start-actions")
				.motion(arrive(Pose::new().y(28.0), 3))
				.child(link(
					"start-primary",
					"Build your first app  ↗",
					"/docs/guide/01-installation.html",
				))
				.child(link("start-source", "Explore the source ↗", REPOSITORY)),
		)
}

fn footer(mode: Mode) -> Element {
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
				.children(match mode {
					Mode::Hosted => Some(link("footer-console", "Console", "/console/")),
					Mode::Public => None,
				}),
		)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_stage_gives_every_platform_its_own_turn() {
		let changes: Vec<f32> = (0..PLATFORMS.len()).map(change_at).collect();
		assert!(changes.windows(2).all(|pair| pair[1] - pair[0] > CHANGE));
		assert!(changes.iter().all(|at| (0.0..1.0).contains(at)));
	}

	#[test]
	fn labels_show_one_platform_at_a_time() {
		// The first platform is there from the start and the last stays.
		let first = while_on_stage(0);
		assert_eq!(first.poses().first(), Some(&(0.0, Pose::REST)));
		let last = while_on_stage(PLATFORMS.len() - 1);
		assert_eq!(last.poses().last(), Some(&(1.0, Pose::REST)));

		// Every other platform's label is hidden at both ends.
		for index in 1..PLATFORMS.len() - 1 {
			let poses = while_on_stage(index);
			assert_eq!(poses.poses().first().unwrap().1, offstage(true));
			assert_eq!(poses.poses().last().unwrap().1, offstage(false));
		}
	}
}
