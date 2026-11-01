use std::sync::Arc;

use leptoskit_plugin_menu::DesktopMenu;
use tray_icon::TrayIcon;
use tray_icon::TrayIconBuilder;

use super::Events;
use crate::Error;
use crate::Options;
use crate::Point;
use crate::Rect;
use crate::RgbaIcon;
use crate::TrayId;

pub(super) struct NativeTray(TrayIcon);
impl NativeTray {
	pub(super) fn new(
		_: TrayId,
		native_id: &str,
		icon: RgbaIcon,
		options: &Options,
		menu: Option<&DesktopMenu>,
		_: Arc<Events>,
	) -> Result<Self, Error> {
		let icon = convert_icon(icon)?;
		let mut builder = TrayIconBuilder::new().with_id(native_id);
		#[cfg(target_os = "macos")]
		{
			builder = if options.template {
				builder.with_icon_templated(icon)
			} else {
				builder.with_icon(icon)
			};
		}
		#[cfg(not(target_os = "macos"))]
		{
			builder = builder.with_icon(icon);
		}

		if let Some(text) = &options.tooltip {
			builder = builder.with_tooltip(text);
		}

		if let Some(title) = &options.title {
			builder = builder.with_title(title);
		}

		if let Some(menu) = menu {
			builder = builder.with_menu(Box::new(menu.native_menu().clone()));
		}

		builder.build().map(Self).map_err(native_error)
	}

	pub(super) fn set_icon(&self, icon: RgbaIcon, template: bool) -> Result<(), Error> {
		let icon = convert_icon(icon)?;

		#[cfg(target_os = "macos")]
		if template {
			return self.0.set_icon_templated(Some(icon)).map_err(native_error);
		}

		#[cfg(not(target_os = "macos"))]
		let _ = template;
		self.0.set_icon(Some(icon)).map_err(native_error)
	}

	pub(super) fn set_tooltip(&self, text: Option<&str>) -> Result<(), Error> {
		self.0.set_tooltip(text).map_err(native_error)
	}

	#[allow(
		clippy::unnecessary_wraps,
		reason = "Matches the fallible Linux service backend."
	)]
	pub(super) fn set_title(&self, title: Option<&str>) -> Result<(), Error> {
		self.0.set_title(title);
		Ok(())
	}

	pub(super) fn set_visible(&self, visible: bool) -> Result<(), Error> {
		self.0.set_visible(visible).map_err(native_error)
	}

	#[allow(
		clippy::unnecessary_wraps,
		reason = "Matches the fallible Linux service backend."
	)]
	pub(super) fn set_menu(&mut self, menu: Option<&DesktopMenu>) -> Result<(), Error> {
		self.0.set_menu(menu.map(|menu| {
			Box::new(menu.native_menu().clone()) as Box<dyn tray_icon::menu::ContextMenu>
		}));
		Ok(())
	}

	#[allow(
		clippy::unnecessary_wraps,
		reason = "Matches the fallible Linux service backend."
	)]
	pub(super) fn bounds(&self) -> Result<Option<Rect>, Error> {
		Ok(self.0.rect().and_then(|rect| {
			(rect.position.x.is_finite() && rect.position.y.is_finite()).then_some(Rect {
				position: Point {
					x: rect.position.x,
					y: rect.position.y,
				},
				width: rect.size.width,
				height: rect.size.height,
			})
		}))
	}

	#[allow(
		clippy::unused_self,
		clippy::unnecessary_wraps,
		reason = "Native menus update directly; Linux requires explicit service refresh."
	)]
	pub(super) fn refresh_menu(&self) -> Result<(), Error> {
		Ok(())
	}
}
fn convert_icon(icon: RgbaIcon) -> Result<tray_icon::Icon, Error> {
	tray_icon::Icon::from_rgba(icon.pixels, icon.width, icon.height).map_err(|_| Error::InvalidIcon)
}
#[allow(
	clippy::needless_pass_by_value,
	reason = "Result::map_err transfers ownership of the native error."
)]
fn native_error(error: tray_icon::Error) -> Error {
	Error::Native(error.to_string())
}
