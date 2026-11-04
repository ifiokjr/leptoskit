//! Signed deployment policies. A project's deployment key signs which
//! origins serve which verified releases, so a copied website cannot claim
//! the project's releases and an old policy cannot replace a newer one.
//! A policy is a statement by the key holder; it does not prove what a
//! server at those origins is running.

use std::path::Path;
use std::path::PathBuf;
use std::time::SystemTime;

use anyhow::Context;
use anyhow::Result;
use anyhow::ensure;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use leptoskit_codepush::ed25519_dalek::Signature;
use leptoskit_codepush::ed25519_dalek::Signer as _;
use leptoskit_codepush::ed25519_dalek::SigningKey;
use leptoskit_codepush::ed25519_dalek::VerifyingKey;
use serde::Deserialize;
use serde::Serialize;
use sha2::Digest as _;
use sha2::Sha256;

const KIND: &str = "leptoskit-deployment-policy";
const FORMAT_VERSION: u32 = 1;

/// The DSSE payload type of a signed policy.
const PAYLOAD_TYPE: &str = "application/vnd.leptoskit.deployment-policy+json";

/// Where an origin publishes its signed policy.
pub(crate) const WELL_KNOWN_PATH: &str = "/.well-known/leptoskit/deployment.json";

const MAX_ENVELOPE_BYTES: u64 = 1024 * 1024;

/// How far a policy's issue time may be ahead of the verifier's clock.
const CLOCK_SKEW_SECONDS: u64 = 300;

/// Writes a new deployment signing key.
#[derive(Debug, clap::Args)]
pub struct KeygenArgs {
	/// A new file for the private key, readable only by you.
	#[arg(long)]
	out: PathBuf,
}

/// Signs a deployment policy.
#[derive(Debug, clap::Args)]
pub struct SignArgs {
	/// The policy JSON; see the verifying releases guide.
	#[arg(long)]
	policy: PathBuf,
	/// The private key written by `verify keygen`.
	#[arg(long)]
	key: PathBuf,
	/// A new file for the signed policy, served at
	/// `/.well-known/leptoskit/deployment.json` on each origin.
	#[arg(long)]
	out: PathBuf,
}

/// Checks the signed policy an origin publishes.
#[derive(Debug, clap::Args)]
pub struct DeploymentArgs {
	/// The website or API origin to check, such as `https://example.com`.
	#[arg(long)]
	origin: String,
	/// A deployment public key you trust, in hex; repeat for each. Get it
	/// from the project, never from the website being checked.
	#[arg(long = "key", value_name = "HEX", required = true)]
	keys: Vec<String>,
	/// The repository the policy must name.
	#[arg(long)]
	repository: Option<String>,
	/// Read the signed policy from this file instead of the origin.
	#[arg(long)]
	policy: Option<PathBuf>,
	/// A file remembering the newest policy seen, so an older one is refused.
	#[arg(long)]
	state: Option<PathBuf>,
}

/// What a deployment key holder approves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Policy {
	pub(crate) format_version: u32,
	pub(crate) kind: String,
	/// The public GitHub repository every release is built from.
	pub(crate) repository: String,
	/// The deployment's name, such as `production`.
	pub(crate) deployment: String,
	/// Every origin this deployment serves, as `https://host[:port]`.
	pub(crate) origins: Vec<String>,
	/// Increases with every new policy for this deployment.
	pub(crate) sequence: u64,
	/// Unix seconds.
	pub(crate) issued_at: u64,
	/// Unix seconds; the policy is refused from this time.
	pub(crate) expires_at: u64,
	/// The releases the origins may serve.
	pub(crate) releases: Vec<Release>,
}

/// One approved release, as `verify build` describes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Release {
	pub(crate) commit: String,
	pub(crate) inventory_sha256: String,
	pub(crate) executable_sha256: String,
}

/// A Dead Simple Signing Envelope, as in-toto uses.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Envelope {
	payload_type: String,
	payload: String,
	signatures: Vec<EnvelopeSignature>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EnvelopeSignature {
	keyid: String,
	sig: String,
}

/// The newest policy a verifier has accepted for one deployment.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
	repository: String,
	deployment: String,
	sequence: u64,
	policy_sha256: String,
}

/// DSSE's pre-authentication encoding: what is actually signed.
fn pae(payload: &[u8]) -> Vec<u8> {
	let mut encoded = format!(
		"DSSEv1 {} {PAYLOAD_TYPE} {} ",
		PAYLOAD_TYPE.len(),
		payload.len()
	)
	.into_bytes();
	encoded.extend_from_slice(payload);
	encoded
}

/// `https://host[:port]` with no path, query, fragment, or credentials.
pub(crate) fn origin(value: &str) -> Result<String> {
	let url = url::Url::parse(value).with_context(|| format!("{value:?} is not an origin"))?;
	ensure!(url.scheme() == "https", "{value:?} must be an https origin");
	ensure!(
		url.username().is_empty()
			&& url.password().is_none()
			&& url.path() == "/"
			&& url.query().is_none()
			&& url.fragment().is_none(),
		"{value:?} must be an origin, without a path, query, or credentials"
	);
	Ok(url.origin().ascii_serialization())
}

/// Checks a policy's own form, independent of time and of any verifier.
pub(crate) fn validate(policy: &Policy) -> Result<()> {
	ensure!(
		policy.format_version == FORMAT_VERSION && policy.kind == KIND,
		"unsupported deployment policy format or kind"
	);
	super::validate_repository(&policy.repository)?;
	ensure!(
		super::is_name(&policy.deployment, b"-_."),
		"the deployment name must be a plain word, such as production"
	);
	ensure!(
		!policy.origins.is_empty(),
		"the policy must list its origins"
	);

	for value in &policy.origins {
		ensure!(
			origin(value)? == *value,
			"write origin {value:?} as {}",
			origin(value)?
		);
	}

	ensure!(
		policy.expires_at > policy.issued_at,
		"the policy must expire after it is issued"
	);
	ensure!(
		!policy.releases.is_empty(),
		"the policy must approve at least one release"
	);

	for release in &policy.releases {
		ensure!(
			super::is_commit(&release.commit)
				&& super::digest_is_valid(&release.inventory_sha256)
				&& super::digest_is_valid(&release.executable_sha256),
			"each release needs a full commit ID and lowercase SHA-256 digests"
		);
	}

	Ok(())
}

fn read_key(path: &Path) -> Result<SigningKey> {
	let text = std::fs::read_to_string(path)
		.with_context(|| format!("could not read the key at {}", path.display()))?;
	let seed: [u8; 32] = hex::decode(text.trim())
		.ok()
		.and_then(|bytes| bytes.try_into().ok())
		.context("the key file must hold 64 hex digits")?;
	Ok(SigningKey::from_bytes(&seed))
}

fn create_new(path: &Path, bytes: &[u8], private: bool) -> Result<()> {
	use std::io::Write as _;

	let mut options = std::fs::File::options();
	options.write(true).create_new(true);

	#[cfg(unix)]
	if private {
		use std::os::unix::fs::OpenOptionsExt as _;

		options.mode(0o600);
	}

	#[cfg(not(unix))]
	let _ = private;

	options
		.open(path)
		.with_context(|| format!("could not create a new file at {}", path.display()))?
		.write_all(bytes)?;
	Ok(())
}

/// Signs `policy` exactly as given, after validating it.
pub(crate) fn sign(policy: &[u8], key: &SigningKey) -> Result<Envelope> {
	let parsed: Policy = serde_json::from_slice(policy).context("invalid deployment policy")?;
	validate(&parsed)?;
	Ok(Envelope {
		payload_type: PAYLOAD_TYPE.to_owned(),
		payload: STANDARD.encode(policy),
		signatures: vec![EnvelopeSignature {
			keyid: hex::encode(key.verifying_key().as_bytes()),
			sig: STANDARD.encode(key.sign(&pae(policy)).to_bytes()),
		}],
	})
}

/// A policy whose signature a trusted key made, with the digest of its
/// exact bytes and the key that signed it.
#[derive(Debug)]
pub(crate) struct Verified {
	pub(crate) policy: Policy,
	pub(crate) sha256: String,
	pub(crate) keyid: String,
}

/// Authenticates `envelope` with any of `keys`, then checks that `origin`
/// is one of the policy's origins and that it is current at `now`.
pub(crate) fn verify(
	envelope: &[u8],
	keys: &[VerifyingKey],
	origin: &str,
	repository: Option<&str>,
	now: u64,
) -> Result<Verified> {
	let envelope: Envelope =
		serde_json::from_slice(envelope).context("invalid signed deployment policy")?;
	ensure!(
		envelope.payload_type == PAYLOAD_TYPE,
		"not a leptoskit deployment policy"
	);
	let payload = STANDARD
		.decode(&envelope.payload)
		.context("invalid policy payload")?;
	let message = pae(&payload);
	let keyid = envelope
		.signatures
		.iter()
		.find_map(|signature| {
			let bytes: [u8; 64] = STANDARD.decode(&signature.sig).ok()?.try_into().ok()?;
			let signature_bytes = Signature::from_bytes(&bytes);
			keys.iter()
				.find(|key| key.verify_strict(&message, &signature_bytes).is_ok())
				.map(|key| hex::encode(key.as_bytes()))
		})
		.context("no signature on the policy is from a key you trust")?;

	// Only now, with the bytes authenticated, is the policy parsed.
	let policy: Policy = serde_json::from_slice(&payload).context("invalid deployment policy")?;
	validate(&policy)?;

	if let Some(repository) = repository {
		ensure!(
			policy.repository == repository,
			"the policy is for {}, not {repository}",
			policy.repository
		);
	}

	ensure!(
		policy.origins.iter().any(|listed| listed == origin),
		"{origin} is not one of this deployment's origins ({}): it may be copying another site's \
		 policy",
		policy.origins.join(", ")
	);
	ensure!(
		policy.issued_at <= now + CLOCK_SKEW_SECONDS,
		"the policy is issued in the future; check this computer's clock"
	);
	ensure!(now < policy.expires_at, "the policy has expired");

	Ok(Verified {
		sha256: hex::encode(Sha256::digest(&payload)),
		policy,
		keyid,
	})
}

/// Refuses a policy older than, or conflicting with, the newest one
/// remembered in `path`, then remembers this one.
fn remember(path: &Path, verified: &Verified) -> Result<()> {
	let current = State {
		repository: verified.policy.repository.clone(),
		deployment: verified.policy.deployment.clone(),
		sequence: verified.policy.sequence,
		policy_sha256: verified.sha256.clone(),
	};

	if path.exists() {
		let previous: State = super::read_json(path)?;
		ensure!(
			previous.repository == current.repository && previous.deployment == current.deployment,
			"{} remembers {} of {}; use a separate state file per deployment",
			path.display(),
			previous.deployment,
			previous.repository
		);
		ensure!(
			current.sequence >= previous.sequence,
			"policy sequence {} is older than the {} already seen: refusing a rollback",
			current.sequence,
			previous.sequence
		);
		ensure!(
			current.sequence > previous.sequence || current.policy_sha256 == previous.policy_sha256,
			"two different policies claim sequence {}; the deployment key may be misused",
			current.sequence
		);

		if previous == current {
			return Ok(());
		}
	}

	let temporary = path.with_extension("tmp");
	std::fs::write(&temporary, serde_json::to_vec_pretty(&current)?)?;
	std::fs::rename(&temporary, path)?;
	Ok(())
}

fn fetch(origin: &str) -> Result<Vec<u8>> {
	let agent: ureq::Agent = ureq::Agent::config_builder()
		.timeout_global(Some(std::time::Duration::from_secs(15)))
		.max_redirects(0)
		.build()
		.into();
	let url = format!("{origin}{WELL_KNOWN_PATH}");
	let mut response = agent
		.get(&url)
		.call()
		.with_context(|| format!("could not fetch {url}"))?;
	ensure!(
		response.status() == 200,
		"{url} returned {}",
		response.status()
	);
	response
		.body_mut()
		.with_config()
		.limit(MAX_ENVELOPE_BYTES)
		.read_to_vec()
		.with_context(|| format!("could not read {url}"))
}

fn timestamp(seconds: u64) -> String {
	i64::try_from(seconds)
		.ok()
		.and_then(|seconds| chrono::DateTime::from_timestamp(seconds, 0))
		.map_or_else(|| seconds.to_string(), |time| time.to_rfc3339())
}

/// `leptoskit verify keygen`.
pub(super) fn keygen(args: &KeygenArgs) -> Result<()> {
	let mut seed = [0_u8; 32];
	getrandom::fill(&mut seed).map_err(|error| anyhow::anyhow!("no randomness: {error}"))?;
	let key = SigningKey::from_bytes(&seed);
	create_new(
		&args.out,
		format!("{}\n", hex::encode(seed)).as_bytes(),
		true,
	)?;
	println!(
		"public key: {}",
		hex::encode(key.verifying_key().as_bytes())
	);
	println!(
		"Keep {} private. Publish the public key in your repository so people can pin it.",
		args.out.display()
	);
	Ok(())
}

/// `leptoskit verify sign`.
pub(super) fn sign_file(args: &SignArgs) -> Result<()> {
	let policy = std::fs::read(&args.policy)
		.with_context(|| format!("could not read {}", args.policy.display()))?;
	let envelope = sign(&policy, &read_key(&args.key)?)?;
	create_new(&args.out, &serde_json::to_vec_pretty(&envelope)?, false)?;
	println!(
		"Signed. Serve {} at {WELL_KNOWN_PATH} on every origin the policy lists.",
		args.out.display()
	);
	Ok(())
}

/// `leptoskit verify deployment`.
pub(super) fn deployment(args: &DeploymentArgs) -> Result<()> {
	let origin = origin(&args.origin)?;
	let keys = args
		.keys
		.iter()
		.map(|key| {
			hex::decode(key)
				.ok()
				.and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
				.and_then(|bytes| VerifyingKey::from_bytes(&bytes).ok())
				.with_context(|| format!("{key:?} is not an ed25519 public key in hex"))
		})
		.collect::<Result<Vec<_>>>()?;
	let envelope = match &args.policy {
		Some(path) => {
			let bytes = std::fs::read(path)?;
			ensure!(
				bytes.len() as u64 <= MAX_ENVELOPE_BYTES,
				"the signed policy is too large"
			);
			bytes
		}
		None => fetch(&origin)?,
	};
	let now = SystemTime::now()
		.duration_since(SystemTime::UNIX_EPOCH)?
		.as_secs();
	let verified = verify(&envelope, &keys, &origin, args.repository.as_deref(), now)?;

	match &args.state {
		Some(path) => remember(path, &verified)?,
		None => {
			if args.policy.is_none() {
				println!("Pass --state to refuse older policies on later checks.");
			}
		}
	}

	let policy = &verified.policy;
	println!(
		"{origin} serves the {} deployment of {} (policy {}, signed by {})",
		policy.deployment, policy.repository, policy.sequence, verified.keyid
	);
	println!("valid until {}", timestamp(policy.expires_at));
	println!("approved releases:");

	for release in &policy.releases {
		println!(
			"  commit {}  inventory SHA-256 {}  executable SHA-256 {}",
			release.commit, release.inventory_sha256, release.executable_sha256
		);
	}

	println!(
		"Rebuild one with `leptoskit verify reproduce --sha256 <inventory SHA-256>` and its \
		 bundle."
	);
	println!(
		"Not verified: that the server at {origin} runs an approved release. That needs runtime \
		 attestation, which leptoskit does not provide yet."
	);
	Ok(())
}

#[cfg(test)]
mod tests;
