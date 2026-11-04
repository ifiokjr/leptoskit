//! The console uses the same component tree for browser and headless tests.
use leptoskit::prelude::*;

use crate::model::Intent;
use crate::model::Model;
use crate::model::SchemaChange;
use crate::model::Tab;
use crate::model::Target;
use crate::store;

const QUIET: &str = "text-sm text-slate-500 dark:text-slate-400";
const CARD: &str = "gap-3 rounded-xl border border-slate-200 p-4 dark:border-slate-800";
const BUTTON: &str = "rounded-lg border border-slate-300 px-3 py-2 text-sm dark:border-slate-700";
const SELECTED: &str =
	"rounded-lg bg-slate-900 px-3 py-2 text-sm text-white dark:bg-slate-100 dark:text-slate-900";

#[must_use]
pub fn screen(model: &Model) -> Element {
	View()
		.class("w-full gap-5 text-slate-900 dark:text-slate-100")
		.test_id("deploy-console")
		.child(
			View()
				.class(
					"flex-row flex-wrap items-center gap-3 border-b border-slate-200 pb-4 \
					 dark:border-slate-800",
				)
				.child(Text("◈").class("text-2xl"))
				.child(Text("leptoskit").class("text-base font-semibold"))
				.child(Text("/  Workspace").class(QUIET))
				.child(View().class("flex-1"))
				.child(
					Text("DESIGN PROTOTYPE")
						.class("text-xs font-semibold text-amber-600 dark:text-amber-400"),
				),
		)
		.child(
			View()
				.class("gap-1")
				.child(Heading(1, "Fieldnotes").class("text-3xl font-semibold"))
				.child(
					Text(
						"One project. Your infrastructure. A shared workflow for agents and \
						 people.",
					)
					.class(QUIET),
				),
		)
		.child(
			View().class("flex-row flex-wrap gap-2").children(
				[
					(Tab::Deployments, "Deployments", "tab-deployments"),
					(Tab::Infrastructure, "Infrastructure", "tab-infrastructure"),
					(Tab::Agents, "Agent access", "tab-agents"),
				]
				.into_iter()
				.map(|(tab, title, id)| {
					Button(title)
						.test_id(id)
						.class(if tab == model.tab { SELECTED } else { BUTTON })
						.aria_pressed(tab == model.tab)
						.on_press(move || store::update(|m| m.tab = tab))
				}),
			),
		)
		.child(
			View()
				.class("gap-1 rounded-lg bg-amber-50 p-3 dark:bg-amber-950")
				.child(
					Text("Preview data · No infrastructure connection")
						.class("text-sm font-semibold text-amber-800 dark:text-amber-200"),
				)
				.child(
					Text(
						"Controls explore the proposed workflow. They do not deploy, scale, roll \
						 back, or accept credentials.",
					)
					.class("text-xs text-amber-800 dark:text-amber-200"),
				),
		)
		.child(match model.tab {
			Tab::Deployments => deployments(model),
			Tab::Infrastructure => infrastructure(model),
			Tab::Agents => agents(model),
		})
}

fn metric(label: &str, value: &str, note: &str) -> Element {
	View()
		.class(format!("flex-1 {CARD}"))
		.child(Text(label).class(QUIET))
		.child(Text(value).class("text-lg font-semibold"))
		.child(Text(note).class("text-xs text-slate-500 dark:text-slate-400"))
}

fn deployments(model: &Model) -> Element {
	let intent = model.request.as_ref().map_or(Intent::Deploy, |r| r.intent);
	let review = model.review(intent);
	View()
		.class("gap-5")
		.child(
			View()
				.class("flex-row gap-3")
				.child(metric("Environment", "Production", "Example workspace"))
				.child(metric(
					"Active release",
					"release-12",
					"Sample state, not live health",
				))
				.child(metric(
					"Runners",
					"Not connected",
					"No operations can execute",
				)),
		)
		.child(
			View()
				.class(CARD)
				.child(
					View()
						.class("flex-row flex-wrap items-center gap-3")
						.child(
							Heading(2, "Current deployment").class("flex-1 text-lg font-semibold"),
						)
						.child(
							Text("●  Sample ready")
								.class("text-sm text-emerald-600 dark:text-emerald-400"),
						),
				)
				.child(Text("release-12  /  a1b2c3d  /  main").class("text-sm"))
				.child(
					Text("Website, API, and live connections share one Rust application image.")
						.class(QUIET),
				)
				.child(
					View()
						.class("flex-row flex-wrap gap-2")
						.child(
							Button("Preview next release")
								.test_id("preview-deploy")
								.class(SELECTED)
								.on_press(|| store::update(|m| m.preview(Intent::Deploy))),
						)
						.child(
							Button("Inspect rollback")
								.test_id("preview-rollback")
								.class(BUTTON)
								.on_press(|| store::update(|m| m.preview(Intent::Rollback))),
						),
				),
		)
		.child(
			View()
				.class(CARD)
				.child(Heading(2, "Before traffic moves").class("text-lg font-semibold"))
				.child(Text("Explore how database changes affect a release.").class(QUIET))
				.child(
					View().class("flex-row flex-wrap gap-2").children(
						[
							(SchemaChange::Unchanged, "Same schema", "schema-same"),
							(
								SchemaChange::ExpandUnverified,
								"Schema expansion",
								"schema-expand",
							),
							(SchemaChange::Reverse, "Reverse migration", "schema-reverse"),
						]
						.into_iter()
						.map(|(schema, title, id)| {
							Button(title)
								.test_id(id)
								.class(if schema == model.schema {
									SELECTED
								} else {
									BUTTON
								})
								.aria_pressed(schema == model.schema)
								.on_press(move || store::update(|m| m.schema(schema)))
						}),
					),
				)
				.child(
					Text(review.title)
						.class("text-base font-semibold")
						.test_id("review-title"),
				)
				.child(Text(review.detail).class(QUIET))
				.child(
					Text(if review.maintenance {
						"Maintenance required by the current runner"
					} else {
						"Readiness gates and bounded connection draining"
					})
					.class("text-xs font-semibold"),
				),
		)
		.child(
			View()
				.class(CARD)
				.child(Heading(2, "Release history").class("text-lg font-semibold"))
				.child(history(
					"release-12",
					"Current",
					"a1b2c3d · Interface improvements",
				))
				.child(history(
					"release-11",
					"Previous",
					"8e7f6a5 · Search improvements",
				))
				.child(history(
					"release-10",
					"Retained",
					"4d3c2b1 · First production release",
				)),
		)
		.children(model.request.as_ref().map(|request| {
			View()
				.class(CARD)
				.test_id("request-preview")
				.child(Heading(2, "Agent request preview").class("text-lg font-semibold"))
				.child(
					Text(
						"Proposed contract only. A real server must authorize, plan, and validate \
						 this request again.",
					)
					.class(QUIET),
				)
				.child(
					View().class("gap-1").children(
						serde_json::to_string_pretty(request)
							.expect("serializable request")
							.lines()
							.map(|line| {
								Text(line.trim()).class(if line.starts_with(' ') {
									"pl-4 text-xs"
								} else {
									"text-xs"
								})
							}),
					),
				)
		}))
}

fn history(release: &str, state: &str, detail: &str) -> Element {
	View()
		.class(
			"flex-row flex-wrap items-center gap-3 border-t border-slate-200 py-3 \
			 dark:border-slate-800",
		)
		.child(Text(release).class("text-sm font-semibold"))
		.child(Text(state).class("text-xs text-slate-500 dark:text-slate-400"))
		.child(View().class("flex-1"))
		.child(Text(detail).class("text-xs text-slate-500 dark:text-slate-400"))
}

fn infrastructure(model: &Model) -> Element {
	View()
		.class("gap-5")
		.child(
			View()
				.class(CARD)
				.child(Heading(2, "Choose an operating profile").class("text-lg font-semibold"))
				.child(
					Text("Start small. Change the target when the workload needs it.").class(QUIET),
				)
				.child(
					View().class("flex-row flex-wrap gap-2").children(
						[Target::SingleVm, Target::RedundantVm, Target::Kubernetes]
							.into_iter()
							.map(|target| {
								let id = match target {
									Target::SingleVm => "target-small",
									Target::RedundantVm => "target-multiple",
									Target::Kubernetes => "target-kubernetes",
								};
								Button(target.title())
									.class(if target == model.target {
										SELECTED
									} else {
										BUTTON
									})
									.test_id(id)
									.aria_pressed(target == model.target)
									.on_press(move || store::update(|m| m.target(target)))
							}),
					),
				)
				.child(Text(model.target.description()).class(QUIET)),
		)
		.child(
			View()
				.class(CARD)
				.child(Heading(2, "What LeptosKit manages").class("text-lg font-semibold"))
				.child(history(
					"Edge",
					"Cloudflare",
					"TLS and private origin access",
				))
				.child(history(
					"Application",
					"Rust container",
					"Website + API + persistent live services",
				))
				.child(history(
					"Database",
					"Postgres",
					"Migration history + verified backups",
				))
				.child(history(
					"Object storage",
					"R2 / B2",
					"Signed updates + private backups",
				))
				.child(history(
					"Optional services",
					"NATS / Valkey",
					"Shared relay / disposable cache",
				)),
		)
		.child(
			View()
				.class(CARD)
				.child(Heading(2, "A budget is a policy").class("text-lg font-semibold"))
				.child(
					Text(
						"Keep a small app within a chosen monthly target. Show compute, \
						 control-plane, build, storage, and traffic costs separately; mark \
						 unknown charges.",
					)
					.class(QUIET),
				)
				.child(
					Text("No live provider quote is loaded in this prototype.")
						.class("text-sm font-semibold"),
				),
		)
		.child(
			View()
				.class(CARD)
				.child(
					Heading(2, "Autoscaling belongs to the runtime").class("text-lg font-semibold"),
				)
				.child(
					Text(
						"For Kubernetes: minimum 2, maximum 6 app replicas is an illustrative \
						 policy, not an applied setting. HPA requires metrics; node autoscaling \
						 and database capacity are separate. A replica cap does not cap the cloud \
						 bill.",
					)
					.class(QUIET),
				),
		)
}

fn agents(model: &Model) -> Element {
	View()
		.class("gap-5")
		.child(
			View()
				.class(CARD)
				.child(Heading(2, "Your deployment control plane").class("text-lg font-semibold"))
				.child(
					Text(
						"The same API and console whether LeptosKit runs it or you host it. \
						 Hosting modes below are a proposal, not an available cloud service.",
					)
					.class(QUIET),
				)
				.child(
					View()
						.class("flex-row gap-2")
						.child(
							Button("Hosted")
								.test_id("hosting-managed")
								.class(if model.self_hosted { BUTTON } else { SELECTED })
								.on_press(|| store::update(|m| m.self_hosted = false)),
						)
						.child(
							Button("Self-hosted")
								.test_id("hosting-self")
								.class(if model.self_hosted { SELECTED } else { BUTTON })
								.on_press(|| store::update(|m| m.self_hosted = true)),
						),
				)
				.child(
					Text(if model.self_hosted {
						"You operate the control-plane service and its backups. Customer \
						 infrastructure can keep running if its console is unavailable."
					} else {
						"A hosted control plane spreads its operating cost across projects. \
						 Application compute stays in the customer's chosen account."
					})
					.class(QUIET),
				),
		)
		.child(
			View()
				.class(CARD)
				.child(Heading(2, "Connect once, deploy by intent").class("text-lg font-semibold"))
				.child(Text("Proposed connection settings").class(QUIET))
				.child(
					Text(
						"URL: https://deploy.example.test\nToken: supplied by your secret store, \
						 never TOML",
					)
					.class("text-sm"),
				)
				.child(
					Text(
						"An agent requests a deployment, receives an operation ID and plan, and \
						 follows structured events. It does not automate dashboard clicks.",
					)
					.class(QUIET),
				),
		)
		.child(
			View()
				.class(CARD)
				.child(
					Heading(2, "One policy for people and agents").class("text-lg font-semibold"),
				)
				.child(
					Text(
						"Project/environment-scoped tokens. Short-lived runner credentials. \
						 Idempotent requests. Revision checks. Cost limits. Explicit data-change \
						 approval. Durable audit events.",
					)
					.class(QUIET),
				)
				.child(
					Text(
						"No token field is enabled here. Authentication and remote execution are \
						 not implemented by this prototype.",
					)
					.class("text-sm font-semibold"),
				),
		)
}
