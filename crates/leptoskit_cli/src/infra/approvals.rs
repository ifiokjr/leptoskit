//! Independent backend approvals. Persist bytes, never a verification token.
use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use anyhow::ensure;
use leptoskit_codepush::ed25519_dalek::Signature;
use leptoskit_codepush::ed25519_dalek::Signer as _;
use leptoskit_codepush::ed25519_dalek::SigningKey;
use leptoskit_codepush::ed25519_dalek::VerifyingKey;
use serde::Deserialize;
use serde::Serialize;
use sha2::Digest as _;
use sha2::Sha256;

use super::verified::Descriptor;
use crate::verify::deployment;

const DOMAIN: &[u8] = b"leptoskit-backend-release-approval-v1\0";
const KIND: &str = "leptoskit-backend-release-approval";

#[derive(Debug, clap::Args)]
pub struct ApproveArgs {
	/// Complete descriptor produced by infra build-verified.
	#[arg(long)]
	descriptor: PathBuf,
	/// Independent reviewer's private key from verify keygen.
	#[arg(long)]
	key: PathBuf,
	/// New approval file.
	#[arg(long)]
	out: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Statement {
	format_version: u32,
	kind: String,
	descriptor_sha256: String,
	repository: String,
	commit: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Approval {
	statement: String,
	public_key: String,
	signature: String,
}

/// Operator-managed file, outside publisher evidence. Keys are decoded before
/// comparison so differently cased aliases cannot increase a threshold.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Configuration {
	format_version: u32,
	repository: String,
	deployment: String,
	publisher: String,
	approvers: Vec<String>,
	threshold: usize,
	deployment_keys: Vec<String>,
}

struct Trust {
	configuration: Configuration,
	approvers: Vec<VerifyingKey>,
	deployment_keys: Vec<VerifyingKey>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Evidence {
	format_version: u32,
	/// Retain original bytes, including whitespace, for later revalidation.
	descriptor: String,
	approvals: Vec<Approval>,
	deployment_policy: String,
}

impl Evidence {
	pub(super) fn check_pin(&self, pin: &str) -> Result<()> {
		ensure!(
			hash(self.descriptor.as_bytes()) == pin,
			"supplied descriptor bytes differ from approved descriptor"
		);
		Ok(())
	}
}

fn hash(bytes: &[u8]) -> String {
	hex::encode(Sha256::digest(bytes))
}

fn key(text: &str) -> Result<VerifyingKey> {
	let bytes: [u8; 32] = hex::decode(text)?
		.try_into()
		.map_err(|_| anyhow::anyhow!("invalid public key length"))?;
	Ok(VerifyingKey::from_bytes(&bytes)?)
}

impl Trust {
	fn read(path: &Path) -> Result<Self> {
		let configuration: Configuration = serde_json::from_slice(&std::fs::read(path)?)?;
		ensure!(
			configuration.format_version == 1,
			"unsupported backend trust configuration"
		);
		let publisher = key(&configuration.publisher)?;
		let approvers = configuration
			.approvers
			.iter()
			.map(|value| key(value))
			.collect::<Result<Vec<_>>>()?;
		let unique: BTreeSet<_> = approvers.iter().map(VerifyingKey::to_bytes).collect();
		ensure!(
			unique.len() == approvers.len(),
			"duplicate approver key aliases"
		);
		ensure!(
			!approvers.contains(&publisher),
			"publisher cannot be an independent approver"
		);
		ensure!(
			configuration.threshold > 0 && configuration.threshold <= unique.len(),
			"invalid approval threshold"
		);

		let deployment_keys = configuration
			.deployment_keys
			.iter()
			.map(|value| key(value))
			.collect::<Result<Vec<_>>>()?;
		ensure!(!deployment_keys.is_empty(), "deployment keys required");
		Ok(Self {
			configuration,
			approvers,
			deployment_keys,
		})
	}
}

fn message(statement: &str) -> Vec<u8> {
	[DOMAIN, statement.as_bytes()].concat()
}

fn sign(descriptor: &[u8], key: &SigningKey) -> Result<Approval> {
	let parsed: Descriptor = serde_json::from_slice(descriptor)?;
	parsed.validate()?;
	let statement = serde_json::to_string(&Statement {
		format_version: 1,
		kind: KIND.into(),
		descriptor_sha256: hash(descriptor),
		repository: parsed.recipe.repository,
		commit: parsed.recipe.commit,
	})?;
	Ok(Approval {
		signature: hex::encode(key.sign(&message(&statement)).to_bytes()),
		public_key: hex::encode(key.verifying_key().as_bytes()),
		statement,
	})
}

pub(crate) fn sign_file(args: &ApproveArgs) -> Result<()> {
	let approval = sign(
		&std::fs::read(&args.descriptor)?,
		&deployment::read_key(&args.key)?,
	)?;
	deployment::create_new(&args.out, &serde_json::to_vec_pretty(&approval)?, false)
}

fn verify_approvals(
	evidence: &Evidence,
	descriptor: &Descriptor,
	trust: &Trust,
	digest: &str,
) -> Result<()> {
	let mut counted = BTreeSet::new();

	for approval in &evidence.approvals {
		let signer = key(&approval.public_key)?;

		if !trust.approvers.contains(&signer) {
			continue;
		}

		let bytes: [u8; 64] = hex::decode(&approval.signature)?
			.try_into()
			.map_err(|_| anyhow::anyhow!("invalid approval signature"))?;
		signer.verify_strict(
			&message(&approval.statement),
			&Signature::from_bytes(&bytes),
		)?;
		let statement: Statement = serde_json::from_str(&approval.statement)?;
		ensure!(
			statement.format_version == 1 && statement.kind == KIND,
			"unsupported backend approval kind"
		);
		ensure!(
			statement.repository == descriptor.recipe.repository
				&& statement.commit == descriptor.recipe.commit,
			"conflicting source commitments"
		);

		ensure!(
			statement.descriptor_sha256 == digest,
			"approval descriptor digest differs"
		);
		counted.insert(signer.to_bytes());
	}

	ensure!(
		counted.len() >= trust.configuration.threshold,
		"independent backend approval threshold not met"
	);
	Ok(())
}

fn verify(
	evidence: &Evidence,
	descriptor: &Descriptor,
	trust: &Trust,
	origin: &str,
	now: u64,
) -> Result<deployment::Verified> {
	ensure!(
		evidence.format_version == 1,
		"unsupported backend authorization evidence"
	);
	let original: Descriptor = serde_json::from_str(&evidence.descriptor)?;
	original.validate()?;
	ensure!(
		serde_json::to_value(&original)? == serde_json::to_value(descriptor)?,
		"stored descriptor differs from approved bytes"
	);
	ensure!(
		descriptor.recipe.repository == trust.configuration.repository,
		"backend repository differs from operator policy"
	);
	let digest = hash(evidence.descriptor.as_bytes());
	verify_approvals(evidence, descriptor, trust, &digest)?;
	let verified = deployment::verify(
		evidence.deployment_policy.as_bytes(),
		&trust.deployment_keys,
		origin,
		Some(&trust.configuration.repository),
		now,
	)?;
	ensure!(
		verified.policy.deployment == trust.configuration.deployment,
		"wrong deployment"
	);
	ensure!(
		verified.policy.format_version == 2
			&& verified.policy.releases.iter().any(|release| {
				release.commit == descriptor.recipe.commit
					&& release.inventory_sha256 == descriptor.inventory_sha256
					&& release.executable_sha256 == descriptor.executable_sha256
					&& release.backend.as_ref().is_some_and(|binding| {
						binding.descriptor_sha256 == digest
							&& binding.engine_image_id == descriptor.engine_image_id
					})
			}),
		"policy lacks exact backend descriptor/image authorization"
	);
	Ok(verified)
}

/// The managed directory is operator-owned, like existing infrastructure state.
/// Always use its current policy, including rollback: a newer policy must name
/// the old release. Never suppress replay checks to recover an old image.
pub(super) fn authorize(
	dir: &Path,
	domain: &str,
	descriptor: Option<&Descriptor>,
	stored: Option<&Evidence>,
) -> Result<Option<Evidence>> {
	let path = dir.join("backend-trust.json");

	if !path.exists() {
		ensure!(
			stored.is_none(),
			"approved history requires operator backend-trust.json"
		);

		return Ok(None);
	}

	let trust = Trust::read(&path)?;
	let descriptor =
		descriptor.context("configured backend approvals prohibit unsigned/legacy releases")?;

	let mut evidence: Evidence = match stored {
		Some(evidence) => evidence.clone(),
		None => serde_json::from_slice(&std::fs::read(dir.join("backend-authorization.json"))?)?,
	};

	evidence.deployment_policy = std::fs::read_to_string(dir.join("backend-deployment.json"))?;
	let origin = deployment::origin(&format!("https://{domain}"))?;
	let now = std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)?
		.as_secs();
	let verified = verify(&evidence, descriptor, &trust, &origin, now)?;
	deployment::remember(&dir.join("backend-policy-state.json"), &verified)?;
	Ok(Some(evidence))
}

#[cfg(test)]
mod tests;
