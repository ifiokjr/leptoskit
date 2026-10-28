//! Terminal startup and event delivery. Application screens live in
//! __NAME___app.

#[cfg(not(target_arch = "wasm32"))]
mod runtime;

#[cfg(not(target_arch = "wasm32"))]
mod generated {
	include!(concat!(
		env!("CARGO_MANIFEST_DIR"),
		"/../.leptoskit/tui/registrant.rs"
	));
}

#[cfg(not(target_arch = "wasm32"))]
mod application {
	use leptoskit::native::EventOutcome;
	use leptoskit::native::NativeEvent;
	use leptoskit::tui::Exit;
	use leptoskit::tui::TuiApplication;
	use leptoskit::tui::TuiBackend;
	use leptoskit::tui::TuiHost;
	use leptoskit::tui::TuiOptions;
	use leptoskit::tui::host::OutputSink;
	use leptoskit::tui::terminal::CrosstermSource;
	use leptoskit::tui::terminal::Terminal;
	use leptoskit::wasm::WasmError;
	use leptoskit::wasm::WasmHost;
	use leptoskit::yoga::NodeId;

	use super::runtime;

	/// The wasm guest over the terminal backend.
	struct Guest {
		host: WasmHost<TuiBackend>,
		failure: Option<WasmError>,
	}

	impl TuiApplication for Guest {
		fn backend(&mut self) -> &mut TuiBackend {
			self.host.surface_mut().backend_mut()
		}

		fn dispatch(&mut self, node: NodeId, event: &NativeEvent) -> EventOutcome {
			if self.failure.is_some() {
				return EventOutcome::Unhandled;
			}

			self.host.dispatch(node, event).unwrap_or_else(|error| {
				self.failure = Some(error);
				EventOutcome::Unhandled
			})
		}

		fn resize(&mut self, cols: u16, rows: u16) {
			let (width, height) = leptoskit::tui::viewport(cols, rows);
			self.host.set_viewport(width, height);
		}
	}

	pub(super) fn run() -> runtime::AppResult<()> {
		let options = TuiOptions {
			title: "__NAME__".into(),
			..TuiOptions::default()
		};
		let mut terminal = Terminal::new(&options)?;
		let (cols, rows) = terminal.size()?;

		let wasm = runtime::guest("tui")?;
		let mut host = WasmHost::new(TuiBackend::new(), &wasm)?;
		super::generated::configure(&mut host);
		host.start(cols as f32 * 8.0, rows as f32 * 16.0)?;

		let mut app = Guest {
			host,
			failure: None,
		};
		let mut source = CrosstermSource;

		let exit = TuiHost::new(options).run(&mut app, &mut source, &mut terminal)?;
		if exit == Exit::ExitKey {
			eprintln!("(exited with ctrl+c)");
		}

		if let Some(error) = app.failure {
			return Err(error.into());
		}

		Ok(())
	}
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> runtime::AppResult<()> {
	application::run()
}

#[cfg(target_arch = "wasm32")]
fn main() {
	eprintln!("build the terminal runner on a desktop with `leptoskit build --platform tui`");
	std::process::exit(1);
}
