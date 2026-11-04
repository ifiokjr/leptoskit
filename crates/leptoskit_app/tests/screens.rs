use leptoskit_app::Shell;
use leptoskit_app::screen;
use leptoskit_deploy_console::hosted_protocol::AccountView;
use leptoskit_deploy_console::hosted_protocol::Installation;
use leptoskit_deploy_console::hosted_protocol::Ownership;
use leptoskit_testing::Queries;
use leptoskit_testing::WidgetTester;
use reactive_graph::owner::Owner;
use reactive_graph::prelude::*;
#[test]
fn shared_screens_render_on_phone_and_web_without_exposing_account_keys() {
	for width in [390.0, 1024.0] {
		let owner = Owner::new();
		let shell = owner.with(Shell::new);
		let tester =
			WidgetTester::new(move || owner.with(|| screen::screen(shell)), width, 1800.0).unwrap();
		tester.get_by_text("Connect your account").unwrap();
		tester.get_by_text("Documentation ↗").unwrap();
		let owner = Owner::new();
		let shell = owner.with(Shell::new);
		shell.key.set("never-render-the-account-secret".into());
		shell.account.set(Some(AccountView {
			id: "example".into(),
			status: "active".into(),
			plan: "team".into(),
			installations: vec![Installation {
				id: "example".into(),
				project: "notes".into(),
				environment: "production".into(),
				ownership: Ownership::Customer,
				last_seen: 0,
			}],
		}));
		let tester =
			WidgetTester::new(move || owner.with(|| screen::screen(shell)), width, 2400.0).unwrap();
		tester.get_by_text("Pair worker").unwrap();
		tester.get_by_text("Plan infrastructure").unwrap();
		assert!(
			tester
				.query_by_text("never-render-the-account-secret")
				.unwrap()
				.is_none()
		);
	}
}

#[test]
fn the_public_example_uses_working_shared_components() {
	use leptoskit_testing::Finder;
	use reactive_graph::signal::RwSignal;
	for width in [390.0, 1024.0] {
		let owner = Owner::new();
		let count = owner.with(|| RwSignal::new(0));
		let mut tester = WidgetTester::new(
			move || owner.with(|| leptoskit_app::landing::demo(count)),
			width,
			600.0,
		)
		.unwrap();
		tester.get_by_text("0 ideas shipped").unwrap();
		tester.tap(&Finder::test_id("demo-increment")).unwrap();
		tester.get_by_text("1 ideas shipped").unwrap();
		tester.tap(&Finder::test_id("demo-increment")).unwrap();
		tester.get_by_text("2 ideas shipped").unwrap();
	}
}
