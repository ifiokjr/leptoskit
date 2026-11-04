use leptoskit::prelude::*;
use leptoskit_deploy_console::hosted_protocol::Installation;
use leptoskit_deploy_console::hosted_protocol::Ownership;
use reactive_graph::prelude::*;
use serde_json::json;

use crate::Shell;
const BUTTON: &str = "rounded-lg border border-slate-300 px-3 py-2 text-sm dark:border-slate-700";
const CARD: &str = "gap-3 rounded-xl border border-slate-200 p-4 dark:border-slate-800";
const QUIET: &str = "text-sm text-slate-500 dark:text-slate-400";

pub fn screen(shell: Shell) -> Element {
	let mut page = ScrollView()
		.class("p-4 gap-5 bg-white text-slate-900 dark:bg-slate-950 dark:text-slate-100")
		.child(crate::motion::mark().id("app-mark"))
		.child(Heading(1, "LeptosKit").class("text-3xl font-semibold"))
		.child(Text("Your applications. Your choice of cloud.").class(QUIET))
		.child(
			Pressable("Documentation")
				.href("/docs/")
				.test_id("documentation")
				.class("px-3 py-2 rounded-lg border border-slate-300 dark:border-slate-700")
				.child(Text("Documentation ↗"))
				.on_press(move || {
					any_spawner::Executor::spawn_local(async move {
						if let Err(error) = crate::client::open_documentation().await {
							shell.notice.set(error);
						}
					});
				}),
		);
	let Some(account) = shell.account.get() else {
		return page
			.child(
				View()
					.class(CARD)
					.child(Heading(2, "Connect your account").class("text-xl font-semibold"))
					.child(
						Text(
							"Use the account key from your LeptosKit invitation. Your cloud \
							 credentials stay on your dedicated worker.",
						)
						.class(QUIET),
					)
					.child(
						PasswordInput()
							.placeholder("Account key")
							.accessibility_label("Account key")
							.test_id("account-key")
							.on_change(move |value| shell.key.set(value.into())),
					)
					.child(
						Button("Connect")
							.class(BUTTON)
							.test_id("connect")
							.enabled(!shell.busy.get())
							.on_press(move || shell.request("/api/v1/account".into(), None)),
					),
			)
			.child(Text(shell.notice.get()).class(QUIET));
	};
	page = page
		.child(
			View()
				.class("flex-row flex-wrap gap-2")
				.child(Text(format!(
					"{} · {} · {}",
					account.id, account.plan, account.status
				)))
				.child(
					Button("Refresh")
						.class(BUTTON)
						.test_id("refresh")
						.on_press(move || shell.request("/api/v1/overview".into(), None)),
				)
				.child(
					Button("Sign out")
						.class(BUTTON)
						.test_id("logout")
						.on_press(move || shell.logout()),
				),
		)
		.child(billing(shell));
	for installation in account.installations {
		page = page.child(installation_card(shell, &installation));
	}
	page.child(View().class(CARD)
        .child(Heading(2,"Connect customer-owned infrastructure").class("text-xl font-semibold"))
        .child(TextInput().placeholder("Project ID from infra.name").accessibility_label("Project ID").test_id("project").on_change(move|value|shell.project.set(value.into())))
        .child(Button("Add installation").class(BUTTON).test_id("add-installation").on_press(move||shell.request("/api/v1/installations".into(),Some(json!({"project":shell.project.get_untracked(),"environment":"production","ownership":"customer"})))))
        .child(Text("Need LeptosKit-managed DigitalOcean capacity? Ask for a dedicated installation and an infrastructure quote.").class(QUIET)))
        .child(Text(shell.notice.get()).class("text-sm text-blue-700 dark:text-blue-300")).child(output_controls(shell))
}
fn output_controls(shell: Shell) -> Element {
	View()
		.class("flex-row flex-wrap gap-2")
		.children((!shell.output.get().is_empty()).then(|| {
			Button("Copy key, code or billing link")
				.class(BUTTON)
				.on_press(move || {
					any_spawner::Executor::spawn_local(async move {
						shell.notice.set(
							match crate::client::copy(&shell.output.get_untracked()).await {
								Ok(()) => "Copied. Keep credentials private.".into(),
								Err(message) => message,
							},
						);
					});
				})
		}))
}

fn billing(shell: Shell) -> Element {
	View()
		.class(CARD)
		.child(Heading(2, "Subscription").class("text-xl font-semibold"))
		.child(
			Text(
				"The management subscription and cloud resource usage are billed separately. Your \
				 checkout shows the current price.",
			)
			.class(QUIET),
		)
		.child(
			View()
				.class("flex-row flex-wrap gap-2")
				.child(Button("Control plan").class(BUTTON).on_press(move || {
					shell.request(
						"/api/v1/billing/checkout".into(),
						Some(json!({"plan":"control"})),
					);
				}))
				.child(Button("Team plan").class(BUTTON).on_press(move || {
					shell.request(
						"/api/v1/billing/checkout".into(),
						Some(json!({"plan":"team"})),
					);
				}))
				.child(Button("Manage billing").class(BUTTON).on_press(move || {
					shell.request("/api/v1/billing/portal".into(), Some(json!({})));
				})),
		)
}
fn installation_card(shell: Shell, i: &Installation) -> Element {
	let project = i.project.clone();
	let pair_project = project.clone();
	let revoke_project = project.clone();
	let deploy_project = project.clone();
	let rollback_project = project.clone();
	let plan_project = project.clone();
	let read_project = project.clone();
	let token_project = project.clone();
	let metrics_project = project.clone();
	let mut card = View()
		.class(CARD)
		.child(Heading(2, &project).class("text-xl font-semibold"))
		.child(
			Text(format!(
				"{} · {}",
				i.environment,
				match i.ownership {
					Ownership::Customer => "Your cloud account",
					Ownership::LeptoskitDigitalocean => "LeptosKit-managed DigitalOcean",
				}
			))
			.class(QUIET),
		)
		.child(
			Text(if i.last_seen == 0 {
				"Worker has not connected yet"
			} else {
				"Worker has connected; refresh for deployment status"
			})
			.class(QUIET),
		);
	if matches!(i.ownership, Ownership::Customer) {
		card = card.child(
			View()
				.class("flex-row flex-wrap gap-2")
				.child(Button("Pair worker").class(BUTTON).on_press(move || {
					shell.request(
						format!("/api/v1/installations/{pair_project}/pair"),
						Some(json!({})),
					);
				}))
				.child(Button("Revoke worker").class(BUTTON).on_press(move || {
					shell.request(
						format!("/api/v1/installations/{revoke_project}/revoke"),
						Some(json!({})),
					);
				})),
		);
	}
	card=card.child(TextInput().placeholder("Image tag or Git commit").accessibility_label("Release version").on_change(move|v|shell.version.set(v.into())))
        .child(View().class("flex-row items-center gap-2").child(Checkbox(shell.setup.get()).accessibility_label("First release").on_toggle(move|value|shell.setup.set(value))).child(Text("First release")).child(Checkbox(shell.maintenance.get()).accessibility_label("Maintenance").on_toggle(move|value|shell.maintenance.set(value))).child(Text("Maintenance")))
        .child(View().class("flex-row flex-wrap gap-2")
        .child(Button("Plan infrastructure").class(BUTTON).on_press(move||shell.operation(&plan_project,&json!({"kind":"plan"}))))
        .child(Button("Deploy").class(BUTTON).on_press(move||shell.operation(&deploy_project,&json!({"kind":"deploy","version":shell.version.get_untracked(),"setup":shell.setup.get_untracked(),"maintenance":shell.maintenance.get_untracked()}))))
        .child(Button("Roll back image").class(BUTTON).on_press(move||shell.operation(&rollback_project,&json!({"kind":"rollback","version":shell.version.get_untracked()}))))
        .child(Button("Application measurements").class(BUTTON).on_press(move||shell.request(format!("/api/v1/projects/{metrics_project}/analytics"),None)))
        .child(Button("Create agent key").class(BUTTON).on_press(move||shell.request("/api/v1/tokens".into(),Some(json!({"project":token_project,"scope":"deploy"}))))));
	let overview = shell.overview.get();
	let operations = overview["operations"]
		.as_array()
		.map(Vec::as_slice)
		.unwrap_or_default();
	if let Some(plan) = operations
		.iter()
		.find(|o| o["project"] == project && o["intent"]["action"]["kind"] == "plan")
		&& plan["phase"] == "succeeded"
	{
		let id = plan["id"].as_str().unwrap_or_default().to_owned();
		let actions = plan["result"]["changes"].to_string();
		card = card
			.child(Text(format!("Resource actions: {actions}")).class(QUIET))
			.child(
				Button("Apply reviewed plan")
					.class(BUTTON)
					.on_press(move || {
						shell.operation(&read_project, &json!({"kind":"apply","plan_id":id}));
					}),
			);
	}
	for operation in operations
		.iter()
		.filter(|o| o["project"] == project)
		.take(5)
	{
		card = card.child(
			Text(format!(
				"{} · {}",
				operation["phase"].as_str().unwrap_or_default(),
				operation["message"].as_str().unwrap_or_default()
			))
			.class(QUIET),
		);
	}
	card.children((shell.measured_project.get() == project).then(|| measurements(shell)))
}

fn measurements(shell: Shell) -> Element {
	let value = shell.measurement.get();
	let mut view = View().class("gap-2");
	if value.is_null() {
		return view;
	}
	view = view.child(
		Text(format!(
			"Reporting instances: {} · stale: {}",
			value["instances"], value["stale_instances"]
		))
		.class(QUIET),
	);
	for series in value["series"]
		.as_array()
		.map(Vec::as_slice)
		.unwrap_or_default()
		.iter()
		.take(12)
	{
		let count: u64 = series["windows"]
			.as_array()
			.map(Vec::as_slice)
			.unwrap_or_default()
			.iter()
			.filter_map(|w| w["count"].as_u64())
			.fold(0u64, u64::saturating_add);
		view = view.child(
			Text(format!(
				"{} · {} · {} · {} samples · {}",
				series["key"]["source"].as_str().unwrap_or_default(),
				series["key"]["operation"].as_str().unwrap_or_default(),
				series["key"]["outcome"].as_str().unwrap_or_default(),
				count,
				series["release"].as_str().unwrap_or_default()
			))
			.class(QUIET),
		);
	}
	view.child(
		Text(
			"Reporting instances may not cover the whole deployment. Browser observations are \
			 separate from backend measurements.",
		)
		.class(QUIET),
	)
}
