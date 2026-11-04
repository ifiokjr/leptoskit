//! One outbound agent per installation; uncertain executions require
//! reconciliation.
use leptoskit_deploy_console::control::Executor;
use leptoskit_deploy_console::control::RunResult;
use leptoskit_deploy_console::hosted::Job;
use serde_json::Value;
use serde_json::json;

use super::*;

pub fn run(url: &str, root: &Path, data: &Path) -> Result<()> {
	let url = client::origin(url)?;
	let root = root.canonicalize()?;
	let config = LeptoskitConfig::load_from(&root)?;
	let infra = config.infra.context("worker needs [infra]")?;
	ensure!(
		infra.hosted_state,
		"set infra.hosted_state=true for the hosted worker; migrate existing state explicitly \
		 first"
	);
	let _lock = directory_lock(data)?;
	let credential = data.join("worker.json");
	let key = if credential.exists() {
		#[cfg(unix)]
		{
			use std::os::unix::fs::PermissionsExt as _;
			ensure!(
				std::fs::metadata(&credential)?
					.permissions()
					.mode()
					.trailing_zeros()
					>= 6,
				"worker credential must be private"
			);
		}
		let value: Value = serde_json::from_slice(&std::fs::read(&credential)?)?;
		ensure!(
			value["url"] == url && value["project"] == infra.name,
			"worker credential is bound to another installation"
		);
		check_managed_policy(&infra, &value["ownership"])?;
		value["key"]
			.as_str()
			.context("invalid worker credential")?
			.to_owned()
	} else {
		let code = std::env::var("LEPTOSKIT_PAIRING_CODE")
			.context("set LEPTOSKIT_PAIRING_CODE from the app's one-use pairing grant")?;
		let value = request(&url, "/api/v1/worker/pair", None, &json!({"code":code}))?;
		ensure!(
			value["installation"]["project"] == infra.name,
			"paired project differs from registered repository"
		);
		check_managed_policy(&infra, &value["installation"]["ownership"])?;
		let key = value["key"]
			.as_str()
			.context("invalid worker key")?
			.to_owned();
		client::private_write(
			&credential,
			&serde_json::to_vec(
				&json!({"url":url,"project":infra.name,"key":key,"ownership":value["installation"]["ownership"]}),
			)?,
		)?;
		key
	};
	let store = Store::open(&data.join("local.sqlite3"))?;
	let project = Project {
		id: infra.name.clone(),
		environment: infra
			.plugins
			.analytics
			.as_ref()
			.map_or_else(|| "production".into(), |a| a.environment.clone()),
		root,
		revision: 0,
	};
	store.register(&project)?;
	let local_key = store.issue(Scope::Owner, None, 86400, false)?;
	let service = Service::new(store, &local_key, &url)?;
	let runtime = tokio::runtime::Builder::new_multi_thread()
		.worker_threads(2)
		.enable_all()
		.build()?;
	println!("Dedicated worker connected for {}", infra.name);
	runtime.block_on(work(&url, &key, data, project, service))
}
fn request(url: &str, path: &str, key: Option<&str>, body: &Value) -> Result<Value> {
	let agent: ureq::Agent = ureq::Agent::config_builder()
		.timeout_global(Some(std::time::Duration::from_secs(20)))
		.max_redirects(0)
		.http_status_as_error(false)
		.build()
		.into();
	let mut request = agent.post(format!("{url}{path}"));
	if let Some(key) = key {
		request = request.header("Authorization", format!("Bearer {key}"));
	}
	let mut response = request
		.send_json(body)
		.map_err(|_| anyhow::anyhow!("hosted connection failed"))?;
	ensure!(
		response.status().is_success(),
		"hosted worker request rejected; check account, pairing and subscription"
	);
	let text = response
		.body_mut()
		.with_config()
		.limit(1024 * 1024)
		.read_to_string()?;
	if text.is_empty() {
		Ok(Value::Null)
	} else {
		Ok(serde_json::from_str(&text)?)
	}
}
async fn work(url: &str, key: &str, data: &Path, project: Project, service: Service) -> Result<()> {
	let pending = data.join("pending.json");
	loop {
		if pending.exists() {
			let saved: Value = serde_json::from_slice(&std::fs::read(&pending)?)?;
			let result = saved.get("result").context(
				"worker stopped during execution; reconcile its pending operation on the hosted \
				 app and remote release ledger before removing pending.json",
			)?;
			if request(url, "/api/v1/worker/finish", Some(key), result).is_ok() {
				std::fs::remove_file(&pending)?;
			} else {
				tokio::time::sleep(std::time::Duration::from_secs(5)).await;
				continue;
			}
		}
		let Ok(value) = request(url, "/api/v1/worker/claim", Some(key), &json!({})) else {
			tokio::time::sleep(std::time::Duration::from_secs(5)).await;
			continue;
		};
		if value.is_null() {
			tokio::time::sleep(std::time::Duration::from_secs(2)).await;
		} else {
			let job: Job = serde_json::from_value(value)?;
			ensure!(
				job.operation.project == project.id,
				"hosted operation addressed a different project"
			);
			client::private_write(
				&pending,
				&serde_json::to_vec(&json!({"operation":job.operation.id}))?,
			)?;
			let mut runner =
				runner::CommandRunner::new(data.into(), std::env::current_exe()?, service.clone());
			runner.hosted_bindings = Some((job.analytics_read_key, job.analytics_ingest_key));
			// Only the fixed engine commands inherit these state bindings. The
			// hosted key is scoped to this installation.
			let address = format!("{url}/api/v1/state/{}", project.id);
			let result = with_state(
				&mut runner,
				project.clone(),
				job.operation.clone(),
				&address,
				key,
			)
			.await;
			let result = json!({"id":job.operation.id,"succeeded":result.succeeded,"message":result.message,"details":result.details});
			client::private_write(
				&pending,
				&serde_json::to_vec(&json!({"operation":job.operation.id,"result":result}))?,
			)?;
		}
	}
}
async fn with_state(
	runner: &mut runner::CommandRunner,
	project: Project,
	operation: leptoskit_deploy_console::control::Operation,
	address: &str,
	key: &str,
) -> RunResult {
	runner.hosted_state = Some((address.into(), key.into()));
	runner.run(project, operation).await
}

fn check_managed_policy(
	infra: &leptoskit_config::infra::InfraConfig,
	ownership: &Value,
) -> Result<()> {
	if ownership == "leptoskit_digitalocean" {
		ensure!(
			matches!(
				infra.compute,
				leptoskit_config::infra::ComputeProvider::Digitalocean
			) && infra.budget.is_some(),
			"managed capacity requires DigitalOcean and an explicit reviewed compute budget"
		);
	}
	Ok(())
}
