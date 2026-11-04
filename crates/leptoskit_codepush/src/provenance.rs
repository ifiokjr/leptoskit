//! Source approvals: who says a release was built from public source, not
//! just who shipped it.
//!
//! A publisher signature ([`crate::signing`]) says who released a bundle. It
//! does not say the bundle was built from the source the publisher points at.
//! An approval does, in a narrow sense: an approver, someone other than the
//! publisher, checks out `repository@commit`, builds the bundle themselves,
//! compares every file with the release, and only when every byte matches
//! signs a [`SourceApproval`] naming the release's bundle hash and that
//! commit.
//!
//! A host that ships a [`ProvenancePolicy`] installs only releases that enough
//! distinct approvers it trusts have approved, all naming the same commit of
//! the policy's repository. Approvers are a second trust root, separate from
//! the publisher keys: a publisher can attach approvals to a release, or
//! withhold them, but cannot make one. An approval signed by any other key,
//! the publisher's included, counts for nothing.
//!
//! An approval means "these approvers say they rebuilt identical files from
//! this commit". It does not say the code is safe, and it says nothing about
//! the host app itself, which is trusted through its store distribution.
//!
//! An approval signs its statement's exact bytes, under a signing context of
//! its own, so a release signature can never be replayed as an approval or
//! an approval as a release signature, and verifying never depends on how
//! JSON is serialized again.

use semver::Version;
use serde::Deserialize;
use serde::Serialize;

use crate::CodePushError;
use crate::manifest::BundleManifest;
use crate::signing::ReleasePublicKey;
use crate::signing::ReleaseSigningKey;
use crate::signing::TrustedKeys;
use crate::signing::decode_hex;

/// The [`SourceApproval::format_version`] this build writes and reads.
pub const APPROVAL_FORMAT_VERSION: u32 = 1;

/// The [`SourceApproval::kind`] of every source approval.
pub const APPROVAL_KIND: &str = "leptoskit-codepush-source-approval";

/// The [`ReleaseApprovals::format_version`] this build writes and reads.
pub const APPROVALS_FORMAT_VERSION: u32 = 1;

/// The largest statement accepted, in bytes, checked before it is parsed or
/// its signature checked.
pub const MAX_STATEMENT_BYTES: usize = 4096;

/// The largest file holding one signed approval, checked before parsing.
pub const MAX_SIGNED_APPROVAL_BYTES: u64 = 16 * 1024;

/// The most approvals one release's approvals file holds.
pub const MAX_APPROVALS: usize = 64;

/// The largest approvals file accepted from a registry or from disk, checked
/// before parsing.
pub const MAX_APPROVALS_BYTES: u64 = 1024 * 1024;

/// The domain separator prepended to every approval signature. It differs
/// from the release signature's context, and neither is a prefix of the
/// other, so no signature verifies as both.
const APPROVAL_CONTEXT: &[u8] = b"leptoskit-codepush-source-approval-v1:";

/// The bytes an approval signature covers: the context, then the statement.
fn signed_message(statement: &str) -> Vec<u8> {
	let mut message = Vec::with_capacity(APPROVAL_CONTEXT.len() + statement.len());
	message.extend_from_slice(APPROVAL_CONTEXT);
	message.extend_from_slice(statement.as_bytes());
	message
}

fn invalid(message: impl Into<String>) -> CodePushError {
	CodePushError::InvalidApproval(message.into())
}

/// Checks that `repository` is a public GitHub repository URL,
/// `https://github.com/<owner>/<repo>`, with no credentials, port, query,
/// fragment, trailing slash, or `.git` suffix.
///
/// The error never quotes the value, which could carry a token.
pub fn validate_repository(repository: &str) -> Result<(), CodePushError> {
	let segment = |segment: &str| {
		!segment.is_empty()
			&& segment != "."
			&& segment != ".."
			&& segment
				.bytes()
				.all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
	};
	let valid = repository
		.strip_prefix("https://github.com/")
		.and_then(|path| path.split_once('/'))
		.is_some_and(|(owner, name)| segment(owner) && segment(name) && !is_git_suffixed(name));

	if valid {
		Ok(())
	} else {
		Err(invalid(
			"the repository must be a GitHub HTTPS URL, https://github.com/<owner>/<repo>, \
			 without credentials, a query, a fragment, a trailing slash, or .git",
		))
	}
}

/// Whether a repository name ends in `.git`, in any case: GitHub drops it, so
/// the canonical URL never has it.
fn is_git_suffixed(name: &str) -> bool {
	name.rsplit_once('.')
		.is_some_and(|(_, extension)| extension.eq_ignore_ascii_case("git"))
}

/// Checks that `commit` is a full Git object id: 40 (SHA-1) or 64 (SHA-256)
/// lowercase hex characters.
pub fn validate_commit(commit: &str) -> Result<(), CodePushError> {
	if is_lower_hex(commit, &[40, 64]) {
		Ok(())
	} else {
		Err(invalid(
			"the commit must be a full Git commit id: 40 or 64 lowercase hex characters",
		))
	}
}

fn is_lower_hex(value: &str, lengths: &[usize]) -> bool {
	lengths.contains(&value.len())
		&& value
			.bytes()
			.all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Whether two validated repository URLs name the same repository. GitHub
/// owner and repository names are case-insensitive.
fn same_repository(left: &str, right: &str) -> bool {
	left.eq_ignore_ascii_case(right)
}

/// An approver's statement: the release with this bundle hash rebuilt,
/// file for file, from `repository` at `commit`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceApproval {
	/// The statement format version, [`APPROVAL_FORMAT_VERSION`].
	pub format_version: u32,
	/// Always [`APPROVAL_KIND`].
	pub kind: String,
	/// The release's app.
	pub app: String,
	/// The release's channel.
	pub channel: String,
	/// The release's version.
	pub version: Version,
	/// The release's build.
	pub build: u64,
	/// The release's [`BundleManifest::bundle_hash`], which covers every file
	/// hash in it.
	pub bundle_hash: String,
	/// The public GitHub repository the approver built from,
	/// `https://github.com/<owner>/<repo>`.
	pub repository: String,
	/// The full Git commit id the approver built.
	pub commit: String,
}

impl SourceApproval {
	/// The statement that `manifest`'s release was rebuilt from `repository`
	/// at `commit`.
	pub fn new(
		manifest: &BundleManifest,
		repository: &str,
		commit: &str,
	) -> Result<Self, CodePushError> {
		let statement = Self {
			format_version: APPROVAL_FORMAT_VERSION,
			kind: APPROVAL_KIND.to_owned(),
			app: manifest.app.clone(),
			channel: manifest.channel.clone(),
			version: manifest.version.clone(),
			build: manifest.build,
			bundle_hash: manifest.bundle_hash(),
			repository: repository.to_owned(),
			commit: commit.to_owned(),
		};
		statement.validate()?;
		Ok(statement)
	}

	/// Checks the format version, kind, bundle hash, repository, and commit.
	pub fn validate(&self) -> Result<(), CodePushError> {
		if self.format_version != APPROVAL_FORMAT_VERSION {
			return Err(invalid(format!(
				"unsupported approval format version {}",
				self.format_version
			)));
		}

		if self.kind != APPROVAL_KIND {
			return Err(invalid(format!("the statement is not a {APPROVAL_KIND}")));
		}

		if !is_lower_hex(&self.bundle_hash, &[64]) {
			return Err(invalid("the bundle hash must be a lowercase hex SHA-256"));
		}

		validate_repository(&self.repository)?;
		validate_commit(&self.commit)
	}

	/// Whether the statement is about exactly the release `manifest`
	/// describes, whose bundle hash is `bundle_hash`.
	fn covers(&self, manifest: &BundleManifest, bundle_hash: &str) -> bool {
		self.app == manifest.app
			&& self.channel == manifest.channel
			&& self.version == manifest.version
			&& self.build == manifest.build
			&& self.bundle_hash == bundle_hash
	}

	/// The release the statement is about, `app/channel/version+build`.
	pub fn release(&self) -> String {
		format!(
			"{}/{}/{}+{}",
			self.app, self.channel, self.version, self.build
		)
	}
}

/// A signed [`SourceApproval`]: the statement's exact JSON text, the
/// approver's public key, and the ed25519 signature over that text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedApproval {
	/// The statement exactly as signed. Verification checks these bytes and
	/// parses them only afterwards; it never serializes them again.
	pub statement: String,
	/// The approver's public key, hex encoded.
	pub public_key: String,
	/// The ed25519 signature over the statement, hex encoded.
	pub signature: String,
}

impl SignedApproval {
	/// Signs `statement` with an approver's key.
	pub fn sign(
		statement: &SourceApproval,
		approver: &ReleaseSigningKey,
	) -> Result<Self, CodePushError> {
		statement.validate()?;
		let text = serde_json::to_string(statement)?;

		if text.len() > MAX_STATEMENT_BYTES {
			return Err(invalid(format!(
				"the statement exceeds {MAX_STATEMENT_BYTES} bytes"
			)));
		}

		let signature = approver.sign_message(&signed_message(&text));
		Ok(Self {
			statement: text,
			public_key: approver.public_key().to_hex(),
			signature: hex::encode(signature),
		})
	}

	/// Parses one signed approval, as `leptoskit codepush approve` writes it,
	/// after checking its size. Nothing is verified yet.
	pub fn from_slice(bytes: &[u8]) -> Result<Self, CodePushError> {
		if bytes.len() as u64 > MAX_SIGNED_APPROVAL_BYTES {
			return Err(invalid(format!(
				"an approval exceeds {MAX_SIGNED_APPROVAL_BYTES} bytes"
			)));
		}

		Ok(serde_json::from_slice(bytes)?)
	}

	/// Checks the signature against the approval's own public key and parses
	/// the statement it covers.
	///
	/// This shows only that the holder of that key signed the statement.
	/// Whether the key counts is a [`ProvenancePolicy`]'s decision.
	pub fn open(&self) -> Result<(ReleasePublicKey, SourceApproval), CodePushError> {
		let key = ReleasePublicKey::from_hex(&self.public_key)?;
		let statement = self.verify_with(&key)?;
		Ok((key, statement))
	}

	/// Checks the signature against `key`, then parses and validates the
	/// statement.
	fn verify_with(&self, key: &ReleasePublicKey) -> Result<SourceApproval, CodePushError> {
		if self.statement.len() > MAX_STATEMENT_BYTES {
			return Err(invalid(format!(
				"the statement exceeds {MAX_STATEMENT_BYTES} bytes"
			)));
		}

		let signature = decode_hex::<64>("an approval signature", &self.signature)
			.map_err(|_| invalid("the approval signature must be 128 hex characters"))?;

		if !key.verifies(&signed_message(&self.statement), &signature) {
			return Err(invalid(format!(
				"the approval signature does not verify for key {}",
				key.to_hex()
			)));
		}

		let statement: SourceApproval = serde_json::from_str(&self.statement)?;
		statement.validate()?;
		Ok(statement)
	}
}

/// The approvals file of one release: every signed approval attached to it.
///
/// It is untrusted input. A host counts only the approvals its
/// [`ProvenancePolicy`] accepts, and ignores the rest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseApprovals {
	/// The approvals file format version, [`APPROVALS_FORMAT_VERSION`].
	pub format_version: u32,
	/// The signed approvals, at most [`MAX_APPROVALS`].
	pub approvals: Vec<SignedApproval>,
}

impl ReleaseApprovals {
	/// An approvals file with no approvals yet.
	pub fn empty() -> Self {
		Self {
			format_version: APPROVALS_FORMAT_VERSION,
			approvals: Vec::new(),
		}
	}

	/// Parses an approvals file after checking its size, then its format
	/// version and approval count. Nothing is verified yet.
	pub fn from_slice(bytes: &[u8]) -> Result<Self, CodePushError> {
		if bytes.len() as u64 > MAX_APPROVALS_BYTES {
			return Err(invalid(format!(
				"the approvals file exceeds {MAX_APPROVALS_BYTES} bytes"
			)));
		}

		let approvals: Self = serde_json::from_slice(bytes)?;

		if approvals.format_version != APPROVALS_FORMAT_VERSION {
			return Err(invalid(format!(
				"unsupported approvals format version {}",
				approvals.format_version
			)));
		}

		if approvals.approvals.len() > MAX_APPROVALS {
			return Err(invalid(format!(
				"the approvals file holds more than {MAX_APPROVALS} approvals"
			)));
		}

		Ok(approvals)
	}

	/// The approvals file as pretty JSON, refused when it would not parse
	/// back within the bounds [`Self::from_slice`] checks.
	pub fn to_vec(&self) -> Result<Vec<u8>, CodePushError> {
		let bytes = serde_json::to_vec_pretty(self)?;
		Self::from_slice(&bytes)?;
		Ok(bytes)
	}

	/// Adds each approval that is not already in the file, keeping the order
	/// they arrive in, and returns how many were added. Fails, adding
	/// nothing, when the file would hold more than [`MAX_APPROVALS`].
	pub fn merge(
		&mut self,
		approvals: impl IntoIterator<Item = SignedApproval>,
	) -> Result<usize, CodePushError> {
		let mut merged = self.approvals.clone();

		for approval in approvals {
			if !merged.contains(&approval) {
				merged.push(approval);
			}
		}

		if merged.len() > MAX_APPROVALS {
			return Err(invalid(format!(
				"a release holds at most {MAX_APPROVALS} approvals"
			)));
		}

		let added = merged.len() - self.approvals.len();
		self.approvals = merged;
		Ok(added)
	}
}

/// A [`ProvenancePolicy`] as a host's update configuration writes it: the
/// repository, the approvers' hex public keys, and the threshold.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProvenanceConfiguration {
	/// The public GitHub repository releases must be built from.
	pub repository: String,
	/// The approvers' ed25519 public keys, hex encoded.
	pub approvers: Vec<String>,
	/// How many distinct approvers must approve a release.
	pub threshold: usize,
}

/// What a host requires before it installs a release: approvals from at
/// least `threshold` distinct keys among `approvers`, all naming the same
/// commit of `repository`.
///
/// The policy belongs to the host, compiled in or shipped with the store
/// build, so downloaded code cannot change it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvenancePolicy {
	repository: String,
	approvers: TrustedKeys,
	threshold: usize,
}

impl ProvenancePolicy {
	/// A policy requiring `threshold` of `approvers` to approve each release
	/// as built from `repository`.
	///
	/// Fails when the repository is not a GitHub HTTPS URL, an approver is
	/// listed twice, or the threshold is zero or more than the number of
	/// approvers.
	pub fn new(
		repository: impl Into<String>,
		approvers: TrustedKeys,
		threshold: usize,
	) -> Result<Self, CodePushError> {
		let repository = repository.into();
		validate_repository(&repository)
			.map_err(|error| CodePushError::InvalidProvenance(error.to_string()))?;
		let keys: Vec<_> = approvers.iter().collect();

		if let Some(duplicate) = keys
			.iter()
			.enumerate()
			.find_map(|(index, key)| keys[..index].contains(key).then_some(key))
		{
			return Err(CodePushError::InvalidProvenance(format!(
				"approver {} is listed more than once",
				duplicate.to_hex()
			)));
		}

		if threshold == 0 || threshold > keys.len() {
			return Err(CodePushError::InvalidProvenance(format!(
				"the threshold must be between 1 and the {} approvers",
				keys.len()
			)));
		}

		Ok(Self {
			repository,
			approvers,
			threshold,
		})
	}

	/// A policy from its configuration.
	pub fn from_configuration(
		configuration: &ProvenanceConfiguration,
	) -> Result<Self, CodePushError> {
		let approvers = TrustedKeys::from_hex(&configuration.approvers)
			.map_err(|error| CodePushError::InvalidProvenance(format!("approvers: {error}")))?;
		Self::new(
			configuration.repository.clone(),
			approvers,
			configuration.threshold,
		)
	}

	/// The repository releases must be built from.
	pub fn repository(&self) -> &str {
		&self.repository
	}

	/// The approver keys.
	pub fn approvers(&self) -> &TrustedKeys {
		&self.approvers
	}

	/// How many distinct approvers must approve a release.
	pub fn threshold(&self) -> usize {
		self.threshold
	}

	/// Checks that `approvals` approve the release `manifest` describes, and
	/// returns the commit they name.
	///
	/// An approval counts when its key is one of the approvers, its signature
	/// verifies over its exact statement bytes, the statement names this
	/// release's app, channel, version, build, and bundle hash, and its
	/// repository is the policy's. Every other approval, including one signed
	/// by a publisher key or by no approver at all, counts for nothing. Each
	/// approver counts once, however many approvals they signed.
	///
	/// Fails with [`CodePushError::ConflictingApprovals`] when counted
	/// approvals name different commits, which is never outvoted, and with
	/// [`CodePushError::Unapproved`] when fewer than the threshold of
	/// approvers count.
	pub fn verify(
		&self,
		manifest: &BundleManifest,
		approvals: &[SignedApproval],
	) -> Result<VerifiedSource, CodePushError> {
		let release = || {
			format!(
				"{}/{}/{}+{}",
				manifest.app, manifest.channel, manifest.version, manifest.build
			)
		};

		if approvals.len() > MAX_APPROVALS {
			return Err(invalid(format!(
				"a release holds at most {MAX_APPROVALS} approvals"
			)));
		}

		let bundle_hash = manifest.bundle_hash();
		let mut counted: Vec<ReleasePublicKey> = Vec::new();
		let mut commit: Option<String> = None;

		for approval in approvals {
			let Some(key) = ReleasePublicKey::from_hex(&approval.public_key)
				.ok()
				.filter(|key| self.approvers.contains(key))
			else {
				continue;
			};
			let Ok(statement) = approval.verify_with(&key) else {
				continue;
			};

			if !statement.covers(manifest, &bundle_hash)
				|| !same_repository(&statement.repository, &self.repository)
			{
				continue;
			}

			match &commit {
				Some(named) if *named != statement.commit => {
					return Err(CodePushError::ConflictingApprovals {
						release: release(),
						first: named.clone(),
						second: statement.commit,
					});
				}
				Some(_) => {}
				None => commit = Some(statement.commit),
			}

			if !counted.contains(&key) {
				counted.push(key);
			}
		}

		let (Some(commit), true) = (commit, counted.len() >= self.threshold) else {
			return Err(CodePushError::Unapproved {
				release: release(),
				approvals: counted.len(),
				threshold: self.threshold,
			});
		};

		Ok(VerifiedSource {
			repository: self.repository.clone(),
			commit,
			approvers: counted.iter().map(ReleasePublicKey::to_hex).collect(),
		})
	}
}

/// What a [`ProvenancePolicy`] verified about a release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedSource {
	/// The policy's repository.
	pub repository: String,
	/// The commit every counted approval names.
	pub commit: String,
	/// The approvers whose approvals counted, hex encoded, each once.
	pub approvers: Vec<String>,
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::manifest::MANIFEST_FORMAT_VERSION;

	const REPOSITORY: &str = "https://github.com/example/app";
	const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
	const OTHER_COMMIT: &str = "89abcdef0123456789abcdef0123456789abcdef";

	fn manifest() -> BundleManifest {
		BundleManifest {
			format_version: MANIFEST_FORMAT_VERSION,
			app: "app".to_owned(),
			channel: "production".to_owned(),
			version: Version::new(1, 0, 0),
			build: 2,
			entry: "app.wasm".to_owned(),
			files: Vec::new(),
			created_unix: 0,
			mandatory: false,
		}
	}

	fn key(seed: u8) -> ReleaseSigningKey {
		ReleaseSigningKey::from_seed(&[seed; 32])
	}

	fn policy(threshold: usize) -> ProvenancePolicy {
		let approvers = TrustedKeys::new([key(1).public_key(), key(2).public_key()]).expect("keys");
		ProvenancePolicy::new(REPOSITORY, approvers, threshold).expect("policy")
	}

	fn approve(seed: u8, manifest: &BundleManifest, commit: &str) -> SignedApproval {
		let statement = SourceApproval::new(manifest, REPOSITORY, commit).expect("statement");
		SignedApproval::sign(&statement, &key(seed)).expect("sign")
	}

	#[test]
	fn repositories_must_be_plain_github_https_urls() {
		for valid in [
			REPOSITORY,
			"https://github.com/Owner-1/repo_name.rs",
			"https://github.com/o/r.s",
		] {
			validate_repository(valid).unwrap_or_else(|error| panic!("{valid}: {error}"));
		}

		for invalid in [
			"",
			"http://github.com/example/app",
			"https://gitlab.com/example/app",
			"https://github.com/example",
			"https://github.com/example/app/",
			"https://github.com/example/app.git",
			"https://github.com/example/app.GIT",
			"https://github.com/example/app/tree/main",
			"https://github.com/example/app?tab=readme",
			"https://github.com/example/app#readme",
			"https://token@github.com/example/app",
			"https://github.com:443/example/app",
			"https://github.com/../app",
			"https://github.com/example/a pp",
			"https://github.com//app",
		] {
			let error = validate_repository(invalid).expect_err(invalid);
			assert!(
				invalid.is_empty() || !error.to_string().contains(invalid),
				"the error never quotes the value: {error}"
			);
		}
	}

	#[test]
	fn commits_must_be_full_lowercase_object_ids() {
		validate_commit(COMMIT).expect("sha-1");
		validate_commit(&"a".repeat(64)).expect("sha-256");

		for invalid in ["", &COMMIT[..39], &COMMIT.to_uppercase(), &"g".repeat(40)] {
			assert!(validate_commit(invalid).is_err(), "{invalid:?}");
		}
	}

	#[test]
	fn verifies_a_threshold_of_distinct_approvers_and_returns_the_commit() {
		let manifest = manifest();
		let approvals = [approve(1, &manifest, COMMIT), approve(2, &manifest, COMMIT)];
		let source = policy(2).verify(&manifest, &approvals).expect("approved");
		assert_eq!(source.commit, COMMIT);
		assert_eq!(source.repository, REPOSITORY);
		assert_eq!(
			source.approvers,
			[key(1).public_key().to_hex(), key(2).public_key().to_hex()]
		);
	}

	#[test]
	fn duplicate_approvals_from_one_key_count_once() {
		let manifest = manifest();
		let approval = approve(1, &manifest, COMMIT);
		// The same key signing again, and the same approval twice.
		let approvals = [approval.clone(), approval, approve(1, &manifest, COMMIT)];
		assert!(matches!(
			policy(2).verify(&manifest, &approvals),
			Err(CodePushError::Unapproved {
				approvals: 1,
				threshold: 2,
				..
			})
		));
	}

	#[test]
	fn approvers_naming_different_commits_are_an_error_not_a_vote() {
		let manifest = manifest();
		let approvals = [
			approve(1, &manifest, COMMIT),
			approve(2, &manifest, OTHER_COMMIT),
		];
		assert!(matches!(
			policy(1).verify(&manifest, &approvals),
			Err(CodePushError::ConflictingApprovals { first, second, .. })
				if first == COMMIT && second == OTHER_COMMIT
		));
	}

	#[test]
	fn keys_outside_the_approver_set_count_for_nothing() {
		let manifest = manifest();
		// The publisher's key, say, or anyone else's.
		let approvals = [approve(9, &manifest, COMMIT)];
		assert!(matches!(
			policy(1).verify(&manifest, &approvals),
			Err(CodePushError::Unapproved { approvals: 0, .. })
		));
		// An outsider naming a different commit doesn't block the approvers.
		let approvals = [
			approve(9, &manifest, OTHER_COMMIT),
			approve(1, &manifest, COMMIT),
		];
		assert_eq!(
			policy(1).verify(&manifest, &approvals).expect("one").commit,
			COMMIT
		);
	}

	#[test]
	fn approvals_of_another_release_or_repository_count_for_nothing() {
		let manifest = manifest();
		let mut other_build = manifest.clone();
		other_build.build = 3;
		let mut other_version = manifest.clone();
		other_version.version = Version::new(1, 0, 1);
		let mut other_files = manifest.clone();
		other_files.mandatory = true;
		let mut other_channel = manifest.clone();
		other_channel.channel = "beta".to_owned();

		for other in [other_build, other_version, other_files, other_channel] {
			let approvals = [approve(1, &other, COMMIT)];
			assert!(
				matches!(
					policy(1).verify(&manifest, &approvals),
					Err(CodePushError::Unapproved { approvals: 0, .. })
				),
				"{other:?}"
			);
		}

		let statement = SourceApproval::new(&manifest, "https://github.com/example/fork", COMMIT)
			.expect("statement");
		let approvals = [SignedApproval::sign(&statement, &key(1)).expect("sign")];
		assert!(matches!(
			policy(1).verify(&manifest, &approvals),
			Err(CodePushError::Unapproved { approvals: 0, .. })
		));
	}

	#[test]
	fn repository_names_compare_without_case() {
		let manifest = manifest();
		let statement = SourceApproval::new(&manifest, "https://github.com/Example/App", COMMIT)
			.expect("statement");
		let approvals = [SignedApproval::sign(&statement, &key(1)).expect("sign")];
		policy(1)
			.verify(&manifest, &approvals)
			.expect("same repository");
	}

	#[test]
	fn tampered_statements_and_signatures_do_not_verify() {
		let manifest = manifest();
		let approval = approve(1, &manifest, COMMIT);

		let mut changed = approval.clone();
		changed.statement = changed.statement.replace(COMMIT, OTHER_COMMIT);
		assert!(changed.open().is_err());

		// Whitespace changes no field, but the signed bytes are what count.
		let mut reformatted = approval.clone();
		reformatted.statement = format!(" {}", reformatted.statement);
		assert!(reformatted.open().is_err());

		let mut signature = approval.clone();
		let flipped = if signature.signature.starts_with('0') {
			"1"
		} else {
			"0"
		};
		signature.signature.replace_range(..1, flipped);
		assert!(signature.open().is_err());

		let mut short = approval.clone();
		short.signature.truncate(64);
		assert!(short.open().is_err());

		for tampered in [changed, reformatted, signature, short] {
			assert!(matches!(
				policy(1).verify(&manifest, &[tampered]),
				Err(CodePushError::Unapproved { approvals: 0, .. })
			));
		}

		let (signer, statement) = approval.open().expect("intact");
		assert_eq!(signer, key(1).public_key());
		assert_eq!(statement.commit, COMMIT);
	}

	#[test]
	fn a_release_signature_cannot_be_replayed_as_an_approval() {
		let manifest = manifest();
		let approver = key(1);
		let release = approver.sign(&manifest);
		// Whatever statement the signature is presented with, it was made
		// under the release context.
		for statement in [
			manifest.bundle_hash(),
			serde_json::to_string(&SourceApproval::new(&manifest, REPOSITORY, COMMIT).unwrap())
				.unwrap(),
		] {
			let replayed = SignedApproval {
				statement,
				public_key: release.public_key.clone(),
				signature: release.signature.clone(),
			};
			assert!(replayed.open().is_err());
			assert!(policy(1).verify(&manifest, &[replayed]).is_err());
		}

		// And an approval's signature is no release signature.
		let approval = approve(1, &manifest, COMMIT);
		let trusted = TrustedKeys::new([approver.public_key()]).expect("keys");
		let forged = crate::signing::ReleaseSignature {
			public_key: approval.public_key,
			signature: approval.signature,
		};
		assert!(matches!(
			trusted.verify(&manifest, Some(&forged)),
			Err(CodePushError::BadSignature(_))
		));
	}

	#[test]
	fn statements_reject_unknown_fields_and_other_kinds() {
		let manifest = manifest();
		let statement = SourceApproval::new(&manifest, REPOSITORY, COMMIT).expect("statement");
		let mut value = serde_json::to_value(&statement).expect("value");
		value["note"] = "extra".into();
		assert!(serde_json::from_value::<SourceApproval>(value).is_err());

		let mut other = statement.clone();
		other.kind = "something-else".to_owned();
		assert!(SignedApproval::sign(&other, &key(1)).is_err());
		let mut future = statement;
		future.format_version = 2;
		assert!(SignedApproval::sign(&future, &key(1)).is_err());
	}

	#[test]
	fn oversized_inputs_are_refused_before_parsing() {
		let mut approval = approve(1, &manifest(), COMMIT);
		approval.statement = " ".repeat(MAX_STATEMENT_BYTES + 1);
		assert!(approval.open().is_err());
		assert!(
			SignedApproval::from_slice(&vec![b' '; MAX_SIGNED_APPROVAL_BYTES as usize + 1])
				.is_err()
		);
		assert!(
			ReleaseApprovals::from_slice(&vec![b' '; MAX_APPROVALS_BYTES as usize + 1]).is_err()
		);
		let mut many = ReleaseApprovals::empty();
		let approval = approve(1, &manifest(), COMMIT);
		many.approvals = vec![approval; MAX_APPROVALS + 1];
		let bytes = serde_json::to_vec(&many).expect("bytes");
		assert!(ReleaseApprovals::from_slice(&bytes).is_err());
		assert!(policy(1).verify(&manifest(), &many.approvals).is_err());
	}

	#[test]
	fn merging_skips_approvals_already_attached() {
		let manifest = manifest();
		let mut approvals = ReleaseApprovals::empty();
		let first = approve(1, &manifest, COMMIT);
		let second = approve(2, &manifest, COMMIT);
		assert_eq!(approvals.merge([first.clone()]).expect("merge"), 1);
		assert_eq!(approvals.merge([first, second]).expect("merge"), 1);
		assert_eq!(approvals.approvals.len(), 2);
		let parsed =
			ReleaseApprovals::from_slice(&approvals.to_vec().expect("bytes")).expect("parse");
		assert_eq!(parsed, approvals);
	}

	#[test]
	fn policies_need_a_reachable_threshold_and_distinct_approvers() {
		let one = TrustedKeys::new([key(1).public_key()]).expect("keys");
		assert!(ProvenancePolicy::new(REPOSITORY, one.clone(), 0).is_err());
		assert!(ProvenancePolicy::new(REPOSITORY, one.clone(), 2).is_err());
		assert!(ProvenancePolicy::new("https://example.com/a/b", one.clone(), 1).is_err());
		let twice = TrustedKeys::new([key(1).public_key(), key(1).public_key()]).expect("keys");
		assert!(matches!(
			ProvenancePolicy::new(REPOSITORY, twice, 1),
			Err(CodePushError::InvalidProvenance(_))
		));
		let configured = ProvenancePolicy::from_configuration(&ProvenanceConfiguration {
			repository: REPOSITORY.to_owned(),
			approvers: vec![key(1).public_key().to_hex()],
			threshold: 1,
		})
		.expect("policy");
		assert_eq!(
			configured,
			ProvenancePolicy::new(REPOSITORY, one, 1).unwrap()
		);
	}
}
