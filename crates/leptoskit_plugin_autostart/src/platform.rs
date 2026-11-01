use std::io;

use winreg::RegKey;
use winreg::enums::HKEY_CURRENT_USER;
use winreg::enums::KEY_READ;
use winreg::enums::KEY_WRITE;

use super::Autostart;
use super::Error;
use super::windows_command;

const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

fn command(app: &Autostart) -> Result<String, Error> {
	let command = windows_command(app);
	// Windows documents a 260-character limit for Run key command lines.
	if command.encode_utf16().count() > 260 {
		return Err(Error::InvalidConfiguration);
	}

	Ok(command)
}

fn matches(key: &RegKey, app: &Autostart, command: &str) -> Result<bool, Error> {
	match key.get_value::<String, _>(&app.identity) {
		Ok(value) if value == command => Ok(true),
		Ok(_) => Err(Error::Conflict),

		Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
		Err(error) => Err(error.into()),
	}
}

pub(super) fn is_registered(app: &Autostart) -> Result<bool, Error> {
	let command = command(app)?;

	match RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(RUN, KEY_READ) {
		Ok(key) => matches(&key, app, &command),

		Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
		Err(error) => Err(error.into()),
	}
}

pub(super) fn enable(app: &Autostart) -> Result<(), Error> {
	let command = command(app)?;
	let (key, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(RUN)?;

	if !matches(&key, app, &command)? {
		key.set_value(&app.identity, &command)?;
	}

	Ok(())
}

pub(super) fn disable(app: &Autostart) -> Result<(), Error> {
	let command = command(app)?;
	let key =
		match RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(RUN, KEY_READ | KEY_WRITE) {
			Ok(key) => key,

			Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
			Err(error) => return Err(error.into()),
		};

	if matches(&key, app, &command)? {
		match key.delete_value(&app.identity) {
			Ok(()) => (),

			Err(error) if error.kind() == io::ErrorKind::NotFound => (),
			Err(error) => return Err(error.into()),
		}
	}

	Ok(())
}
