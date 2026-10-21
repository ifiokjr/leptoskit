use std::fs;
use std::io::Read;
use std::io::Write;
use std::path::PathBuf;

use super::Autostart;
use super::Error;

pub(super) struct Entry {
	path: PathBuf,
	content: String,
}
impl Entry {
	pub(super) fn new(path: PathBuf, content: String) -> Self {
		Self { path, content }
	}

	pub(super) fn is_registered(&self) -> Result<bool, Error> {
		match fs::symlink_metadata(&self.path) {
			Ok(metadata) if metadata.file_type().is_file() => {
				let mut content = Vec::new();
				fs::File::open(&self.path)?
					.take(self.content.len() as u64 + 1)
					.read_to_end(&mut content)?;
				if content == self.content.as_bytes() {
					Ok(true)
				} else {
					Err(Error::Conflict)
				}
			}
			Ok(_) => Err(Error::Conflict),
			Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
			Err(error) => Err(error.into()),
		}
	}

	pub(super) fn enable(&self) -> Result<(), Error> {
		if self.is_registered()? {
			return Ok(());
		}
		let parent = self.path.parent().ok_or(Error::InvalidConfiguration)?;
		fs::create_dir_all(parent)?;
		if !fs::symlink_metadata(parent)?.file_type().is_dir() {
			return Err(Error::Conflict);
		}
		let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
		temporary.write_all(self.content.as_bytes())?;
		temporary.as_file().sync_all()?;
		match temporary.persist_noclobber(&self.path) {
			Ok(_) => Ok(()),
			Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
				if self.is_registered()? {
					Ok(())
				} else {
					Err(Error::Conflict)
				}
			}
			Err(error) => Err(error.error.into()),
		}
	}

	pub(super) fn disable(&self) -> Result<(), Error> {
		if self.is_registered()? {
			match fs::remove_file(&self.path) {
				Ok(()) => (),
				Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
				Err(error) => return Err(error.into()),
			}
		}
		Ok(())
	}
}

#[cfg(any(target_os = "macos", test))]
pub(super) fn launch_agent(app: &Autostart) -> String {
	fn escaped(value: &str) -> String {
		value
			.replace('&', "&amp;")
			.replace('<', "&lt;")
			.replace('>', "&gt;")
			.replace('"', "&quot;")
			.replace('\'', "&apos;")
	}
	let mut arguments = String::new();
	for argument in std::iter::once(&app.executable).chain(&app.arguments) {
		arguments.push_str("<string>");
		arguments.push_str(&escaped(argument));
		arguments.push_str("</string>");
	}
	format!(
		"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict><key>Label</key><string>{}</string><key>ProgramArguments</key><array>{arguments}</array><key>RunAtLoad</key><true/></dict></plist>\n",
		app.identity
	)
}

#[cfg(any(target_os = "linux", test))]
pub(super) fn desktop_entry(app: &Autostart) -> String {
	fn quoted(value: &str) -> String {
		let mut argument = String::from("\"");
		for ch in value.chars() {
			if matches!(ch, '"' | '`' | '$' | '\\') {
				argument.push('\\');
			}
			if ch == '%' {
				argument.push('%');
			}
			argument.push(ch);
		}
		argument.push('"');
		argument.replace('\\', "\\\\")
	}
	let command = std::iter::once(&app.executable)
		.chain(&app.arguments)
		.map(|arg| quoted(arg))
		.collect::<Vec<_>>()
		.join(" ");
	format!(
		"[Desktop Entry]\nType=Application\nVersion=1.0\nName={}\nExec={command}\nTerminal=false\n",
		app.identity
	)
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn registration_is_idempotent_and_never_overwrites_another_entry() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("app.plist");
		let entry = Entry::new(path.clone(), "owned".into());
		assert!(!entry.is_registered().unwrap());
		entry.enable().unwrap();
		entry.enable().unwrap();
		assert!(entry.is_registered().unwrap());
		let other = Entry::new(path.clone(), "other".into());
		assert!(matches!(other.enable(), Err(Error::Conflict)));
		assert!(matches!(other.disable(), Err(Error::Conflict)));
		assert_eq!(fs::read_to_string(&path).unwrap(), "owned");
		entry.disable().unwrap();
		entry.disable().unwrap();
		assert!(!path.exists());
	}
	// Login-entry files are a Linux and macOS mechanism; this module is
	// compiled on Windows only for its tests. There, a path under a regular
	// file reports NotFound, which is correctly "not registered".
	#[cfg(unix)]
	#[test]
	fn unreadable_entry_paths_report_their_io_error() {
		let dir = tempfile::tempdir().unwrap();
		let file = dir.path().join("file");
		fs::write(&file, "not a directory").unwrap();
		let entry = Entry::new(file.join("app.plist"), "owned".into());
		assert!(matches!(entry.is_registered(), Err(Error::Io(_))));
		assert!(matches!(entry.enable(), Err(Error::Io(_))));
	}
	// A symlinked login-items directory could redirect registration
	// elsewhere; Windows has no login-entry files.
	#[cfg(unix)]
	#[test]
	fn symlinked_entry_directory_is_a_conflict() {
		let dir = tempfile::tempdir().unwrap();
		let target = dir.path().join("target");
		fs::create_dir(&target).unwrap();
		let linked = dir.path().join("linked");
		std::os::unix::fs::symlink(&target, &linked).unwrap();
		let entry = Entry::new(linked.join("app.plist"), "owned".into());
		assert!(matches!(entry.enable(), Err(Error::Conflict)));
		assert!(!target.join("app.plist").exists());
	}
	#[test]
	fn plist_round_trips_metacharacters_and_empty_arguments() {
		let app = Autostart {
			identity: "test".into(),
			executable: "/Applications/A&B<app>/run".into(),
			arguments: vec![String::new(), "quotes\"'$` & <>".into()],
		};
		let plist = plist::Value::from_reader_xml(launch_agent(&app).as_bytes()).unwrap();
		let args = plist.as_dictionary().unwrap()["ProgramArguments"]
			.as_array()
			.unwrap();
		assert_eq!(
			args.iter()
				.map(|v| v.as_string().unwrap())
				.collect::<Vec<_>>(),
			[&app.executable, &app.arguments[0], &app.arguments[1]]
		);
	}
	#[test]
	fn desktop_command_escapes_both_string_and_argument_layers() {
		let app = Autostart {
			identity: "test".into(),
			executable: "/opt/My App/run".into(),
			arguments: vec![String::new(), "a$`\"\\%F".into()],
		};
		assert!(
			desktop_entry(&app)
				.contains("Exec=\"/opt/My App/run\" \"\" \"a\\\\$\\\\`\\\\\"\\\\\\\\%%F\"\n")
		);
	}
	// Login-entry files exist only on Linux and macOS; Windows registers a
	// `Run` registry value instead, covered by `tests/registration.rs`.
	#[cfg(unix)]
	#[test]
	fn symbolic_link_entry_is_rejected_without_touching_target() {
		let dir = tempfile::tempdir().unwrap();
		let target = dir.path().join("target");
		fs::write(&target, "owned").unwrap();
		let link = dir.path().join("link");
		std::os::unix::fs::symlink(&target, &link).unwrap();
		let entry = Entry::new(link, "owned".into());
		assert!(matches!(entry.enable(), Err(Error::Conflict)));
		assert!(matches!(entry.disable(), Err(Error::Conflict)));
		assert_eq!(fs::read_to_string(target).unwrap(), "owned");
	}
}
