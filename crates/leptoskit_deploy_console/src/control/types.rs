use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;

/// A repository explicitly registered by the service owner. API callers
/// cannot supply a path, shell command, remote URL or environment variable.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Project {
	pub id: String,
	pub environment: String,
	#[serde(skip)]
	pub root: PathBuf,
	pub revision: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
	Read,
	Deploy,
	Ingest,
	Owner,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, tag = "kind", rename_all = "snake_case")]
pub enum Action {
	Plan,
	Apply {
		plan_id: String,
	},
	Deploy {
		version: String,
		#[serde(default)]
		setup: bool,
		#[serde(default)]
		maintenance: bool,
	},
	Rollback {
		version: String,
	},
	Status,
	Backup,
}

impl Action {
	#[must_use]
	pub fn valid(&self) -> bool {
		let tag = |s: &str| {
			!s.is_empty()
				&& s.len() <= 128
				&& s.bytes()
					.all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
		};
		match self {
			Self::Apply { plan_id } => super::identifier(plan_id),
			Self::Deploy { version, .. } | Self::Rollback { version } => {
				tag(version) && !version.starts_with('-')
			}
			Self::Plan | Self::Status | Self::Backup => true,
		}
	}
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Intent {
	pub expected_revision: u64,
	pub idempotency_key: String,
	pub action: Action,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
	Queued,
	Running,
	Succeeded,
	Failed,
	Interrupted,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Operation {
	#[serde(default)]
	pub events: Vec<Event>,
	pub id: String,
	pub project: String,
	pub revision: u64,
	pub intent: Intent,
	pub phase: Phase,
	pub created: u64,
	pub updated: u64,
	pub message: String,
	pub result: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Event {
	pub sequence: u64,
	pub phase: Phase,
	pub message: String,
	pub created: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FleetSeries {
	pub release: String,
	pub key: leptoskit::analytics::SeriesKey,
	pub windows: Vec<leptoskit::analytics::Window>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Fleet {
	pub hosts: Vec<HostReport>,
	pub retained_from: Option<u64>,
	pub from: u64,
	pub to: u64,
	pub instances: usize,
	pub stale_instances: usize,
	pub series: Vec<FleetSeries>,
	pub limitations: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HostReport {
	pub instance: String,
	pub release: String,
	pub seen: u64,
	pub host: Option<leptoskit::analytics::HostSample>,
	pub export: Option<leptoskit::analytics::ExportHealth>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Overview {
	pub format_version: u32,
	pub projects: Vec<Project>,
	pub operations: Vec<Operation>,
}

#[derive(Clone)]
pub(crate) struct Principal {
	pub scope: Scope,
	pub project: Option<String>,
	pub session: bool,
}

impl Principal {
	pub fn allows(&self, project: &str, scope: Scope) -> bool {
		self.project
			.as_ref()
			.is_none_or(|allowed| allowed == project)
			&& (self.scope == Scope::Owner
				|| self.scope == scope
				|| (self.scope == Scope::Deploy && scope == Scope::Read))
	}
}
