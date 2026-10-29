//! `view!` and `#[component]` against the real components: the markup must
//! build the same element tree as the builder calls it stands for.

use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use leptoskit::components::helpers;
use leptoskit::components::helpers::Card;
use leptoskit::native::Color;
use leptoskit::native::HeadlessBackend;
use leptoskit::native::NativeKind;
use leptoskit::prelude::*;

/// A tree's shape and content, for comparing two ways of building it.
fn outline(element: &Element) -> String {
	let mut outline = format!(
		"{:?}[{}] {:?} {:?} {:?}",
		element.kind,
		element.classes().classes(),
		element.props,
		element.style,
		element.children.len()
	);
	for child in &element.children {
		outline.push_str("\n  ");
		outline.push_str(&outline_child(child));
	}
	outline
}

fn outline_child(element: &Element) -> String {
	outline(element).replace('\n', "\n  ")
}

#[cfg(feature = "rive")]
#[test]
fn rive_markup_builds_the_same_named_scene_as_the_builder() {
	use leptoskit::rive::Asset;
	use leptoskit::rive::Playback;
	let content = Playback::new(Asset::new(b"RIVE\x07\x00\x00\x00").unwrap());
	let markup = view! {
		<Rive
			rive_playback=content.clone()
			aria_label="Upload progress"
			running=true
			class="w-32 h-32"
		/>
	};
	let builder = Rive(content)
		.aria_label("Upload progress")
		.running(true)
		.class("w-32 h-32");
	assert_eq!(outline(&markup), outline(&builder));
	markup.validate().unwrap();
}

#[test]
fn markup_matches_the_builder_calls() {
	let count = 3;
	let ink = Color::rgba(22, 17, 31, 255);
	let markup = view! {
		<View class="flex-col gap-2 p-4" background=ink>
			<Text class="font-semibold text-[17px]">"Count: " {count}</Text>
			<Button class="rounded-lg" border=(1.0, ink) enabled=false>"Add one"</Button>
			<TextInput placeholder="Name" value="Ada" />
			<Image source="avatar.png" accessibility_label="Ada" />
		</View>
	};
	let builders = View()
		.class("flex-col gap-2 p-4")
		.background(ink)
		.child(Text("Count: 3").class("text-[17px] font-semibold"))
		.child(
			Button("Add one")
				.class("rounded-lg")
				.border(1.0, ink)
				.enabled(false),
		)
		.child(TextInput().placeholder("Name").value("Ada"))
		.child(Image("avatar.png").accessibility_label("Ada"));

	assert_eq!(outline(&markup), outline(&builders));
}

#[test]
fn controls_take_their_values_as_props() {
	let group = RadioGroup::new("size").unwrap();
	let slider = SliderState::new(0.0, 1.0, 0.5, Some(0.1)).unwrap();
	let markup = view! {
		<View>
			<Switch checked />
			<Checkbox checked=false />
			<Radio radio_group=group.clone() checked accessibility_label="Small" />
			<Slider slider=slider />
			<LinearProgressIndicator progress=Progress::Indeterminate />
			<ActivityIndicator running=false />
			<PasswordInput placeholder="Password" />
			<Pressable accessibility_label="Open">
				<Text>"Open"</Text>
			</Pressable>
			<ScrollView class="flex-1" />
		</View>
	};
	let builders = View().children([
		Switch(true),
		Checkbox(false),
		Radio(group, true).accessibility_label("Small"),
		Slider(slider),
		LinearProgressIndicator(Progress::Indeterminate),
		ActivityIndicator(false),
		PasswordInput().placeholder("Password"),
		Pressable("Open").child(Text("Open")),
		ScrollView().class("flex-1"),
	]);

	assert_eq!(outline(&markup), outline(&builders));
	markup.validate().expect("the controls form a valid tree");
}

#[test]
fn live_regions_take_their_text_as_content() {
	let markup = view! {
		<View>
			<Status class="text-sm">"Saved"</Status>
			<Alert>"Offline"</Alert>
		</View>
	};
	let builders = View()
		.child(Status("Saved").class("text-sm"))
		.child(Alert("Offline"));

	assert_eq!(outline(&markup), outline(&builders));
}

#[test]
fn helpers_with_paths_are_tags() {
	let markup = view! {
		<Card>
			<helpers::Divider />
		</Card>
	};
	assert_eq!(outline(&markup), outline(&Card().child(helpers::Divider())));
}

#[test]
fn handlers_fire_from_markup() {
	let presses = Arc::new(AtomicUsize::new(0));
	let counter = Arc::clone(&presses);
	let changes = Arc::new(AtomicUsize::new(0));
	let change_count = Arc::clone(&changes);
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(390.0, 844.0);
	surface.set_root(&view! {
		<View>
			<Button on:press=move || {
				counter.fetch_add(1, Ordering::SeqCst);
			}>
				"Tap"
			</Button>
			<TextInput
				placeholder="Name"
				on:change=move |value: &str| {
					change_count.fetch_add(value.len(), Ordering::SeqCst);
				}
			/>
		</View>
	});

	let root = surface.root().expect("a mounted root");
	let [button, input] = surface.children(root) else {
		panic!("two children");
	};
	let (button, input) = (*button, *input);
	surface.dispatch_event(button, &NativeEvent::Press);
	surface.dispatch_event(
		input,
		&NativeEvent::Change {
			value: "four".to_owned(),
		},
	);

	assert_eq!(presses.load(Ordering::SeqCst), 1);
	assert_eq!(changes.load(Ordering::SeqCst), 4);
}

#[test]
fn children_can_be_optional_repeated_or_closures() {
	let names = ["Ada", "Grace"];
	let unread = Some(2);
	let markup = view! {
		<View>
			{names
				.iter()
				.map(|name| view! { <Text>{*name}</Text> })
				.collect::<Fragment>()}
			{unread.map(|count| view! { <Text>{count}" unread"</Text> })} {None::<Element>}
			{move || "from a closure"} <>"in a fragment"</>
			{
				let shout = "!";
				shout.repeat(2)
			}
		</View>
	};
	let texts = markup
		.children
		.iter()
		.map(|child| child.props.text.clone().unwrap_or_default())
		.collect::<Vec<_>>();

	assert_eq!(
		texts,
		[
			"Ada",
			"Grace",
			"2 unread",
			"from a closure",
			"in a fragment",
			"!!"
		]
	);
}

#[test]
fn several_roots_make_a_fragment() {
	let fragment: Fragment = view! {
		<Text>"one"</Text>
		<Text>"two"</Text>
	};
	assert_eq!(fragment.len(), 2);

	let empty: Fragment = view! {};
	assert!(empty.is_empty());
}

/// A labelled row; the docs on each prop appear in this function's docs.
#[component]
fn Row(
	/// What the row is about.
	#[prop(into)]
	title: String,
	#[prop(optional)] detail: Option<String>,
	#[prop(default = 1)] weight: u8,
	#[prop(optional)] tags: Vec<&'static str>,
	children: Children,
) -> impl IntoView {
	View()
		.class("flex-row gap-2")
		.child(Text(title))
		.child(Text(detail.unwrap_or_default()))
		.child(Text(weight.to_string()))
		.child(Text(tags.join(",")))
		.children(children)
}

#[test]
fn components_take_named_props() {
	let markup = view! {
		<Row title="Wi-Fi" detail="On".to_owned() tags=vec!["fast"]>
			<Switch checked />
		</Row>
	};
	let positional = Row(
		"Wi-Fi".to_owned(),
		Some("On".to_owned()),
		1,
		vec!["fast"],
		view! {
			<>
				<Switch checked />
			</>
		},
	)
	.into_view();

	assert_eq!(outline(&markup), outline(&positional));
}

#[test]
fn optional_props_fall_back_to_their_defaults() {
	let markup = view! {
		<Row title="Bluetooth" weight=3>
			<></>
		</Row>
	};
	let texts = markup
		.children
		.iter()
		.map(|child| child.props.text.clone().unwrap_or_default())
		.collect::<Vec<_>>();
	assert_eq!(texts, ["Bluetooth", "", "3", ""]);
}

/// A greeting that borrows its name and takes any displayable count.
#[component]
fn Greeting<T: std::fmt::Display + 'static>(
	name: &str,
	count: T,
	format: impl Fn(&str, &T) -> String,
) -> Element {
	Text(format(name, &count))
}

#[test]
fn components_keep_generics_and_borrowed_props() {
	let name = String::from("Ada");
	let markup = view! {
		<Greeting name=&name count=3_u8 format=|name, count| format!("{name} has {count}") />
	};
	assert_eq!(markup.props.text.as_deref(), Some("Ada has 3"));
	assert_eq!(markup.kind, NativeKind::Text);
}

mod screens {
	use leptoskit::prelude::*;

	/// A component in another module, called through its path.
	#[component]
	pub fn Banner(#[prop(into)] message: String) -> impl IntoView {
		move || Text(message.clone()).class("font-bold")
	}
}

#[test]
fn components_are_called_through_paths() {
	let markup = view! { <screens::Banner message="Saved" /> };
	assert_eq!(markup.props.text.as_deref(), Some("Saved"));
	assert_eq!(markup.classes().classes(), "font-bold");
}

#[test]
fn components_and_markup_nest() {
	let markup = view! {
		<View>
			<screens::Banner message="Top" />
			{screens::Banner("Positional".to_owned())}
		</View>
	};
	assert_eq!(markup.children.len(), 2);
	assert_eq!(markup.children[1].props.text.as_deref(), Some("Positional"));
}

/// A meter whose prop names match the builder's own words.
#[component]
fn Meter(value: u8, #[prop(optional)] r#type: Option<&'static str>) -> Element {
	Text(format!("{}: {value}", r#type.unwrap_or("level")))
}

#[test]
fn props_may_share_names_with_builder_internals() {
	let markup = view! { <Meter r#type="volume" value=7 /> };
	assert_eq!(markup.props.text.as_deref(), Some("volume: 7"));
	assert_eq!(
		view! { <Meter value=2 /> }.props.text.as_deref(),
		Some("level: 2")
	);
}
