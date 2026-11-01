//! Main-thread tests of the ARIA-to-NSAccessibility mapping.
#![allow(unsafe_code)]

fn main() {
	let mut args = libtest_mimic::Arguments::from_args();
	args.test_threads = Some(1);
	#[cfg(target_os = "macos")]
	let trials = macos::trials();
	#[cfg(not(target_os = "macos"))]
	let trials = Vec::new();
	libtest_mimic::run(&args, trials).exit();
}

#[cfg(target_os = "macos")]
mod macos {
	use leptoskit_appkit::AppKitBackend;
	use leptoskit_appkit::MainThreadMarker;
	use leptoskit_native::NativeSurface;
	use leptoskit_native::Orientation;
	use leptoskit_native::Role;
	use leptoskit_native::components::Text;
	use leptoskit_native::components::View;
	use libtest_mimic::Trial;
	use objc2::msg_send;
	use objc2::rc::Retained;
	use objc2_app_kit::NSAccessibility;
	use objc2_app_kit::NSAccessibilityOrientation;
	use objc2_app_kit::NSApplication;
	use objc2_app_kit::NSView;
	use objc2_foundation::NSString;

	pub fn trials() -> Vec<Trial> {
		[
			(
				"explicit_roles_and_states_map_onto_nsaccessibility",
				explicit_roles_and_states_map_onto_nsaccessibility as fn(),
			),
			(
				"named_containers_become_elements_and_absorb_content",
				named_containers_become_elements_and_absorb_content,
			),
			(
				"hidden_semantics_leave_the_accessibility_tree",
				hidden_semantics_leave_the_accessibility_tree,
			),
			(
				"unmapped_roles_keep_their_aria_name",
				unmapped_roles_keep_their_aria_name,
			),
		]
		.into_iter()
		.map(|(name, test)| {
			Trial::test(name, move || {
				test();
				Ok(())
			})
		})
		.collect()
	}

	fn mounted(root: &leptoskit_native::Element) -> NativeSurface<AppKitBackend> {
		let mtm = MainThreadMarker::new().expect("main-thread test harness");
		NSApplication::sharedApplication(mtm);
		let mut surface = NativeSurface::new(AppKitBackend::new(mtm));
		surface.set_viewport(320.0, 240.0);
		surface.set_root(root);
		surface
	}

	fn first_child_view(surface: &NativeSurface<AppKitBackend>) -> &NSView {
		let id = surface.children(surface.root().unwrap())[0];
		surface.backend().view(id).unwrap()
	}

	fn explicit_roles_and_states_map_onto_nsaccessibility() {
		let surface = mounted(
			&View().child(
				View()
					.role(Role::Heading)
					.aria_label("Profile")
					.aria_level(2)
					.aria_description("Your account")
					.aria_value_text("Level 2 of 6")
					.aria_selected(true)
					.aria_expanded(true)
					.aria_required(true)
					.aria_orientation(Orientation::Vertical),
			),
		);
		let view = first_child_view(&surface);
		assert_eq!(view.accessibilityRole().unwrap().to_string(), "AXHeading");
		assert_eq!(
			view.accessibilityHelp().unwrap().to_string(),
			"Your account"
		);
		// SAFETY: the backend stores the value as a string, and the getter is
		// declared to return any object.
		let value: Option<Retained<NSString>> = unsafe { msg_send![view, accessibilityValue] };
		assert_eq!(value.unwrap().to_string(), "Level 2 of 6");
		assert!(view.isAccessibilitySelected());
		assert!(view.isAccessibilityExpanded());

		assert!(view.isAccessibilityRequired());
		assert_eq!(
			view.accessibilityOrientation(),
			NSAccessibilityOrientation::Vertical
		);
		assert_eq!(view.accessibilityLabel().unwrap().to_string(), "Profile");
	}

	fn named_containers_become_elements_and_absorb_content() {
		let mut surface =
			mounted(&View().child(View().aria_label("Order card").child(Text("Order #42"))));
		let view = first_child_view(&surface);
		assert_eq!(view.accessibilityLabel().unwrap().to_string(), "Order card");
		assert!(view.isAccessibilityElement());
		let children = view.accessibilityChildren();
		assert!(children.is_none_or(|children| children.is_empty()));

		surface.set_root(
			&View().child(
				View()
					.role(Role::List)
					.aria_label("Steps")
					.child(Text("One")),
			),
		);
		let view = first_child_view(&surface);
		assert_eq!(view.accessibilityRole().unwrap().to_string(), "AXList");
		assert!(view.isAccessibilityElement());
		// Lists keep their children in the accessibility tree.
		assert!(view.accessibilityChildren().is_some_and(|c| !c.is_empty()));
	}

	fn hidden_semantics_leave_the_accessibility_tree() {
		let surface = mounted(
			&View().child(
				View()
					.aria_label("Decorative")
					.aria_hidden(true)
					.child(Text("Chosen for looks")),
			),
		);
		let view = first_child_view(&surface);
		assert!(!view.isAccessibilityElement());
		assert_eq!(view.accessibilityLabel().unwrap().to_string(), "Decorative");
	}

	fn unmapped_roles_keep_their_aria_name() {
		let mut surface = mounted(
			&View().child(
				View()
					.role(Role::Feed)
					.aria_label("Timeline")
					.child(Text("Post")),
			),
		);
		let view = first_child_view(&surface);
		assert_eq!(view.accessibilityRole().unwrap().to_string(), "AXGroup");
		assert_eq!(
			view.accessibilityRoleDescription().unwrap().to_string(),
			"feed"
		);

		surface.set_root(
			&View().child(
				View()
					.role(Role::Feed)
					.aria_role_description("timeline")
					.aria_label("Timeline"),
			),
		);
		let view = first_child_view(&surface);
		assert_eq!(
			view.accessibilityRoleDescription().unwrap().to_string(),
			"timeline"
		);
	}
}
