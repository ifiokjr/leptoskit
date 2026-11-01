//! `lk auth idp client …`: the CLI builds the app's server and runs it with
//! a command in the environment, which the server carries out against its
//! own database and exits.

use std::path::Path;

use crate::COMMAND_ENV;
use crate::COMMAND_OUT_ENV;
use crate::ClientCommand;
use crate::ClientCommandOutput;
use crate::Idp;

impl Idp {
	/// Carries out the `lk auth idp client` command in
	/// `LEPTOSKIT_IDP_COMMAND`, if there is one, writes the answer to the file
	/// `LEPTOSKIT_IDP_OUT` names, and exits. Without one it returns, and the
	/// server starts as usual.
	///
	/// Call it once the provider is built, before the server binds its port:
	///
	/// ```ignore
	/// let idp = Idp::builder(&db, &auth).build()?;
	/// idp.run_command_if_requested().await;
	/// ```
	pub async fn run_command_if_requested(&self) {
		let (Ok(command), Ok(out)) = (std::env::var(COMMAND_ENV), std::env::var(COMMAND_OUT_ENV))
		else {
			return;
		};

		match self.answer(&command, Path::new(&out)).await {
			Ok(failed) => std::process::exit(i32::from(failed)),
			Err(error) => {
				eprintln!("error: couldn't write the answer to {out}: {error}");
				std::process::exit(1);
			}
		}
	}

	/// Carries out `command`, given as JSON, and writes the answer to `out`.
	/// Returns whether the command failed.
	pub(crate) async fn answer(&self, command: &str, out: &Path) -> std::io::Result<bool> {
		let output = match serde_json::from_str::<ClientCommand>(command) {
			Ok(command) => self.run_command(&command).await,
			Err(error) => {
				ClientCommandOutput::Failed {
					message: format!("the command isn't one this server understands: {error}"),
				}
			}
		};

		write_output(out, &output)?;
		Ok(matches!(output, ClientCommandOutput::Failed { .. }))
	}

	/// Carries out one `lk auth idp client` command.
	pub async fn run_command(&self, command: &ClientCommand) -> ClientCommandOutput {
		let result = match command {
			ClientCommand::Add(spec) => {
				self.register_client(spec.clone())
					.await
					.map(|(client, secret)| {
						ClientCommandOutput::Added {
							client,
							secret: secret.map(|secret| secret.expose().to_owned()),
						}
					})
			}
			ClientCommand::List => {
				self.clients()
					.await
					.map(|clients| ClientCommandOutput::Listed { clients })
			}
			ClientCommand::Remove { client_id } => {
				self.remove_client(client_id)
					.await
					.map(|removed| ClientCommandOutput::Removed { removed })
			}
			ClientCommand::RotateSecret { client_id } => {
				match self.rotate_secret(client_id).await {
					Ok(Some(secret)) => {
						Ok(ClientCommandOutput::Rotated {
							secret: secret.expose().to_owned(),
						})
					}
					Ok(None) => {
						Ok(ClientCommandOutput::Failed {
							message: "no app with a secret has that client ID".into(),
						})
					}
					Err(error) => Err(error),
				}
			}
		};

		result.unwrap_or_else(|error| {
			ClientCommandOutput::Failed {
				message: match &error {
					crate::IdpError::OAuth { description, .. } => description.clone(),
					_ => error.to_string(),
				},
			}
		})
	}
}

/// Writes the answer readable only by its owner, since it may hold a client
/// secret.
fn write_output(path: &Path, output: &ClientCommandOutput) -> std::io::Result<()> {
	let json = serde_json::to_vec(output).map_err(std::io::Error::other)?;
	let mut options = std::fs::OpenOptions::new();
	options.write(true).create(true).truncate(true);

	#[cfg(unix)]
	{
		use std::os::unix::fs::OpenOptionsExt;
		options.mode(0o600);
	}

	std::io::Write::write_all(&mut options.open(path)?, &json)
}

#[cfg(all(test, feature = "sqlite"))]
mod tests {
	use leptoskit_auth::Auth;
	use leptoskit_auth::Keys;
	use leptoskit_auth::Mailbox;

	use super::*;

	#[tokio::test]
	async fn answers_go_to_a_private_file() {
		let db = leptoskit_db::testing::sqlite().await;
		let auth = Auth::builder(&db, Keys::single("test", [1; 32]))
			.origin("https://notes.example")
			.mailer(Mailbox::new())
			.build()
			.unwrap();
		let idp = Idp::builder(&db, &auth).build().unwrap();
		let directory = tempfile::tempdir().unwrap();
		let out = directory.path().join("answer.json");

		assert!(!idp.answer(r#"{"command":"list"}"#, &out).await.unwrap());
		let answer: ClientCommandOutput =
			serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
		assert_eq!(
			answer,
			ClientCommandOutput::Listed {
				clients: Vec::new()
			}
		);
		#[cfg(unix)]
		{
			use std::os::unix::fs::PermissionsExt;
			let mode = std::fs::metadata(&out).unwrap().permissions().mode();
			assert_eq!(mode & 0o777, 0o600);
		}

		assert!(idp.answer("not json", &out).await.unwrap());
		let answer: ClientCommandOutput =
			serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
		assert!(
			matches!(&answer, ClientCommandOutput::Failed { message } if message.contains("understands"))
		);

		let missing = directory.path().join("missing").join("answer.json");
		assert!(idp.answer(r#"{"command":"list"}"#, &missing).await.is_err());

		// Without the variables, the server just starts.
		idp.run_command_if_requested().await;
	}
}
