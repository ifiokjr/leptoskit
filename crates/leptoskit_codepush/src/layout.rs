//! The static registry layout: where every release file lives, so that any
//! file server, object store, or CDN can host a channel without running code.
//!
//! ```text
//! <app>/<channel>/index.json                                 the channel's newest release
//! <app>/<channel>/<version>+<build>/release.lkb              the container
//! <app>/<channel>/<version>+<build>/release.sig              the publisher signature
//! <app>/<channel>/<version>+<build>/deltas/<base>.jsondelta  a delta from the release
//!                                                            whose container hashes to <base>
//! ```
//!
//! Releases are immutable and content addressed, so the only file that ever
//! changes is `index.json`: a signed pointer to the newest release. A client
//! reads it, verifies it, and asks for the delta from the exact container it
//! runs by that container's hash. A miss is a plain 404 and the client
//! downloads the full container instead.
//!
//! Writers publish every release file before `index.json`, so a reader never
//! learns of a release whose files are not there yet.

use semver::Version;
use serde::Deserialize;
use serde::Serialize;

use crate::manifest::BundleManifest;
use crate::signing::ReleaseSignature;

/// The channel index file name.
pub const INDEX_FILE: &str = "index.json";

/// The container file name inside a release directory.
pub const RELEASE_FILE: &str = "release.lkb";

/// The signature file name inside a release directory.
pub const SIGNATURE_FILE: &str = "release.sig";

/// The directory of deltas inside a release directory.
pub const DELTAS_DIR: &str = "deltas";

/// The file extension of a delta.
pub const DELTA_EXTENSION: &str = "jsondelta";

/// The `index.json` format version.
pub const INDEX_FORMAT_VERSION: u32 = 1;

/// How long caches may serve `index.json` before revalidating: how quickly
/// devices behind a CDN see a new release.
pub const INDEX_CACHE_CONTROL: &str = "public, max-age=60";

/// Release files never change once published, so caches keep them forever.
pub const IMMUTABLE_CACHE_CONTROL: &str = "public, max-age=31536000, immutable";

/// The newest release on a channel, as `index.json` describes it.
///
/// It carries the whole manifest and its signature, so a client can verify
/// the release before downloading anything: the signature covers the app,
/// channel, version, build, mandatory flag, and every file hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelIndex {
	/// The index format version.
	pub format_version: u32,
	/// The manifest of the newest release.
	pub manifest: BundleManifest,
	/// The publisher signature over the manifest. Clients reject an index
	/// without one.
	#[serde(default)]
	pub signature: Option<ReleaseSignature>,
}

/// What a file in the layout holds, which decides how it is served.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ObjectKind {
	/// `index.json`, the only file that changes.
	Index,
	/// A release container.
	Release,
	/// A release signature.
	Signature,
	/// A delta between two containers.
	Delta,
}

impl ObjectKind {
	/// Maximum response body bytes accepted from a registry. Transports must
	/// enforce this while reading, including bodies without `Content-Length`.
	pub const fn max_bytes(self) -> u64 {
		match self {
			Self::Index => 2 * crate::manifest::MAX_MANIFEST_BYTES,
			Self::Signature => 4096,
			Self::Release | Self::Delta => crate::manifest::MAX_CONTAINER_BYTES,
		}
	}

	/// The `Content-Type` to serve the file with.
	pub fn content_type(self) -> &'static str {
		match self {
			Self::Release => "application/octet-stream",
			Self::Index | Self::Signature | Self::Delta => "application/json",
		}
	}

	/// The `Cache-Control` to serve the file with.
	pub fn cache_control(self) -> &'static str {
		match self {
			Self::Index => INDEX_CACHE_CONTROL,
			Self::Release | Self::Signature | Self::Delta => IMMUTABLE_CACHE_CONTROL,
		}
	}
}

/// The `/`-separated path of a file in the layout, relative to the registry
/// root.
///
/// The constructors are the only way to build one, and each one sanitizes or
/// validates its parts, so a key never contains `..`, an absolute path, or a
/// hidden file: it is safe to join onto a directory or a bucket prefix.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ObjectKey {
	path: String,
	kind: ObjectKind,
}

impl ObjectKey {
	/// The key of a channel's `index.json`.
	pub fn index(app: &str, channel: &str) -> Self {
		Self {
			path: format!("{}/{INDEX_FILE}", channel_path(app, channel)),
			kind: ObjectKind::Index,
		}
	}

	/// The key of a release's container.
	pub fn release(app: &str, channel: &str, version: &Version, build: u64) -> Self {
		Self {
			path: format!(
				"{}/{RELEASE_FILE}",
				release_path(app, channel, version, build)
			),
			kind: ObjectKind::Release,
		}
	}

	/// The key of a release's signature.
	pub fn signature(app: &str, channel: &str, version: &Version, build: u64) -> Self {
		Self {
			path: format!(
				"{}/{SIGNATURE_FILE}",
				release_path(app, channel, version, build)
			),
			kind: ObjectKind::Signature,
		}
	}

	/// The key of the delta that turns the container hashing to `base_hash`
	/// into this release's container.
	///
	/// Returns `None` unless `base_hash` is a lowercase hex SHA-256, as
	/// [`crate::hash_bytes`] produces.
	pub fn delta(
		app: &str,
		channel: &str,
		version: &Version,
		build: u64,
		base_hash: &str,
	) -> Option<Self> {
		is_sha256_hex(base_hash).then(|| {
			Self {
				path: format!(
					"{}/{DELTAS_DIR}/{}",
					release_path(app, channel, version, build),
					delta_file_name(base_hash)
				),
				kind: ObjectKind::Delta,
			}
		})
	}

	/// The key as a relative path.
	pub fn as_str(&self) -> &str {
		&self.path
	}

	/// What the file holds.
	pub fn kind(&self) -> ObjectKind {
		self.kind
	}

	/// The key as a URL path, each segment percent-encoded, so the `+` in a
	/// release directory reaches the server as `%2B` rather than being read as
	/// a space.
	pub fn url_path(&self) -> String {
		self.path
			.split('/')
			.map(encode_component)
			.collect::<Vec<_>>()
			.join("/")
	}
}

/// A file of the layout with its contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticObject {
	/// Where the file lives.
	pub key: ObjectKey,
	/// The file contents.
	pub bytes: Vec<u8>,
}

/// The directory name of a release: `<version>+<build>`.
pub fn release_name(version: &Version, build: u64) -> String {
	format!("{version}+{build}")
}

/// Parses a release directory name back into its version and build.
///
/// Only the canonical form [`release_name`] writes is accepted, so the parsed
/// release always maps back to the same directory.
pub fn parse_release_name(name: &str) -> Option<(Version, u64)> {
	let (version, build) = name.rsplit_once('+')?;
	let version = Version::parse(version).ok()?;
	let build = build.parse().ok()?;
	(release_name(&version, build) == name).then_some((version, build))
}

/// The file name of the delta from the container hashing to `base_hash`:
/// `<base hash>.jsondelta`.
pub(crate) fn delta_file_name(base_hash: &str) -> String {
	format!("{base_hash}.{DELTA_EXTENSION}")
}

/// Parses a delta file name, `<base hash>.jsondelta`, into the base hash.
pub fn parse_delta_file_name(name: &str) -> Option<&str> {
	let base_hash = name.strip_suffix(DELTA_EXTENSION)?.strip_suffix('.')?;
	is_sha256_hex(base_hash).then_some(base_hash)
}

/// The directory of a channel: `<app>/<channel>`, both sanitized.
pub(crate) fn channel_path(app: &str, channel: &str) -> String {
	format!("{}/{}", sanitize(app), sanitize(channel))
}

/// The directory of a release: `<app>/<channel>/<version>+<build>`.
pub(crate) fn release_path(app: &str, channel: &str, version: &Version, build: u64) -> String {
	format!(
		"{}/{}",
		channel_path(app, channel),
		release_name(version, build)
	)
}

/// Maps an app or channel name onto `[A-Za-z0-9_-]`, replacing everything
/// else with `_`, so it is a single safe path segment.
fn sanitize(component: &str) -> String {
	let sanitized: String = component
		.chars()
		.map(|character| {
			if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
				character
			} else {
				'_'
			}
		})
		.collect();
	if sanitized.is_empty() {
		"_".to_owned()
	} else {
		sanitized
	}
}

fn is_sha256_hex(hash: &str) -> bool {
	hash.len() == 64
		&& hash
			.bytes()
			.all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Percent-encodes one URL path or query component: everything except the
/// RFC 3986 unreserved characters is escaped, so a version like `1.0.0+abc`
/// survives decoding instead of turning into `1.0.0 abc`.
pub(crate) fn encode_component(component: &str) -> String {
	const HEX: &[u8; 16] = b"0123456789ABCDEF";
	let mut encoded = String::with_capacity(component.len());
	for byte in component.bytes() {
		if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
			encoded.push(char::from(byte));
		} else {
			encoded.push('%');
			encoded.push(char::from(HEX[usize::from(byte >> 4)]));
			encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
		}
	}
	encoded
}

#[cfg(test)]
mod tests {
	use super::*;

	const HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

	#[test]
	fn keys_follow_the_layout() {
		let version = Version::parse("1.2.0-beta.1+abc").expect("version");
		assert_eq!(
			ObjectKey::index("my app", "production").as_str(),
			"my_app/production/index.json"
		);
		assert_eq!(
			ObjectKey::release("app", "production", &version, 3).as_str(),
			"app/production/1.2.0-beta.1+abc+3/release.lkb"
		);
		assert_eq!(
			ObjectKey::signature("app", "production", &version, 3).as_str(),
			"app/production/1.2.0-beta.1+abc+3/release.sig"
		);
		assert_eq!(
			ObjectKey::delta("app", "production", &version, 3, HASH)
				.expect("valid hash")
				.as_str(),
			format!("app/production/1.2.0-beta.1+abc+3/deltas/{HASH}.jsondelta")
		);
	}

	#[test]
	fn url_paths_escape_the_build_separator() {
		let key = ObjectKey::release("app", "production", &Version::new(1, 0, 0), 2);
		assert_eq!(key.url_path(), "app/production/1.0.0%2B2/release.lkb");
	}

	#[test]
	fn names_that_could_escape_a_directory_are_sanitized_or_rejected() {
		assert_eq!(
			ObjectKey::index("..", "../etc").as_str(),
			"__/___etc/index.json"
		);
		assert_eq!(ObjectKey::index("", "").as_str(), "_/_/index.json");
		for hash in [
			"",
			"..",
			&HASH.to_uppercase(),
			&HASH[1..],
			"../../../index.json",
		] {
			assert!(
				ObjectKey::delta("app", "production", &Version::new(1, 0, 0), 1, hash).is_none(),
				"{hash:?} must be rejected"
			);
		}
	}

	#[test]
	fn release_names_round_trip_only_in_canonical_form() {
		let version = Version::parse("1.0.0+abc").expect("version");
		assert_eq!(
			parse_release_name(&release_name(&version, 7)),
			Some((version, 7))
		);
		for name in [
			"1.0.0",
			"1.0.0+",
			"1.0.0+x",
			"../1.0.0+1",
			"1.0.0+01",
			"v1.0.0+1",
		] {
			assert_eq!(parse_release_name(name), None, "{name:?}");
		}
	}

	#[test]
	fn delta_file_names_carry_a_sha256() {
		assert_eq!(
			parse_delta_file_name(&format!("{HASH}.jsondelta")),
			Some(HASH)
		);
		assert_eq!(parse_delta_file_name(&format!("{HASH}jsondelta")), None);
		assert_eq!(parse_delta_file_name(&format!("{HASH}.json")), None);
		assert_eq!(parse_delta_file_name("x.jsondelta"), None);
	}
}
