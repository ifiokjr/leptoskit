//! The shared ARIA model: roles, builders, validation, and the resolver that
//! folds relationships into concrete fields for native backends.

use leptoskit_native::Aria;
use leptoskit_native::ControlTreeError;
use leptoskit_native::Current;
use leptoskit_native::Element;
use leptoskit_native::HasPopup;
use leptoskit_native::HeadlessBackend;
use leptoskit_native::Invalid;
use leptoskit_native::Live;
use leptoskit_native::Mutation;
use leptoskit_native::NativeKind;
use leptoskit_native::NativeProps;
use leptoskit_native::NativeSurface;
use leptoskit_native::Orientation;
use leptoskit_native::RadioGroup;
use leptoskit_native::Role;
use leptoskit_native::Tristate;
use leptoskit_native::components::ActivityIndicator;
use leptoskit_native::components::Button;
use leptoskit_native::components::Checkbox;
use leptoskit_native::components::Image;
use leptoskit_native::components::PasswordInput;
use leptoskit_native::components::Pressable;
use leptoskit_native::components::Radio;
use leptoskit_native::components::Slider;
use leptoskit_native::components::Switch;
use leptoskit_native::components::Text;
use leptoskit_native::components::TextInput;
use leptoskit_native::components::View;
use leptoskit_native::tailwind::StyleContext;

#[test]
fn every_role_round_trips_through_its_aria_name() {
	assert_eq!(Role::ALL.len(), 81);

	for role in Role::ALL {
		assert_eq!(Role::from_wire(role.wire_name()), Some(*role));
		assert_eq!(role.to_string(), role.wire_name());
		assert!(
			role.wire_name()
				.bytes()
				.all(|byte| byte.is_ascii_lowercase())
		);
	}

	assert_eq!(Role::from_wire("Button"), None);
	assert!(Role::Button.is_interactive());
	assert!(!Role::Heading.is_interactive());
	assert!(Role::Slider.is_range());
	assert!(Role::None.is_presentational() && Role::Presentation.is_presentational());
	assert!(Role::Main.is_landmark());
	assert_eq!(Role::Alert.implicit_live(), Some(Live::Assertive));
	assert_eq!(Role::Status.implicit_live(), Some(Live::Polite));
	assert_eq!(Role::Marquee.implicit_live(), None);
	assert!(Role::Alert.implicit_atomic());
}

#[test]
fn widget_kinds_imply_their_roles_and_explicit_roles_override_them() {
	assert_eq!(Role::implicit(NativeKind::Button), Some(Role::Button));
	assert_eq!(Role::implicit(NativeKind::Pressable), Some(Role::Button));
	assert_eq!(Role::implicit(NativeKind::Switch), Some(Role::Switch));
	assert_eq!(Role::implicit(NativeKind::Checkbox), Some(Role::Checkbox));
	assert_eq!(Role::implicit(NativeKind::Radio), Some(Role::Radio));
	assert_eq!(Role::implicit(NativeKind::Slider), Some(Role::Slider));
	assert_eq!(Role::implicit(NativeKind::TextInput), Some(Role::TextBox));
	assert_eq!(
		Role::implicit(NativeKind::PasswordInput),
		Some(Role::TextBox)
	);
	assert_eq!(Role::implicit(NativeKind::Image), Some(Role::Img));
	assert_eq!(
		Role::implicit(NativeKind::LinearProgressIndicator),
		Some(Role::ProgressBar)
	);
	assert_eq!(
		Role::implicit(NativeKind::ActivityIndicator),
		Some(Role::ProgressBar)
	);
	assert_eq!(Role::implicit(NativeKind::View), None);
	assert_eq!(Role::implicit(NativeKind::Text), None);
	assert_eq!(Role::implicit(NativeKind::Custom("canvas")), None);

	let heading = Text("Profile").role(Role::Heading).aria_level(2);
	assert_eq!(
		heading.props.aria.effective_role(heading.kind),
		Some(Role::Heading)
	);
	assert!(!heading.props.aria.is_interactive(heading.kind));

	let link = Pressable("Docs").role(Role::Link);
	assert_eq!(link.props.aria.effective_role(link.kind), Some(Role::Link));
	assert!(link.props.aria.is_interactive(link.kind));

	let status = View().role(Role::Status);
	assert_eq!(status.props.aria.effective_live(status.kind), Live::Polite);
	assert_eq!(
		View()
			.role(Role::Status)
			.aria_live(Live::Off)
			.props
			.aria
			.effective_live(NativeKind::View),
		Live::Off
	);
}

#[test]
fn builders_set_every_aria_property_and_the_legacy_label_is_an_alias() {
	let element = View()
		.id("panel")
		.role(Role::TabPanel)
		.aria_label("Details")
		.aria_labelled_by(["tab-details"])
		.aria_description("Shows the order")
		.aria_described_by(["help", "error"])
		.aria_role_description("panel")
		.aria_level(3)
		.aria_pos_in_set(2)
		.aria_set_size(5)
		.aria_orientation(Orientation::Vertical)
		.aria_hidden(false)
		.aria_busy(true)
		.aria_checked(Tristate::Mixed)
		.aria_pressed(true)
		.aria_selected(true)
		.aria_expanded(false)
		.aria_current(Current::Page)
		.aria_modal(true)
		.aria_required(true)
		.aria_invalid(Invalid::Spelling)
		.aria_read_only(true)
		.aria_multiline(false)
		.aria_has_popup(HasPopup::Menu)
		.aria_value_now(5.0)
		.aria_value_min(0.0)
		.aria_value_max(10.0)
		.aria_value_text("five")
		.aria_live(Live::Assertive)
		.aria_atomic(true)
		.aria_controls(["panel-body"])
		.tab_index(-1)
		.aria_attr("aria-rowindex", "3");
	let aria = &element.props.aria;

	assert_eq!(element.props.id.as_deref(), Some("panel"));
	assert_eq!(aria.role, Some(Role::TabPanel));
	assert_eq!(aria.label.as_deref(), Some("Details"));
	assert_eq!(aria.labelled_by, ["tab-details"]);
	assert_eq!(aria.description.as_deref(), Some("Shows the order"));
	assert_eq!(aria.described_by, ["help", "error"]);
	assert_eq!(aria.role_description.as_deref(), Some("panel"));
	assert_eq!(aria.level, Some(3));
	assert_eq!(aria.pos_in_set, Some(2));
	assert_eq!(aria.set_size, Some(5));
	assert_eq!(aria.orientation, Some(Orientation::Vertical));
	assert_eq!(aria.hidden, Some(false));
	assert_eq!(aria.busy, Some(true));
	assert_eq!(aria.checked, Some(Tristate::Mixed));
	assert_eq!(aria.pressed, Some(Tristate::True));
	assert_eq!(aria.selected, Some(true));
	assert_eq!(aria.expanded, Some(false));
	assert_eq!(aria.current, Some(Current::Page));
	assert_eq!(aria.modal, Some(true));
	assert_eq!(aria.required, Some(true));
	assert_eq!(aria.invalid, Some(Invalid::Spelling));
	assert_eq!(aria.read_only, Some(true));
	assert_eq!(aria.multiline, Some(false));
	assert_eq!(aria.has_popup, Some(HasPopup::Menu));
	assert_eq!(aria.value_now, Some(5.0));
	assert_eq!(aria.value_min, Some(0.0));
	assert_eq!(aria.value_max, Some(10.0));
	assert_eq!(aria.value_text.as_deref(), Some("five"));
	assert_eq!(aria.live, Some(Live::Assertive));
	assert_eq!(aria.atomic, Some(true));
	assert_eq!(aria.controls, ["panel-body"]);
	assert_eq!(aria.tab_index, Some(-1));
	assert_eq!(
		aria.extra.get("aria-rowindex").map(String::as_str),
		Some("3")
	);

	let legacy = Button("Save").accessibility_label("Save the order");
	assert_eq!(legacy.props.aria.label.as_deref(), Some("Save the order"));
	assert_eq!(legacy.props.accessible_name(), Some("Save the order"));
	assert_eq!(Button("Save").props.accessible_name(), Some("Save"));
	assert_eq!(
		View()
			.with_aria(|aria| aria.hidden = Some(true))
			.props
			.aria
			.hidden,
		Some(true)
	);
	assert_eq!(Tristate::from(false), Tristate::False);
	assert_eq!(Invalid::from(true), Invalid::True);
	assert_eq!(HasPopup::from(false), HasPopup::False);
}

#[test]
fn text_inputs_require_an_accessible_name_but_accept_any_naming_channel() {
	for kind in [NativeKind::TextInput, NativeKind::PasswordInput] {
		assert_eq!(
			Element::new(kind).validate(),
			Err(ControlTreeError::MissingInputLabel { kind })
		);
		assert_eq!(
			Element::new(kind).placeholder("  ").validate(),
			Err(ControlTreeError::MissingInputLabel { kind })
		);
		assert!(Element::new(kind).aria_label("Email").validate().is_ok());
		assert!(Element::new(kind).placeholder("Search").validate().is_ok());
	}

	assert!(
		View()
			.child(Text("Email").id("email-label"))
			.child(TextInput().aria_labelled_by(["email-label"]))
			.validate()
			.is_ok()
	);
	assert!(
		View()
			.child(Text("Open").id("open-label"))
			.child(
				Pressable("")
					.aria_labelled_by(["open-label"])
					.child(Text("Open"))
			)
			.validate()
			.is_ok()
	);
	assert_eq!(
		Pressable(" ").child(Text("Open")).validate(),
		Err(ControlTreeError::MissingPressableLabel)
	);
	assert_eq!(
		ControlTreeError::MissingInputLabel {
			kind: NativeKind::TextInput
		}
		.to_string(),
		"text_input requires an accessible name: set aria_label, aria_labelled_by, or a \
		 placeholder"
	);
}

#[test]
fn ids_are_unique_and_relationships_reference_existing_ids() {
	assert_eq!(
		View()
			.child(Text("One").id("twin"))
			.child(Text("Two").id("twin"))
			.validate(),
		Err(ControlTreeError::DuplicateId { id: "twin".into() })
	);

	for (attribute, element) in [
		("aria-labelledby", View().aria_labelled_by(["missing"])),
		("aria-describedby", View().aria_described_by(["missing"])),
		("aria-controls", View().aria_controls(["missing"])),
	] {
		assert_eq!(
			View()
				.child(Text("x").id("present"))
				.child(element)
				.validate(),
			Err(ControlTreeError::UnknownIdReference {
				attribute,
				id: "missing".into()
			})
		);
	}

	assert!(
		View()
			.child(Text("Help").id("help"))
			.child(
				Button("Go")
					.aria_described_by(["help"])
					.aria_controls(["help"])
			)
			.validate()
			.is_ok()
	);
	assert_eq!(
		ControlTreeError::UnknownIdReference {
			attribute: "aria-controls",
			id: "panel".into()
		}
		.to_string(),
		"aria-controls references id `panel`, which no element in the tree has"
	);
}

#[test]
fn spec_rules_reject_named_presentation_zero_levels_and_bad_passthrough_names() {
	assert_eq!(
		Image("/decor.png")
			.role(Role::Presentation)
			.aria_label("Decoration")
			.validate(),
		Err(ControlTreeError::LabelOnPresentation)
	);
	assert_eq!(
		View().role(Role::None).aria_labelled_by(["x"]).validate(),
		Err(ControlTreeError::LabelOnPresentation)
	);
	assert!(
		Image("/decor.png")
			.role(Role::Presentation)
			.validate()
			.is_ok()
	);
	assert_eq!(
		Text("Title").role(Role::Heading).aria_level(0).validate(),
		Err(ControlTreeError::InvalidAriaLevel)
	);
	assert!(
		Text("Title")
			.role(Role::Heading)
			.aria_level(7)
			.validate()
			.is_ok()
	);

	for name in [
		"rowindex",
		"aria-",
		"aria-RowIndex",
		"aria-label",
		"aria-valuenow",
		"data-x",
	] {
		assert_eq!(
			View().aria_attr(name, "1").validate(),
			Err(ControlTreeError::InvalidAriaAttribute { name: name.into() }),
			"{name}"
		);
	}

	assert!(View().aria_attr("aria-rowindex", "1").validate().is_ok());
	assert!(
		View()
			.aria_attr("aria-activedescendant", "opt-1")
			.validate()
			.is_ok()
	);
}

#[test]
fn errors_display_their_rule() {
	assert_eq!(
		ControlTreeError::DuplicateId { id: "a".into() }.to_string(),
		"id `a` is used by more than one element"
	);
	assert_eq!(
		ControlTreeError::LabelOnPresentation.to_string(),
		"a presentational role cannot have an accessible name"
	);
	assert_eq!(
		ControlTreeError::InvalidAriaLevel.to_string(),
		"aria_level must be at least one"
	);
	assert!(
		ControlTreeError::InvalidAriaAttribute {
			name: "aria-label".into()
		}
		.to_string()
		.contains("`aria-label` is not a passthrough ARIA attribute")
	);
}

#[test]
fn resolver_folds_references_into_names_and_descriptions_for_native_backends() {
	let tree = View()
		.child(Text("Email").id("email-label"))
		.child(Text("We never share it").id("email-help").aria_hidden(true))
		.child(
			TextInput()
				.id("email")
				.aria_label("Ignored once referenced")
				.aria_labelled_by(["email-label"])
				.aria_described_by(["email-help", "email-label"]),
		)
		.child(
			View()
				.id("card")
				.child(Text("Order").class("font-bold"))
				.child(Text("#42"))
				.child(Text("decorative").aria_hidden(true))
				.child(
					View()
						.with_props(|props| props.hidden = Some(true))
						.child(Text("gone")),
				),
		)
		.child(Button("Open").aria_labelled_by(["card", "unknown"]));
	assert!(
		tree.validate().is_err(),
		"unknown references fail validation"
	);

	let resolved = tree.resolved(&StyleContext::default());
	let input = &resolved.children[2];
	assert_eq!(input.props.aria.label.as_deref(), Some("Email"));
	assert_eq!(
		input.props.aria.description.as_deref(),
		Some("We never share it Email")
	);
	assert_eq!(input.props.aria.labelled_by, ["email-label"]);

	let button = &resolved.children[4];
	assert_eq!(button.props.aria.label.as_deref(), Some("Order #42"));
	assert_eq!(resolved.children[3].text_content(), "Order #42");
	assert_eq!(
		tree.text_content(),
		"Email Ignored once referenced Order #42 Open"
	);

	let untouched = tree.children[2].props.aria.label.as_deref();
	assert_eq!(untouched, Some("Ignored once referenced"));
}

#[test]
fn resolver_numbers_radio_options_and_keeps_explicit_positions() {
	let group = RadioGroup::new("size").unwrap();
	let other = RadioGroup::new("color").unwrap();
	let tree = View()
		.child(Radio(group.clone(), true).aria_label("Small"))
		.child(View().child(Radio(group.clone(), false).aria_label("Medium")))
		.child(Radio(other, false).aria_label("Red"))
		.child(
			Radio(group, false)
				.aria_label("Large")
				.aria_pos_in_set(9)
				.aria_set_size(9),
		);
	let resolved = tree.resolved(&StyleContext::default());
	let positions =
		|element: &Element| (element.props.aria.pos_in_set, element.props.aria.set_size);

	assert_eq!(positions(&resolved.children[0]), (Some(1), Some(3)));
	assert_eq!(
		positions(&resolved.children[1].children[0]),
		(Some(2), Some(3))
	);
	assert_eq!(positions(&resolved.children[2]), (Some(1), Some(1)));
	assert_eq!(positions(&resolved.children[3]), (Some(9), Some(9)));
	assert_eq!(positions(&tree.children[0]), (None, None));
}

#[test]
fn resolver_fills_live_text_and_inherits_hidden_semantics() {
	let tree = View()
		.child(
			View()
				.aria_hidden(true)
				.child(View().child(Text("Decorative"))),
		)
		.child(
			View()
				.role(Role::Status)
				.child(Text("3 items"))
				.child(Text("in your cart")),
		)
		.child(Text("Saved").aria_live(Live::Polite))
		.child(Text("Plain"))
		.child(
			View()
				.aria_hidden(true)
				.role(Role::Status)
				.child(Text("Muted")),
		);
	let resolved = tree.resolved(&StyleContext::default());

	assert_eq!(resolved.children[0].props.aria.hidden, Some(true));
	assert_eq!(
		resolved.children[0].children[0].props.aria.hidden,
		Some(true)
	);
	assert_eq!(
		resolved.children[0].children[0].children[0]
			.props
			.aria
			.hidden,
		Some(true)
	);
	assert!(resolved.children[0].children[0].props.aria.is_hidden());
	assert_eq!(
		resolved.children[1].props.aria.live_text.as_deref(),
		Some("3 items in your cart")
	);
	assert_eq!(
		resolved.children[2].props.aria.live_text.as_deref(),
		Some("Saved")
	);
	assert_eq!(resolved.children[3].props.aria.live_text, None);
	assert_eq!(resolved.children[4].props.aria.live_text, None);
	assert_eq!(tree.children[1].props.aria.live_text, None);
	assert_eq!(tree.children[0].children[0].props.aria.hidden, None);
}

#[test]
fn a_role_change_recreates_the_widget_like_a_kind_change() {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_root(&View().child(View().role(Role::Group).aria_label("Options")));
	let root = surface.root().unwrap();
	let group = surface.children(root)[0];
	surface.backend_mut().clear();

	surface.set_root(&View().child(View().role(Role::Group).aria_label("Choices")));
	assert_eq!(surface.children(root)[0], group);
	assert!(surface.backend().log().iter().all(|mutation| {
		matches!(
			mutation,
			Mutation::UpdateProps { .. } | Mutation::UpdateLayout { .. }
		)
	}));
	assert_eq!(surface.props(group).aria.label.as_deref(), Some("Choices"));
	surface.backend_mut().clear();

	surface.set_root(&View().child(View().role(Role::RadioGroup).aria_label("Choices")));
	let replaced = surface.children(root)[0];
	assert_ne!(replaced, group);
	assert!(
		surface
			.backend()
			.log()
			.iter()
			.any(|mutation| { matches!(mutation, Mutation::Delete { id } if *id == group) })
	);
	assert!(surface.backend().log().iter().any(|mutation| {
		matches!(mutation, Mutation::Create { id, kind: NativeKind::View, .. } if *id == replaced)
	}));
	assert_eq!(surface.props(replaced).aria.role, Some(Role::RadioGroup));
}

#[test]
fn controls_carry_their_states_through_the_committed_tree() {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_root(
		&View()
			.child(Switch(true).aria_label("Wi-Fi"))
			.child(
				Checkbox(false)
					.aria_label("Agree")
					.aria_checked(Tristate::Mixed),
			)
			.child(
				Slider(leptoskit_native::SliderState::new(0.0, 1.0, 0.5, None).unwrap())
					.aria_label("Volume")
					.aria_value_text("50 percent"),
			)
			.child(ActivityIndicator(true))
			.child(PasswordInput().aria_label("Password").aria_required(true)),
	);
	let root = surface.root().unwrap();
	let children = surface.children(root).to_vec();

	assert_eq!(
		surface.props(children[1]).aria.checked,
		Some(Tristate::Mixed)
	);
	assert_eq!(
		surface.props(children[2]).aria.value_text.as_deref(),
		Some("50 percent")
	);
	assert_eq!(surface.props(children[4]).aria.required, Some(true));
	assert_eq!(surface.props(children[3]).accessible_name(), None);
	assert_eq!(surface.props(children[0]).accessible_name(), Some("Wi-Fi"));
}

#[cfg(feature = "serde")]
#[test]
fn aria_props_keep_the_wire_format_additive_and_lenient() {
	let legacy = r#"{"text":"Count: 1","accessibility_label":"Counter","enabled":true,"extra":{}}"#;
	let props: NativeProps = serde_json::from_str(legacy).expect("legacy props decode");
	assert_eq!(props.aria.label.as_deref(), Some("Counter"));
	assert_eq!(props.aria.role, None);
	assert_eq!(props.id, None);

	let encoded = serde_json::to_value(NativeProps::default()).expect("encode");
	assert!(
		encoded
			.get("accessibility_label")
			.is_some_and(serde_json::Value::is_null)
	);
	assert!(encoded.get("id").is_none());
	assert!(encoded.get("role").is_none());
	assert!(encoded.get("aria_live").is_none());
	assert!(encoded.get("aria_extra").is_none());
	assert!(encoded.get("aria_labelled_by").is_none());

	let element = View()
		.id("status")
		.role(Role::Status)
		.aria_label("Cart")
		.aria_live(Live::Polite)
		.aria_checked(Tristate::Mixed)
		.aria_current(Current::Page)
		.aria_has_popup(HasPopup::ListBox)
		.aria_invalid(Invalid::Grammar)
		.aria_orientation(Orientation::Vertical)
		.aria_value_now(2.5)
		.aria_labelled_by(["a", "b"])
		.aria_attr("aria-rowindex", "3")
		.tab_index(0);
	let json = serde_json::to_value(&element.props).expect("encode");
	assert_eq!(json["id"], "status");
	assert_eq!(json["role"], "status");
	assert_eq!(json["accessibility_label"], "Cart");
	assert_eq!(json["aria_live"], "polite");
	assert_eq!(json["aria_checked"], "mixed");
	assert_eq!(json["aria_current"], "page");
	assert_eq!(json["aria_has_popup"], "listbox");
	assert_eq!(json["aria_invalid"], "grammar");
	assert_eq!(json["aria_orientation"], "vertical");
	assert_eq!(json["aria_value_now"], 2.5);
	assert_eq!(json["aria_labelled_by"], serde_json::json!(["a", "b"]));
	assert_eq!(json["aria_extra"]["aria-rowindex"], "3");
	assert_eq!(json["tab_index"], 0);
	let decoded: NativeProps = serde_json::from_value(json).expect("decode");
	assert_eq!(decoded, element.props);

	let newer = r#"{"role":"holographic-dial","aria_live":"polite"}"#;
	let props: NativeProps = serde_json::from_str(newer).expect("unknown roles are ignored");
	assert_eq!(props.aria.role, None);
	assert_eq!(props.aria.live, Some(Live::Polite));
	assert!(serde_json::from_str::<Role>("\"holographic-dial\"").is_err());
	assert_eq!(
		serde_json::from_str::<Role>("\"menuitemcheckbox\"").unwrap(),
		Role::MenuItemCheckbox
	);

	let aria: Aria = serde_json::from_str("{}").expect("empty aria");
	assert_eq!(aria, Aria::default());
}
