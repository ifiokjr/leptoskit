//! Browser console logging. No persistence or platform-log capture is implied.
use super::*;

/// Console logger with an explicit threshold and lifecycle.
#[derive(Debug)]
pub struct Logger {
	minimum: Level,
	closed: bool,
}

impl Logger {
	/// Creates a logger. It does not replace global console methods.
	pub fn new(minimum: Level) -> Self {
		Self {
			minimum,
			closed: false,
		}
	}

	/// Emits a structured object to the matching console severity function.
	/// Returns false when filtered. The browser owns console buffering.
	pub fn write(&self, record: &Record) -> Result<bool, Error> {
		if self.closed {
			return Err(Error::Cancelled);
		}
		record.validate()?;
		if record.level < self.minimum {
			return Ok(false);
		}
		let milliseconds = js_sys::Date::now();
		if !milliseconds.is_finite() || milliseconds < 0.0 || milliseconds >= u64::MAX as f64 {
			return Err(Error::Native);
		}
		let bytes = json(record, milliseconds as u64)?;
		let value = js_sys::JSON::parse(std::str::from_utf8(&bytes).map_err(|_| Error::Native)?)
			.map_err(|_| Error::Native)?;
		match record.level {
			Level::Trace | Level::Debug => web_sys::console::debug_1(&value),
			Level::Info => web_sys::console::info_1(&value),
			Level::Warn => web_sys::console::warn_1(&value),
			Level::Error => web_sys::console::error_1(&value),
		}
		Ok(true)
	}

	/// The console API offers no flush acknowledgement; calls emit immediately.
	pub fn flush(&self) -> Result<(), Error> {
		if self.closed {
			Err(Error::Cancelled)
		} else {
			Ok(())
		}
	}

	/// Closes this instance. There are no listeners or timers to release.
	pub fn close(&mut self) {
		self.closed = true;
	}
}
