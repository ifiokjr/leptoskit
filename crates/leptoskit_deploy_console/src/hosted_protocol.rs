//! Shared account and installation representations for web and native clients.
use serde::Deserialize;
use serde::Serialize;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ownership {
	Customer,
	LeptoskitDigitalocean,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Installation {
	pub id: String,
	pub project: String,
	pub environment: String,
	pub ownership: Ownership,
	pub last_seen: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AccountView {
	pub id: String,
	pub status: String,
	pub plan: String,
	pub installations: Vec<Installation>,
}
