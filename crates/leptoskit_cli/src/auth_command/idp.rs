//! `leptoskit auth idp`: "Sign in with your app".
//!
//! `auth idp client` registers, lists, and removes the apps that may sign
//! people in with yours. The apps live in the app's own database, which only
//! the app's server knows how to reach, so the command builds the server and
//! runs it with the command in `LEPTOSKIT_IDP_COMMAND`. The server's call to
//! `Idp::run_command_if_requested()` carries it out, writes the answer to the
//! file `LEPTOSKIT_IDP_OUT` names, and exits.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;
use std::time::Duration;
use std::time::Instant;

use anyhow::Context;
use anyhow::Result;
use camino::Utf8PathBuf;
use clap::Subcommand;
use leptoskit_config::LeptoskitConfig;
use leptoskit_idp::COMMAND_ENV;
use leptoskit_idp::COMMAND_OUT_ENV;
use leptoskit_idp::ClientAuth;
use leptoskit_idp::ClientCommand;
use leptoskit_idp::ClientCommandOutput;
use leptoskit_idp::ClientInfo;
use leptoskit_idp::GrantType;
use leptoskit_idp::NewClientSpec;
use leptoskit_idp::SigningAlgorithm;

use crate::wasm;

/// How long the server may take to carry out a command.
const TIMEOUT: Duration = Duration::from_mins(5);

/// `auth idp` subcommands.
#[derive(Debug, Subcommand)]
pub enum IdpCommand {
	/// Registers, lists, and removes apps.
	///
	/// The server must call `idp.run_command_if_requested().await` once its
	/// `Idp` is built: these commands build the server and run it to reach
	/// the app's database.
	Client {
		#[command(subcommand)]
		command: ClientArgs,
	},
}

/// `auth idp client` subcommands.
#[derive(Debug, Subcommand)]
pub enum ClientArgs {
	/// Registers an app, printing its client ID and, this once, its secret.
	Add {
		/// The app's name, as the consent screen shows it.
		name: String,
		/// Where codes may be sent, compared exactly (repeatable). Needed for
		/// the `code` grant.
		#[arg(long = "redirect")]
		redirects: Vec<String>,
		/// Where people may be sent after signing out (repeatable).
		#[arg(long = "post-logout-redirect")]
		post_logout_redirects: Vec<String>,
		/// The scopes it may ask for, comma-separated, such as
		/// `openid,profile,posts:read`.
		#[arg(long, value_delimiter = ',', required = true)]
		scopes: Vec<String>,
		/// How it gets tokens (repeatable): `code`, sending people to sign in
		/// in a browser; `device`, showing a code to enter on another device,
		/// for CLIs and TVs; or `machine`, acting as itself.
		#[arg(long = "grant", value_enum, default_values_t = [GrantArg::Code])]
		grants: Vec<GrantArg>,
		/// A public app, such as a native or single-page app or a CLI: no
		/// secret, and PKCE protects its codes.
		#[arg(long, conflicts_with_all = ["secret_post", "jwks"])]
		public: bool,
		/// Sends its secret in the form body instead of with HTTP Basic.
		#[arg(long, conflicts_with = "jwks")]
		secret_post: bool,
		/// Authenticates with `private_key_jwt` instead of a secret: the JSON
		/// file holding its public keys as a JWK set.
		#[arg(long, value_name = "FILE")]
		jwks: Option<Utf8PathBuf>,
		/// Signs its ID tokens with RS256 instead of ES256. The server needs an
		/// RSA key (`rsa_key_env` in `[auth.idp]`).
		#[arg(long)]
		rs256: bool,
		/// One of your own apps: it sees the account ID as `sub`, and needs no
		/// consent.
		#[arg(long)]
		first_party: bool,
		/// May introspect other apps' tokens, as a resource server does.
		#[arg(long)]
		introspect: bool,
		/// Must bind every access token to its own key with DPoP (RFC 9449),
		/// so a stolen token is useless without the key. The server needs
		/// `dpop = true` in `[auth.idp]`.
		#[arg(long)]
		dpop: bool,
	},
	/// Lists every registered app.
	List,
	/// Removes an app, ending every grant and token it holds.
	Remove {
		/// The app's client ID.
		client_id: String,
	},
	/// Replaces an app's secret, printing the new one this once. Tokens the
	/// app got as itself end; people's sign-ins with it carry on.
	RotateSecret {
		/// The app's client ID.
		client_id: String,
	},
}

/// How an app gets tokens, as `--grant` names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum GrantArg {
	/// The authorization code flow.
	Code,
	/// The device flow (RFC 8628).
	Device,
	/// Client credentials: the app acting as itself.
	Machine,
}

impl From<GrantArg> for GrantType {
	fn from(grant: GrantArg) -> Self {
		match grant {
			GrantArg::Code => Self::AuthorizationCode,
			GrantArg::Device => Self::DeviceCode,
			GrantArg::Machine => Self::ClientCredentials,
		}
	}
}

/// Runs an `auth` command.
///
/// # Errors
///
/// When the server can't be built or run, or refuses the command.
pub fn run(command: IdpCommand, config: &LeptoskitConfig, root: PathBuf) -> Result<()> {
	let IdpCommand::Client { command } = command;
	let command = client_command(command)?;
	let root = Utf8PathBuf::from_path_buf(root)
		.map_err(|_| anyhow::anyhow!("project path is not UTF-8"))?;
	let binary = server_binary(config, &root)?;
	let options = crate::build::server_options(config, false)?;
	let output = run_server(
		binary.as_std_path(),
		root.as_std_path(),
		&options.env,
		&command,
		TIMEOUT,
	)?;
	let (text, failed) = render(&output);

	if failed {
		anyhow::bail!("{text}");
	}

	print!("{text}");
	Ok(())
}

/// The command the server carries out.
fn client_command(args: ClientArgs) -> Result<ClientCommand> {
	Ok(match args {
		ClientArgs::Add {
			name,
			redirects,
			post_logout_redirects,
			scopes,
			grants,
			public,
			secret_post,
			jwks,
			rs256,
			first_party,
			introspect,
			dpop,
		} => {
			let jwks = jwks
				.map(|path| -> Result<serde_json::Value> {
					let text = std::fs::read_to_string(&path)
						.with_context(|| format!("could not read {path}"))?;
					serde_json::from_str(&text)
						.with_context(|| format!("{path} isn't a JSON JWK set"))
				})
				.transpose()?;
			let auth_method = if public {
				ClientAuth::None
			} else if jwks.is_some() {
				ClientAuth::PrivateKeyJwt
			} else if secret_post {
				ClientAuth::SecretPost
			} else {
				ClientAuth::SecretBasic
			};

			ClientCommand::Add(NewClientSpec {
				name,
				redirect_uris: redirects,
				post_logout_redirect_uris: post_logout_redirects,
				scopes,
				auth_method,
				grant_types: grants.into_iter().map(GrantType::from).collect(),
				jwks,
				id_token_alg: if rs256 {
					SigningAlgorithm::Rs256
				} else {
					SigningAlgorithm::Es256
				},

				first_party,
				introspection: introspect,
				dpop_bound: dpop,
			})
		}
		ClientArgs::List => ClientCommand::List,
		ClientArgs::Remove { client_id } => ClientCommand::Remove { client_id },
		ClientArgs::RotateSecret { client_id } => ClientCommand::RotateSecret { client_id },
	})
}

fn server_binary(config: &LeptoskitConfig, root: &Utf8PathBuf) -> Result<Utf8PathBuf> {
	let metadata = wasm::cargo_metadata(root.as_std_path())?;
	metadata.package(&config.app.name).with_context(|| {
		format!(
			"app.name `{}` in leptoskit.toml must name the server package",
			config.app.name
		)
	})?;
	let options = crate::build::server_options(config, false)?;
	let binary = crate::build::compile_server(root, &config.app.name, &options)?
		.with_context(|| format!("`{}` built no server binary", config.app.name))?;
	Utf8PathBuf::from_path_buf(binary)
		.map_err(|_| anyhow::anyhow!("the server binary path is not UTF-8"))
}

/// Runs the server with `command`, and reads its answer.
fn run_server(
	binary: &Path,
	root: &Path,
	env: &BTreeMap<String, String>,
	command: &ClientCommand,
	timeout: Duration,
) -> Result<ClientCommandOutput> {
	// The answer may hold a client secret, so it goes in a directory only
	// this user can read, removed when this returns.
	let directory = tempfile::Builder::new()
		.prefix("leptoskit-idp-")
		.tempdir()
		.context("could not create a temporary directory")?;
	let out = directory.path().join("answer.json");
	let mut child = Command::new(binary)
		.current_dir(root)
		.envs(env)
		.env(COMMAND_ENV, serde_json::to_string(command)?)
		.env(COMMAND_OUT_ENV, &out)
		.stdin(Stdio::null())
		.stdout(Stdio::null())
		.stderr(Stdio::piped())
		.spawn()
		.with_context(|| format!("could not run {}", binary.display()))?;
	let started = Instant::now();

	let status = loop {
		if let Some(status) = child.try_wait().context("waiting for the server")? {
			break status;
		}

		if started.elapsed() > timeout {
			let _ = child.kill();
			let _ = child.wait();
			anyhow::bail!(
				"{} didn't answer within {}s. Call `idp.run_command_if_requested().await` once \
				 the Idp is built, before the server binds its port.",
				binary.display(),
				timeout.as_secs()
			);
		}

		std::thread::sleep(Duration::from_millis(50));
	};

	let mut stderr = String::new();

	if let Some(mut pipe) = child.stderr.take() {
		std::io::Read::read_to_string(&mut pipe, &mut stderr)
			.context("reading the server's diagnostics")?;
	}

	let Ok(text) = std::fs::read_to_string(&out) else {
		anyhow::bail!(
			"{} exited with {status} without answering. Call \
			 `idp.run_command_if_requested().await` once the Idp is built.\n{}",
			binary.display(),
			stderr.trim()
		);
	};
	serde_json::from_str(&text).context("reading the server's answer")
}

/// The answer, for people, and whether it's a failure.
fn render(output: &ClientCommandOutput) -> (String, bool) {
	let mut text = String::new();

	match output {
		ClientCommandOutput::Added { client, secret } => {
			let _ = writeln!(text, "Registered {}.\n", client.name);
			describe(&mut text, client);
			match secret {
				Some(secret) => {
					let _ = writeln!(
						text,
						"\n  client secret: {secret}\n\nThe secret is shown this once; only its \
						 hash is kept. Store it in the app's secrets now."
					);
				}

				None if client.auth_method == ClientAuth::PrivateKeyJwt => {
					let _ = writeln!(
						text,
						"\nIt signs in with its own key (private_key_jwt): there's no secret."
					);
				}

				None => {
					let _ = writeln!(text, "\nA public app: it has no secret, and uses PKCE.");
				}
			}
		}

		ClientCommandOutput::Listed { clients } if clients.is_empty() => {
			text.push_str("No apps are registered.\n");
		}

		ClientCommandOutput::Listed { clients } => {
			for (index, client) in clients.iter().enumerate() {
				if index > 0 {
					text.push('\n');
				}

				let _ = writeln!(text, "{}", client.name);
				describe(&mut text, client);
			}
		}
		ClientCommandOutput::Removed { removed: true } => {
			text.push_str("Removed the app, with every grant and token it held.\n");
		}
		ClientCommandOutput::Removed { removed: false } => {
			text.push_str("No app has that client ID.\n");
			return (text, true);
		}
		ClientCommandOutput::Rotated { secret } => {
			let _ = writeln!(
				text,
				"  client secret: {secret}\n\nThe old secret no longer works, and the tokens the \
				 app got as itself have ended. The new secret is shown this once; store it in the \
				 app's secrets now."
			);
		}
		ClientCommandOutput::Failed { message } => {
			text.push_str(message);
			return (text, true);
		}
	}

	(text, false)
}

fn describe(text: &mut String, client: &ClientInfo) {
	let _ = writeln!(text, "  client id: {}", client.client_id);
	let _ = writeln!(text, "  auth: {}", client.auth_method.as_str());
	let grants: Vec<&str> = client.grant_types.iter().map(GrantType::as_str).collect();
	let _ = writeln!(text, "  grants: {}", grants.join(", "));

	if !client.redirect_uris.is_empty() {
		let _ = writeln!(text, "  redirects: {}", client.redirect_uris.join(", "));
	}

	if client.id_token_alg == SigningAlgorithm::Rs256 {
		let _ = writeln!(text, "  ID tokens: RS256");
	}

	if !client.post_logout_redirect_uris.is_empty() {
		let _ = writeln!(
			text,
			"  post-logout redirects: {}",
			client.post_logout_redirect_uris.join(", ")
		);
	}

	let _ = writeln!(text, "  scopes: {}", client.scopes.join(" "));

	if client.first_party {
		let _ = writeln!(text, "  first party: sees account IDs, skips consent");
	}

	if client.introspection {
		let _ = writeln!(text, "  may introspect other apps' tokens");
	}

	if client.dpop_bound {
		let _ = writeln!(text, "  tokens bound to its key with DPoP");
	}

	if client.self_registered {
		let _ = writeln!(text, "  registered itself (dynamic registration)");
	}
}

#[cfg(test)]
mod tests {
	use clap::Parser;

	use super::*;
	use crate::auth_command::AuthCommand;

	#[derive(Debug, Parser)]
	struct Harness {
		#[command(subcommand)]
		command: AuthCommand,
	}

	fn parse(args: &[&str]) -> ClientCommand {
		let harness =
			Harness::try_parse_from(std::iter::once("auth").chain(args.iter().copied())).unwrap();
		let AuthCommand::Idp {
			command: IdpCommand::Client { command },
		} = harness.command
		else {
			panic!("not an `auth idp client` command");
		};
		client_command(command).unwrap()
	}

	fn client(name: &str) -> ClientInfo {
		ClientInfo {
			client_id: "abc".into(),
			name: name.into(),
			auth_method: ClientAuth::SecretBasic,
			grant_types: leptoskit_idp::default_grant_types(),
			jwks: None,
			id_token_alg: SigningAlgorithm::Rs256,
			redirect_uris: vec!["https://partner.example/cb".into()],
			post_logout_redirect_uris: vec!["https://partner.example/out".into()],
			scopes: vec!["openid".into(), "posts:read".into()],
			first_party: true,
			introspection: true,
			dpop_bound: false,
			self_registered: false,
			created_at: chrono::DateTime::default(),
		}
	}

	#[test]
	fn arguments_become_the_command() {
		assert_eq!(
			parse(&[
				"idp",
				"client",
				"add",
				"Partner App",
				"--redirect",
				"https://partner.example/cb",
				"--scopes",
				"openid,profile,posts:read",
				"--public",
			]),
			ClientCommand::Add(NewClientSpec {
				name: "Partner App".into(),
				redirect_uris: vec!["https://partner.example/cb".into()],
				post_logout_redirect_uris: Vec::new(),
				scopes: vec!["openid".into(), "profile".into(), "posts:read".into()],
				auth_method: ClientAuth::None,
				grant_types: vec![GrantType::AuthorizationCode],
				jwks: None,
				id_token_alg: SigningAlgorithm::Es256,
				first_party: false,
				introspection: false,
				dpop_bound: false,
			})
		);
		let ClientCommand::Add(cli) = parse(&[
			"idp",
			"client",
			"add",
			"Notes CLI",
			"--scopes",
			"notes:read",
			"--public",
			"--grant",
			"device",
		]) else {
			panic!("an add");
		};
		assert_eq!(cli.grant_types, [GrantType::DeviceCode]);
		assert_eq!(cli.redirect_uris, [] as [String; 0]);

		let directory = tempfile::tempdir().unwrap();
		let jwks = directory.path().join("jwks.json");
		std::fs::write(&jwks, r#"{"keys":[{"kty":"EC"}]}"#).unwrap();
		let ClientCommand::Add(keyed) = parse(&[
			"idp",
			"client",
			"add",
			"Worker",
			"--scopes",
			"notes:read",
			"--grant",
			"machine",
			"--grant",
			"code",
			"--redirect",
			"https://a.example/cb",
			"--jwks",
			jwks.to_str().unwrap(),
			"--rs256",
		]) else {
			panic!("an add");
		};
		assert_eq!(keyed.auth_method, ClientAuth::PrivateKeyJwt);
		assert_eq!(
			keyed.grant_types,
			[GrantType::ClientCredentials, GrantType::AuthorizationCode]
		);
		assert_eq!(keyed.jwks.unwrap()["keys"][0]["kty"], "EC");

		assert_eq!(keyed.id_token_alg, SigningAlgorithm::Rs256);
		assert_eq!(
			parse(&["idp", "client", "rotate-secret", "abc"]),
			ClientCommand::RotateSecret {
				client_id: "abc".into()
			}
		);
		for bad in [
			"{not json".to_owned(),
			directory.path().join("missing.json").display().to_string(),
		] {
			let path = if bad.starts_with('{') {
				std::fs::write(directory.path().join("bad.json"), &bad).unwrap();
				directory.path().join("bad.json").display().to_string()
			} else {
				bad
			};

			let harness = Harness::try_parse_from([
				"auth", "idp", "client", "add", "X", "--scopes", "openid", "--jwks", &path,
			])
			.unwrap();
			let AuthCommand::Idp {
				command: IdpCommand::Client { command },
			} = harness.command
			else {
				panic!("not an `auth idp client` command");
			};
			assert!(client_command(command).is_err(), "{path}");
		}
		let ClientCommand::Add(spec) = parse(&[
			"idp",
			"client",
			"add",
			"API",
			"--redirect",
			"https://a.example/cb",
			"--post-logout-redirect",
			"https://a.example/out",
			"--scopes",
			"openid",
			"--secret-post",
			"--first-party",
			"--introspect",
			"--dpop",
		]) else {
			panic!("an add");
		};
		assert_eq!(spec.auth_method, ClientAuth::SecretPost);
		assert_eq!(spec.post_logout_redirect_uris, ["https://a.example/out"]);
		assert!(spec.first_party && spec.introspection && spec.dpop_bound);
		assert_eq!(parse(&["idp", "client", "list"]), ClientCommand::List);
		assert_eq!(
			parse(&["idp", "client", "remove", "abc"]),
			ClientCommand::Remove {
				client_id: "abc".into()
			}
		);
		assert!(
			Harness::try_parse_from([
				"auth",
				"idp",
				"client",
				"add",
				"X",
				"--redirect",
				"https://a.example/cb",
				"--scopes",
				"openid",
				"--public",
				"--secret-post",
			])
			.is_err()
		);
	}

	#[test]
	fn answers_read_plainly() {
		let (text, failed) = render(&ClientCommandOutput::Added {
			client: client("Partner"),
			secret: Some("s3cret".into()),
		});
		assert!(!failed);
		assert!(text.starts_with("Registered Partner."), "{text}");
		assert!(text.contains("client id: abc"), "{text}");
		assert!(text.contains("client secret: s3cret"), "{text}");
		assert!(text.contains("shown this once"), "{text}");
		assert!(text.contains("first party"), "{text}");

		assert!(text.contains("grants: authorization_code"), "{text}");
		assert!(text.contains("ID tokens: RS256"), "{text}");
		let (text, _) = render(&ClientCommandOutput::Added {
			client: client("Phone"),
			secret: None,
		});
		assert!(text.contains("uses PKCE"), "{text}");
		let mut keyed = client("Worker");
		keyed.auth_method = ClientAuth::PrivateKeyJwt;
		keyed.redirect_uris.clear();
		let (text, _) = render(&ClientCommandOutput::Added {
			client: keyed,
			secret: None,
		});

		assert!(text.contains("private_key_jwt"), "{text}");
		assert!(!text.contains("\n  redirects:"), "{text}");
		let (text, failed) = render(&ClientCommandOutput::Rotated {
			secret: "n3w".into(),
		});
		assert!(!failed);
		assert!(text.contains("client secret: n3w"), "{text}");

		let mut bound = client("Two");
		bound.dpop_bound = true;
		bound.self_registered = true;
		let (text, _) = render(&ClientCommandOutput::Listed {
			clients: vec![client("One"), bound],
		});
		assert!(text.contains("One\n") && text.contains("\nTwo\n"), "{text}");
		assert!(text.contains("post-logout redirects"), "{text}");
		assert!(text.contains("bound to its key with DPoP"), "{text}");
		assert!(text.contains("registered itself"), "{text}");

		let (text, _) = render(&ClientCommandOutput::Listed {
			clients: Vec::new(),
		});
		assert_eq!(text, "No apps are registered.\n");

		assert!(!render(&ClientCommandOutput::Removed { removed: true }).1);
		assert!(render(&ClientCommandOutput::Removed { removed: false }).1);
		let (text, failed) = render(&ClientCommandOutput::Failed {
			message: "nope".into(),
		});
		assert!(failed);
		assert_eq!(text, "nope");
	}

	#[cfg(unix)]
	#[test]
	fn the_server_answers_in_the_file_it_is_given() {
		use std::os::unix::fs::PermissionsExt;

		let directory = tempfile::tempdir().unwrap();
		let script = directory.path().join("server");
		std::fs::write(
			&script,
			format!(
				"#!/bin/sh\ncase \"${COMMAND_ENV}\" in\n  *list*) printf '%s' \
				 '{{\"result\":\"listed\",\"clients\":[]}}' > \"${COMMAND_OUT_ENV}\" ;;\n  \
				 *remove*) echo 'not configured' >&2; exit 3 ;;\n  *) sleep 5 ;;\nesac\n"
			),
		)
		.unwrap();
		std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
		let env = BTreeMap::from([("EXTRA".to_owned(), "1".to_owned())]);

		let listed = run_server(
			&script,
			directory.path(),
			&env,
			&ClientCommand::List,
			Duration::from_secs(10),
		)
		.unwrap();
		assert_eq!(
			listed,
			ClientCommandOutput::Listed {
				clients: Vec::new()
			}
		);

		let error = run_server(
			&script,
			directory.path(),
			&env,
			&ClientCommand::Remove {
				client_id: "x".into(),
			},
			Duration::from_secs(10),
		)
		.unwrap_err()
		.to_string();
		assert!(error.contains("without answering"), "{error}");
		assert!(error.contains("not configured"), "{error}");

		let error = run_server(
			&script,
			directory.path(),
			&env,
			&ClientCommand::Add(NewClientSpec {
				name: "Slow".into(),
				redirect_uris: Vec::new(),
				post_logout_redirect_uris: Vec::new(),
				scopes: Vec::new(),
				auth_method: ClientAuth::None,
				grant_types: Vec::new(),
				jwks: None,
				id_token_alg: SigningAlgorithm::Es256,
				first_party: false,
				introspection: false,
				dpop_bound: false,
			}),
			Duration::from_millis(200),
		)
		.unwrap_err()
		.to_string();
		assert!(error.contains("didn't answer"), "{error}");
	}
}
