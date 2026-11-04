use leptos::prelude::RenderHtml as _;
use leptoskit::prelude::*;

use super::*;

const CARD: &str = "gap-3 rounded-xl border border-slate-200 p-4 dark:border-slate-800";
const QUIET: &str = "text-sm text-slate-500 dark:text-slate-400";
const BUTTON: &str = "rounded-lg border border-slate-300 px-3 py-2 text-sm dark:border-slate-700";

pub(crate) fn html(overview: Option<&Overview>, store: &Store) -> String {
	let body = leptoskit::web::render(screen(overview, store)).to_html();
	format!(
		r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>LeptosKit Deploy</title><style>body{{margin:0;font-family:system-ui;background:#f8fafc;color:#0f172a}}main{{max-width:1100px;margin:auto;padding:24px}}button{{cursor:pointer}}button[aria-pressed=true]{{background:#e2e8f0;border-color:#64748b}}input{{min-height:40px}}p{{overflow-wrap:anywhere}}@media(prefers-color-scheme:dark){{body{{background:#020617;color:#f1f5f9}}button[aria-pressed=true]{{background:#1e293b;border-color:#94a3b8}}}}</style><script type="module" src="/console.mjs"></script></head><body><main>{body}</main></body></html>"#
	)
}

fn screen(overview: Option<&Overview>, store: &Store) -> Element {
	let header = View()
		.class(
			"flex-row flex-wrap items-center gap-3 border-b border-slate-200 pb-4 \
			 dark:border-slate-800",
		)
		.child(Text("◈ leptoskit").class("text-lg font-semibold"))
		.child(Text("Deploy").class(QUIET))
		.child(View().class("flex-1"))
		.children(overview.map(|_| Button("Sign out").class(BUTTON).test_id("logout")));
	let mut page = View()
		.class("gap-5 text-slate-900 dark:text-slate-100")
		.child(header)
		.child(Heading(1, "Your applications").class("text-3xl font-semibold"))
		.child(Text("Deployment history and application measurements in one place.").class(QUIET))
		.child(
			Text("")
				.test_id("notice")
				.class("text-sm text-amber-700 dark:text-amber-300"),
		);
	let Some(overview) = overview else {
		return page.child(
			View()
				.class(CARD)
				.child(
					Heading(2, "Connect to your deployment service").class("text-xl font-semibold"),
				)
				.child(
					Text(
						"Enter the service owner's key. This browser receives a temporary session \
						 cookie.",
					)
					.class(QUIET),
				)
				.child(
					TextInput()
						.placeholder("Owner key")
						.aria_label("Owner key")
						.test_id("owner-key")
						.class("rounded-lg border border-slate-300 p-3 dark:border-slate-700"),
				)
				.child(Button("Sign in").class(BUTTON).test_id("login")),
		);
	};
	if overview.projects.is_empty() {
		page = page.child(
			Text(
				"No projects registered. Start the service with --project pointing to your \
				 application repository.",
			)
			.class(QUIET),
		);
	}
	for project in &overview.projects {
		let operations: Vec<_> = overview
			.operations
			.iter()
			.filter(|o| o.project == project.id)
			.collect();
		let fleet = store
			.fleet(&project.id, now().saturating_sub(3600), now())
			.ok();
		page = page.child(project_card(project, &operations, fleet.as_ref()));
	}
	page
}

fn project_card(project: &Project, operations: &[&Operation], fleet: Option<&Fleet>) -> Element {
	let mut card = View()
		.class(CARD)
		.test_id(format!("project-{}", project.id))
		.child(Heading(2, &project.id).class("text-xl font-semibold"))
		.child(Text(&project.environment).class(QUIET));
	let busy = operations
		.iter()
		.any(|o| matches!(o.phase, Phase::Queued | Phase::Running));
	card = card.child(
		View()
			.class("flex-row flex-wrap gap-2")
			.child(
				Button("Plan infrastructure")
					.test_id("plan")
					.class(BUTTON)
					.enabled(!busy),
			)
			.child(
				Button("Check health")
					.test_id("status")
					.class(BUTTON)
					.enabled(!busy),
			)
			.child(
				Button("Verify backup")
					.test_id("backup")
					.class(BUTTON)
					.enabled(!busy),
			),
	);
	card = card.child(
		View()
			.class("gap-2")
			.child(
				TextInput()
					.placeholder("Image tag or Git commit")
					.aria_label("Image tag or Git commit")
					.test_id("version")
					.class("rounded-lg border border-slate-300 p-3 dark:border-slate-700"),
			)
			.child(
				View()
					.class("flex-row flex-wrap gap-2")
					.child(
						Button("Deploy")
							.test_id("deploy")
							.class(BUTTON)
							.enabled(!busy),
					)
					.child(
						Button("Roll back image")
							.test_id("rollback")
							.class(BUTTON)
							.enabled(!busy),
					)
					.child(
						Button("First release")
							.test_id("setup")
							.aria_pressed(false)
							.class(BUTTON),
					)
					.child(
						Button("Maintenance")
							.test_id("maintenance")
							.aria_pressed(false)
							.class(BUTTON),
					),
			),
	);
	if let Some(plan) = operations
		.iter()
		.find(|o| matches!(o.intent.action, Action::Plan))
		&& plan.phase == Phase::Succeeded
	{
		card = card.child(
			View()
				.class("gap-2 rounded-lg bg-slate-100 p-3 dark:bg-slate-900")
				.child(Text("Infrastructure plan").class("font-semibold"))
				.children(
					plan.result
						.get("changes")
						.and_then(serde_json::Value::as_array)
						.into_iter()
						.flatten()
						.map(|change| {
							Text(format!(
								"{} · {}",
								change["address"].as_str().unwrap_or("resource"),
								change["actions"]
									.as_array()
									.map_or_else(String::new, |actions| {
										actions
											.iter()
											.filter_map(serde_json::Value::as_str)
											.collect::<Vec<_>>()
											.join(", ")
									})
							))
							.class(QUIET)
						}),
				)
				.child(
					Button("Apply this plan")
						.test_id(format!("apply-{}", plan.id))
						.class(BUTTON)
						.enabled(!busy),
				),
		);
	}
	card = card.child(measurements(fleet));
	card = card.child(Heading(3, "Recent operations").class("font-semibold"));
	for operation in operations.iter().take(10) {
		card = card.child(
			View()
				.class("gap-1 border-t border-slate-200 pt-3 dark:border-slate-800")
				.child(
					Text(format!(
						"{} · {:?}",
						action_name(&operation.intent.action),
						operation.phase
					))
					.class("text-sm font-semibold"),
				)
				.child(Text(&operation.message).class(QUIET))
				.child(
					Text(format!(
						"{} · operation {}",
						age(operation.updated),
						operation.id.get(..8).unwrap_or(&operation.id)
					))
					.class("text-xs text-slate-500 dark:text-slate-400"),
				),
		);
	}
	card
}

fn action_name(action: &Action) -> String {
	match action {
		Action::Deploy { version, .. } => format!("Deploy {version}"),
		Action::Rollback { version } => format!("Rollback to {version}"),
		Action::Plan => "Plan infrastructure".into(),
		Action::Apply { .. } => "Apply infrastructure".into(),
		Action::Status => "Health check".into(),
		Action::Backup => "Backup verification".into(),
	}
}
fn age(at: u64) -> String {
	let seconds = now().saturating_sub(at);
	if seconds < 60 {
		"Just now".into()
	} else if seconds < 3600 {
		format!("{}m ago", seconds / 60)
	} else {
		format!("{}h ago", seconds / 3600)
	}
}

fn measurements(fleet: Option<&Fleet>) -> Element {
	let mut card = View()
		.class("gap-2")
		.child(Heading(3, "Last hour").class("font-semibold"));
	if let Some(fleet) = fleet
		&& fleet.instances > 0
	{
		card = card.child(
			Text(format!(
				"{} reporting instances · {} stale",
				fleet.instances, fleet.stale_instances
			))
			.class(QUIET),
		);
		for report in &fleet.hosts {
			if let Some(host) = &report.host {
				let memory = host
					.cgroup_memory_bytes
					.or(host.process_resident_bytes)
					.map_or_else(
						|| "unavailable".into(),
						|n| format!("{:.1} MiB", n as f64 / 1_048_576.0),
					);
				let disk = host.filesystem_available_bytes.map_or_else(
					|| "unavailable".into(),
					|n| format!("{:.1} GiB", n as f64 / 1_073_741_824.0),
				);
				card = card.child(
					Text(format!(
						"{} · memory {} · filesystem available {}",
						report.instance, memory, disk
					))
					.class(QUIET),
				);
			}
		}
		for series in fleet.series.iter().take(20) {
			let count = series.windows.iter().map(|w| w.count).sum::<u64>();
			let duration = series
				.windows
				.iter()
				.map(|w| w.duration_sum_ms)
				.sum::<f64>();
			card = card.child(
				Text(format!(
					"{:?} · {:?} · {:?} · {} · {} samples · {:.1} ms mean · {}",
					series.key.source,
					series.key.signal,
					series.key.outcome,
					series.key.operation,
					count,
					duration / count.max(1) as f64,
					series.release
				))
				.class(
					if matches!(
						series.key.outcome,
						leptoskit::analytics::Outcome::ServerError
					) {
						"text-sm text-red-600 dark:text-red-400"
					} else {
						QUIET
					},
				),
			);
		}
	} else {
		card = card.child(
			Text("Waiting for measurements. Connect the app's analytics exporter to this service.")
				.class(QUIET),
		);
	}

	card
}
