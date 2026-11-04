//! Read-only agent access. This intentionally returns one instance's evidence
//! and preserves that limitation instead of inventing a fleet aggregate.
use anyhow::Context;
use anyhow::Result;
use anyhow::ensure;
use leptoskit_analytics::Evidence;
use leptoskit_config::infra::InfraConfig;

pub(super) fn query(c: &InfraConfig) -> Result<()> {
	let plugin = c
		.plugins
		.analytics
		.as_ref()
		.context("enable [infra.plugins.analytics] first")?;
	let key = std::env::var("LEPTOSKIT_ANALYTICS_READ_KEY").map_err(|_| {
		anyhow::anyhow!(
			"set LEPTOSKIT_ANALYTICS_READ_KEY; this is a read-only analytics credential"
		)
	})?;
	let evidence = fetch(&format!("https://{}/_leptoskit/analytics", c.domain), &key)?;
	ensure!(
		evidence.snapshot.resource.service == c.name
			&& evidence.snapshot.resource.environment == plugin.environment,
		"analytics identity differs from the selected service/environment"
	);
	println!("{}", serde_json::to_string_pretty(&evidence)?);
	Ok(())
}

fn fetch(url: &str, key: &str) -> Result<Evidence> {
	ensure!(
		(32..=256).contains(&key.len()) && key.bytes().all(|b| (33..127).contains(&b)),
		"invalid analytics read key"
	);
	let agent: ureq::Agent = ureq::Agent::config_builder()
		.timeout_global(Some(std::time::Duration::from_secs(10)))
		.max_redirects(0)
		.build()
		.into();
	let mut response = agent
		.get(url)
		.header("Authorization", format!("Bearer {key}"))
		.call()
		.map_err(|_| {
			anyhow::anyhow!(
				"analytics request failed; check the read key, app integration and network"
			)
		})?;
	ensure!(
		response.status() == 200,
		"analytics API did not return evidence"
	);
	let body = response
		.body_mut()
		.with_config()
		.limit(16 * 1024 * 1024)
		.read_to_string()
		.context("couldn't read bounded analytics response")?;
	let evidence: Evidence =
		serde_json::from_str(&body).context("invalid analytics evidence response")?;
	ensure!(
		evidence.snapshot.format_version == 1,
		"unsupported analytics format version"
	);
	ensure!(
		!evidence.infrastructure_changes_authorized,
		"analytics cannot authorize infrastructure changes"
	);
	Ok(evidence)
}

#[cfg(test)]
mod tests {
	use std::io::BufRead as _;
	use std::io::Write as _;
	use std::net::TcpListener;

	use super::*;

	#[test]
	fn a_redirect_is_not_followed_and_credentials_are_not_in_the_error() {
		let listener = TcpListener::bind("127.0.0.1:0").unwrap();
		let address = listener.local_addr().unwrap();
		let key = "test-only-analytics-read-key-0123456789";
		let server = std::thread::spawn(move || {
			let (mut socket, _) = listener.accept().unwrap();
			socket
				.set_read_timeout(Some(std::time::Duration::from_secs(5)))
				.unwrap();
			let mut reader = std::io::BufReader::new(&socket);
			let mut request = String::new();
			loop {
				let mut line = String::new();
				if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
					break;
				}
				request.push_str(&line);
			}
			let request = request.to_ascii_lowercase();
			assert!(request.contains(&format!("authorization: bearer {key}")));
			socket.write_all(b"HTTP/1.1 302 Found\r\nLocation: https://must-not-follow.example/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
		});
		let error = fetch(&format!("http://{address}/_leptoskit/analytics"), key).unwrap_err();
		assert!(!error.to_string().contains(key));
		server.join().unwrap();
	}
}
