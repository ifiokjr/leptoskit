use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use super::*;

/// A fixed operation adapter. The service resolves the registered repository;
/// implementations must never interpret remote input as a shell command.
pub trait Executor: Send + Sync + 'static {
	fn run(
		&self,
		project: Project,
		operation: Operation,
	) -> Pin<Box<dyn Future<Output = RunResult> + Send>>;
}

pub struct RunResult {
	pub succeeded: bool,
	pub message: String,
	pub details: serde_json::Value,
}

pub(crate) async fn work(
	store: Store,
	executor: Arc<dyn Executor>,
	wake: Arc<tokio::sync::Notify>,
) {
	let mut maintenance = std::time::Instant::now();
	if store.maintain().is_err() {
		return;
	}
	loop {
		if maintenance.elapsed().as_secs() >= 60 {
			if store.maintain().is_err() {
				return;
			}
			maintenance = std::time::Instant::now();
		}
		let next = store.claim();
		match next {
			Ok(Some(operation)) => {
				let project = store
					.projects()
					.ok()
					.and_then(|projects| projects.into_iter().find(|p| p.id == operation.project));
				let Some(project) = project else {
					return;
				};
				let result = executor.run(project, operation.clone()).await;
				if store.finish(&operation.id, &result).is_err() {
					return;
				}
			}
			Ok(None) => {
				tokio::select! { ()=wake.notified()=>{}, ()=tokio::time::sleep(std::time::Duration::from_secs(1))=>{} }
			}
			Err(_) => return,
		}
	}
}
