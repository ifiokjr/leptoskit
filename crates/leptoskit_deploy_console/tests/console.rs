use leptoskit_deploy_console::model::Intent;
use leptoskit_deploy_console::model::Model;
use leptoskit_deploy_console::model::SchemaChange;
use leptoskit_deploy_console::model::Tab;
use leptoskit_deploy_console::model::Target;
use leptoskit_deploy_console::store;
use leptoskit_deploy_console::view;
use leptoskit_testing::Finder;
use leptoskit_testing::Queries;
use leptoskit_testing::WidgetTester;

#[test]
fn changing_policy_invalidates_the_old_request() {
	let mut model = Model::default();
	model.preview(Intent::Deploy);
	let first = model.request.clone().unwrap();
	model.target(Target::Kubernetes);
	assert!(model.request.is_none());
	model.preview(Intent::Deploy);
	assert_ne!(
		first.idempotency_key,
		model.request.as_ref().unwrap().idempotency_key
	);
	assert_eq!(
		model.review(Intent::Deploy).title,
		"Cluster adapter required"
	);
	model.target(Target::RedundantVm);
	model.schema(SchemaChange::ExpandUnverified);
	assert!(model.review(Intent::Rollback).maintenance);
	model.schema(SchemaChange::Reverse);
	assert!(model.review(Intent::Deploy).maintenance);
	model.schema(SchemaChange::Unchanged);
	assert!(!model.review(Intent::Rollback).maintenance);
}

#[test]
fn every_screen_renders_with_the_supported_components_and_classes() {
	for target in [Target::SingleVm, Target::RedundantVm, Target::Kubernetes] {
		for tab in [Tab::Deployments, Tab::Infrastructure, Tab::Agents] {
			for schema in [
				SchemaChange::Unchanged,
				SchemaChange::ExpandUnverified,
				SchemaChange::Reverse,
			] {
				let mut model = Model::new(target, tab, schema);
				model.preview(Intent::Rollback);
				let tester =
					WidgetTester::new(move || view::screen(&model), 728.0, 1800.0).unwrap();
				tester
					.get_by_text("Preview data · No infrastructure connection")
					.unwrap();
			}
		}
	}
}

#[test]
fn interface_controls_change_the_same_model_an_agent_request_reads() {
	store::update(|m| *m = Model::default());
	let mut tester = WidgetTester::new(|| store::read(view::screen), 900.0, 2000.0).unwrap();
	tester.tap(&Finder::text("Inspect rollback")).unwrap();
	assert_eq!(
		store::read(|m| m.request.as_ref().unwrap().intent),
		Intent::Rollback
	);
	tester.tap(&Finder::text("Schema expansion")).unwrap();
	assert!(store::read(|m| m.request.is_none()));
	tester.tap(&Finder::text("Infrastructure")).unwrap();
	tester.tap(&Finder::text("Kubernetes")).unwrap();
	assert_eq!(store::read(|m| m.target), Target::Kubernetes);
	let serialized = store::read(|m| {
		let mut model = m.clone();
		model.preview(Intent::Deploy);
		serde_json::to_string(model.request.as_ref().unwrap()).unwrap()
	});
	assert!(serialized.contains("expected_revision"));
	assert!(!serialized.contains("token") && !serialized.contains("secret"));
}
