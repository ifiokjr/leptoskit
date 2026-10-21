//! An S3-compatible bucket for `leptoskit codepush publish` and `pull`:
//! Backblaze B2,
//! Cloudflare R2, AWS S3, `MinIO`, or anything else that speaks the S3 API.
//!
//! Requests are presigned with `SigV4` by `rusty-s3` and sent with `ureq`. The
//! credentials come only from the environment and never appear in output:
//! errors name the object key and the status, never the signed URL.

use std::time::Duration;

use anyhow::Context;
use anyhow::Result;
use leptoskit_codepush::Bucket;
use leptoskit_codepush::CodePushError;
use leptoskit_codepush::ObjectKey;
use leptoskit_codepush::StaticObject;
use rusty_s3::Credentials;
use rusty_s3::S3Action;
use rusty_s3::UrlStyle;

/// The environment variable holding the S3 access key id.
pub const ACCESS_KEY_ID_ENV: &str = "AWS_ACCESS_KEY_ID";

/// The environment variable holding the S3 secret access key.
pub const SECRET_ACCESS_KEY_ENV: &str = "AWS_SECRET_ACCESS_KEY";

/// The environment variable holding an optional session token, for
/// temporary credentials.
pub const SESSION_TOKEN_ENV: &str = "AWS_SESSION_TOKEN";

/// How long each presigned request stays valid. Every request is signed just
/// before it is sent, so this only needs to cover one upload.
const PRESIGN_DURATION: Duration = Duration::from_mins(15);

/// The longest any one request may take, a large container upload included.
const REQUEST_TIMEOUT: Duration = Duration::from_mins(10);

/// The largest object a publish reads back, well above any bundle.
const MAX_OBJECT_SIZE: u64 = 1 << 30;

/// Where a bucket lives and how to address it: the bucket flags of
/// `leptoskit codepush publish` and `pull`.
#[derive(Debug, Clone, clap::Args)]
pub struct S3Location {
	/// The S3 API endpoint, such as
	/// `https://<account id>.r2.cloudflarestorage.com` or
	/// `https://s3.us-west-004.backblazeb2.com`.
	#[arg(long, value_name = "URL")]
	pub endpoint: String,
	/// The bucket name.
	#[arg(long)]
	pub bucket: String,
	/// The bucket's region; `auto` suits R2.
	#[arg(long, default_value = "auto")]
	pub region: String,
	/// A key prefix inside the bucket, so one bucket can hold more than the
	/// registry.
	#[arg(long)]
	pub prefix: Option<String>,
	/// Addresses the bucket by path (`<endpoint>/<bucket>`) instead of by
	/// subdomain, for `MinIO` and other local servers.
	#[arg(long)]
	pub path_style: bool,
}

/// An S3-compatible bucket.
///
/// Deliberately not `Debug`: it holds the secret key.
pub struct S3Bucket {
	bucket: rusty_s3::Bucket,
	credentials: Credentials,
	prefix: String,
	agent: ureq::Agent,
}

impl S3Bucket {
	/// A bucket at `location`, signing with credentials from
	/// [`ACCESS_KEY_ID_ENV`], [`SECRET_ACCESS_KEY_ENV`], and optionally
	/// [`SESSION_TOKEN_ENV`].
	pub fn from_env(location: &S3Location) -> Result<Self> {
		let variable = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
		let (Some(key), Some(secret)) =
			(variable(ACCESS_KEY_ID_ENV), variable(SECRET_ACCESS_KEY_ENV))
		else {
			anyhow::bail!(
				"publishing needs the bucket's S3 credentials: set {ACCESS_KEY_ID_ENV} and \
				 {SECRET_ACCESS_KEY_ENV} (and {SESSION_TOKEN_ENV} for temporary credentials)"
			);
		};
		let credentials = match variable(SESSION_TOKEN_ENV) {
			Some(token) => Credentials::new_with_token(key, secret, token),
			None => Credentials::new(key, secret),
		};
		Self::new(location, credentials)
	}

	/// A bucket at `location`, signing with `credentials`.
	pub fn new(location: &S3Location, credentials: Credentials) -> Result<Self> {
		let endpoint = location
			.endpoint
			.parse()
			.with_context(|| format!("{} is not a valid endpoint URL", location.endpoint))?;
		let style = if location.path_style {
			UrlStyle::Path
		} else {
			UrlStyle::VirtualHost
		};
		let bucket = rusty_s3::Bucket::new(
			endpoint,
			style,
			location.bucket.clone(),
			location.region.clone(),
		)
		.with_context(|| format!("could not address bucket {}", location.bucket))?;
		let agent = ureq::Agent::config_builder()
			.http_status_as_error(false)
			.timeout_global(Some(REQUEST_TIMEOUT))
			.build()
			.into();
		Ok(Self {
			bucket,
			credentials,
			prefix: normalize_prefix(location.prefix.as_deref())?,
			agent,
		})
	}

	/// The bucket key of a registry file.
	fn object_name(&self, key: &ObjectKey) -> String {
		format!("{}{}", self.prefix, key.as_str())
	}
}

impl Bucket for S3Bucket {
	fn get(&mut self, key: &ObjectKey) -> Result<Option<Vec<u8>>, CodePushError> {
		let name = self.object_name(key);
		let url = self
			.bucket
			.get_object(Some(&self.credentials), &name)
			.sign(PRESIGN_DURATION);
		let mut response = self
			.agent
			.get(url.as_str())
			.call()
			.map_err(|error| request_error("GET", &name, &error))?;
		match response.status().as_u16() {
			200 => {
				response
					.body_mut()
					.with_config()
					.limit(MAX_OBJECT_SIZE)
					.read_to_vec()
					.map(Some)
					.map_err(|error| request_error("GET", &name, &error))
			}
			404 => Ok(None),
			status => Err(status_error("GET", &name, status)),
		}
	}

	fn put(&mut self, object: &StaticObject) -> Result<(), CodePushError> {
		let name = self.object_name(&object.key);
		let kind = object.key.kind();
		let mut action = self.bucket.put_object(Some(&self.credentials), &name);
		// Signed, so a proxy cannot change how CDNs cache the file.
		action
			.headers_mut()
			.insert("content-type", kind.content_type());
		action
			.headers_mut()
			.insert("cache-control", kind.cache_control());
		let url = action.sign(PRESIGN_DURATION);
		let response = self
			.agent
			.put(url.as_str())
			.header("content-type", kind.content_type())
			.header("cache-control", kind.cache_control())
			.send(&object.bytes[..])
			.map_err(|error| request_error("PUT", &name, &error))?;
		match response.status().as_u16() {
			200..=299 => Ok(()),
			status => Err(status_error("PUT", &name, status)),
		}
	}
}

/// Trims a key prefix to `segment/.../` form, rejecting segments that are
/// empty or that could escape it.
fn normalize_prefix(prefix: Option<&str>) -> Result<String> {
	let Some(prefix) = prefix.map(|prefix| prefix.trim_matches('/')) else {
		return Ok(String::new());
	};
	if prefix.is_empty() {
		return Ok(String::new());
	}
	anyhow::ensure!(
		prefix
			.split('/')
			.all(|segment| !segment.is_empty() && segment != "." && segment != ".."),
		"the bucket prefix {prefix:?} must be plain path segments"
	);
	Ok(format!("{prefix}/"))
}

/// A failed request. The error is described by kind only: some `ureq` errors
/// quote the URL, and a presigned URL carries a signature.
fn request_error(method: &str, name: &str, error: &ureq::Error) -> CodePushError {
	let reason = match error {
		ureq::Error::Io(error) => error.to_string(),
		ureq::Error::Timeout(_) => "the request timed out".to_owned(),
		ureq::Error::HostNotFound => "the endpoint host was not found".to_owned(),
		ureq::Error::ConnectionFailed => "the connection failed".to_owned(),
		ureq::Error::BodyExceedsLimit(_) => "the object is too large".to_owned(),
		ureq::Error::Tls(error) => format!("TLS failed: {error}"),
		_ => "the request failed".to_owned(),
	};
	CodePushError::Transport(format!("{method} {name}: {reason}"))
}

fn status_error(method: &str, name: &str, status: u16) -> CodePushError {
	let hint = match status {
		401 | 403 => {
			"; check the credentials and their access to the bucket (on AWS a missing object also \
			 reads as 403 unless the credentials may list the bucket)"
		}
		_ => "",
	};
	CodePushError::Transport(format!("{method} {name}: HTTP {status}{hint}"))
}

#[cfg(test)]
mod tests {
	use leptoskit_codepush::ReleaseSigningKey;
	use leptoskit_codepush::ReleaseStore;
	use leptoskit_codepush::publish_channel;
	use semver::Version;

	use super::*;

	impl S3Bucket {
		/// Deletes an object, so the real-bucket test leaves nothing behind.
		fn delete(&mut self, key: &ObjectKey) {
			let name = self.object_name(key);
			let url = self
				.bucket
				.delete_object(Some(&self.credentials), &name)
				.sign(PRESIGN_DURATION);
			let response = self.agent.delete(url.as_str()).call().expect("delete");
			assert!(response.status().is_success(), "DELETE {name}");
		}
	}

	/// Publishes two releases to a real bucket under a fresh prefix, reads
	/// every file back, and deletes them again.
	///
	/// ```bash
	/// LEPTOSKIT_S3_TEST_ENDPOINT=https://s3.us-west-004.backblazeb2.com \
	/// LEPTOSKIT_S3_TEST_BUCKET=my-bucket LEPTOSKIT_S3_TEST_REGION=us-west-004 \
	/// AWS_ACCESS_KEY_ID=... AWS_SECRET_ACCESS_KEY=... \
	/// cargo test -p leptoskit_cli publishes_to_a_real_bucket -- --ignored
	/// ```
	#[test]
	#[ignore = "needs a real S3-compatible bucket: set LEPTOSKIT_S3_TEST_ENDPOINT, \
	            LEPTOSKIT_S3_TEST_BUCKET, and the AWS_* credentials"]
	fn publishes_to_a_real_bucket() {
		let variable = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
		let unique = std::time::SystemTime::now()
			.duration_since(std::time::UNIX_EPOCH)
			.expect("clock")
			.as_nanos();
		let location = S3Location {
			endpoint: variable("LEPTOSKIT_S3_TEST_ENDPOINT").expect("LEPTOSKIT_S3_TEST_ENDPOINT"),
			bucket: variable("LEPTOSKIT_S3_TEST_BUCKET").expect("LEPTOSKIT_S3_TEST_BUCKET"),
			region: variable("LEPTOSKIT_S3_TEST_REGION").unwrap_or_else(|| "auto".to_owned()),
			prefix: Some(format!("leptoskit-test-{unique}")),
			path_style: variable("LEPTOSKIT_S3_TEST_PATH_STYLE").is_some(),
		};
		let mut bucket = S3Bucket::from_env(&location).expect("credentials");

		let scratch = std::env::temp_dir().join(format!("leptoskit_s3_test_{unique}"));
		let store = ReleaseStore::open(scratch.join("releases")).expect("store");
		let signer = ReleaseSigningKey::from_seed(&[0x5a; 32]);
		for build in 1..=2 {
			let dir = scratch.join(format!("bundle{build}"));
			std::fs::create_dir_all(&dir).expect("bundle");
			let mut wasm = vec![1u8; 32 * 1024];
			wasm.extend_from_slice(format!("build {build}").as_bytes());
			std::fs::write(dir.join("app.wasm"), wasm).expect("wasm");
			store
				.ingest_directory(
					&signer,
					"app",
					"production",
					&Version::new(1, 0, 0),
					build,
					&dir,
					false,
				)
				.expect("ingest");
		}

		let report = publish_channel(&store, "app", "production", &mut bucket).expect("publish");
		assert_eq!(report.releases, ["1.0.0+1", "1.0.0+2"]);

		let index = ObjectKey::index("app", "production");
		let mut objects = Vec::new();
		for record in store.list("app", "production").expect("list") {
			objects.extend(store.release_objects(&record).expect("objects"));
		}
		objects.push(StaticObject {
			bytes: store.read_object(&index).expect("read").expect("index"),
			key: index,
		});
		for object in &objects {
			assert_eq!(
				bucket.get(&object.key).expect("get").as_ref(),
				Some(&object.bytes),
				"{}",
				object.key.as_str()
			);
		}
		for object in &objects {
			bucket.delete(&object.key);
		}
		let _ = std::fs::remove_dir_all(&scratch);
	}

	#[test]
	fn prefixes_are_normalized_and_validated() {
		assert_eq!(normalize_prefix(None).expect("none"), "");
		assert_eq!(normalize_prefix(Some("/")).expect("root"), "");
		assert_eq!(
			normalize_prefix(Some("/codepush/v1/")).expect("prefix"),
			"codepush/v1/"
		);
		for prefix in ["a//b", "../a", "a/./b"] {
			assert!(normalize_prefix(Some(prefix)).is_err(), "{prefix:?}");
		}
	}
}
