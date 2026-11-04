//! Shared presentation model for the proposed human and agent workflow.

use serde::Deserialize;
use serde::Serialize;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
	#[default]
	Deployments,
	Infrastructure,
	Agents,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Target {
	#[default]
	SingleVm,
	RedundantVm,
	Kubernetes,
}

impl Target {
	#[must_use]
	pub const fn title(self) -> &'static str {
		match self {
			Self::SingleVm => "Small app",
			Self::RedundantVm => "Multiple hosts",
			Self::Kubernetes => "Kubernetes",
		}
	}

	#[must_use]
	pub const fn description(self) -> &'static str {
		match self {
			Self::SingleVm => {
				"One VM. Lowest resource count. Overlapping app releases need memory headroom; \
				 host and database failures still interrupt service."
			}
			Self::RedundantVm => {
				"Two app hosts. Replace one at a time. Shared Postgres and NATS still need a \
				 separate availability plan."
			}
			Self::Kubernetes => {
				"Proposed cluster adapter. Delegate replicas to Kubernetes HPA and node capacity \
				 to a node autoscaler. Neither is connected in this prototype."
			}
		}
	}
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SchemaChange {
	#[default]
	Unchanged,
	ExpandUnverified,
	Reverse,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Intent {
	Deploy,
	Rollback,
}

/// A command envelope for review. No bearer token or provider secret belongs
/// in this object. The future API must independently authorize and validate it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
	pub project: String,
	pub environment: String,
	pub expected_revision: u64,
	pub idempotency_key: String,
	pub intent: Intent,
	pub release: String,
	pub target: Target,
	pub schema_change: SchemaChange,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Review {
	pub title: &'static str,
	pub detail: &'static str,
	pub maintenance: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Model {
	pub tab: Tab,
	pub target: Target,
	pub schema: SchemaChange,
	pub self_hosted: bool,
	pub request: Option<Request>,
	sequence: u64,
}

impl Model {
	#[must_use]
	pub const fn new(target: Target, tab: Tab, schema: SchemaChange) -> Self {
		Self {
			target,
			tab,
			schema,
			self_hosted: false,
			request: None,
			sequence: 0,
		}
	}

	#[must_use]
	pub fn review(&self, intent: Intent) -> Review {
		if self.target == Target::Kubernetes {
			return Review {
				title: "Cluster adapter required",
				detail: "Kubernetes execution and autoscaling are proposed. This prototype cannot \
				         connect to or change a cluster.",
				maintenance: false,
			};
		}
		match (intent, self.schema) {
			(_, SchemaChange::Reverse) => {
				Review {
					title: "Maintenance and recovery review",
					detail: "Stop writers, verify a backup, review down migrations, and obtain an \
					         explicit data-change approval. Image rollback never silently \
					         reverses the database.",
					maintenance: true,
				}
			}
			(_, SchemaChange::ExpandUnverified) => {
				Review {
					title: "Compatibility proof required",
					detail: "Additive SQL alone does not prove the previous binary can run. The \
					         current runner requires maintenance for schema changes. Online \
					         expansion needs a tested compatibility contract first.",
					maintenance: true,
				}
			}
			(Intent::Rollback, SchemaChange::Unchanged) => {
				Review {
					title: "Review previous release",
					detail: "Verify its recorded image digest, the actual schema, and readiness \
					         before moving traffic. The database stays on its current compatible \
					         schema.",
					maintenance: false,
				}
			}
			(Intent::Deploy, SchemaChange::Unchanged) => {
				Review {
					title: "Review application release",
					detail: "Verify the image and schema, start the new process, check readiness, \
					         move traffic, then drain old connections. This aims to avoid \
					         deployment interruptions; it is not a host-availability guarantee.",
					maintenance: false,
				}
			}
		}
	}

	/// Selecting a different policy invalidates a previously displayed request.
	pub fn target(&mut self, target: Target) {
		self.target = target;
		self.request = None;
	}

	pub fn schema(&mut self, schema: SchemaChange) {
		self.schema = schema;
		self.request = None;
	}

	pub fn preview(&mut self, intent: Intent) {
		self.sequence += 1;
		self.request = Some(Request {
			project: "fieldnotes".into(),
			environment: "production".into(),
			expected_revision: 12,
			idempotency_key: format!("demo-request-{}", self.sequence),
			intent,
			release: match intent {
				Intent::Deploy => "candidate-13",
				Intent::Rollback => "release-11",
			}
			.into(),
			target: self.target,
			schema_change: self.schema,
		});
	}
}
