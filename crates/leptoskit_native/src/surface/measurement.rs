//! Cached text and native-control measurement for the existing layout tree.

use std::sync::Arc;
use std::sync::PoisonError;
use std::sync::RwLock;

use leptoskit_yoga::AvailableSpace;
use leptoskit_yoga::NodeId;

use super::MeasuredText;
use super::NativeSurface;
use super::TEXT_FIELD_CHROME_HEIGHT;
use super::has_native_chrome;
use super::measured_content;
use crate::backend::NativeBackend;
use crate::element::Element;
use crate::props::NativeKind;

impl<B: NativeBackend> NativeSurface<B> {
	/// Replaces cached text metrics with the backend's current measurer and
	/// lays out again without replacing native widgets or event handlers.
	///
	/// Call after a host changes its font environment, such as display DPI or
	/// system text scale. Each returned measurer must still have immutable
	/// metrics: changing its results behind the layout cache is unsupported.
	pub fn refresh_text_measurer(&mut self) {
		let mut pending: Vec<_> = self.root.into_iter().collect();

		while let Some(id) = pending.pop() {
			pending.extend_from_slice(&self.shadows[id.index()].children);
			let Some(cell) = self.shadows[id.index()].text_cell.clone() else {
				continue;
			};
			let measured = cell.read().unwrap_or_else(PoisonError::into_inner).clone();
			self.attach_text_measure(id, measured);
		}

		self.commit_layout();
	}

	pub(super) fn attach_text_measure(&mut self, id: NodeId, text: MeasuredText) {
		let measurer = self.backend.text_measurer();
		let kind = self.shadows[id.index()].kind;
		let single_line = matches!(
			self.shadows[id.index()].kind,
			NativeKind::TextInput | NativeKind::PasswordInput
		);
		let cell = Arc::new(RwLock::new(text));
		let closure_cell = Arc::clone(&cell);
		self.yoga.set_measure_func(
			id,
			Some(Arc::new(move |mut available| {
				// Writers only ever replace whole values, so a poisoned lock
				// still holds a valid text and style.
				let current = closure_cell.read().unwrap_or_else(PoisonError::into_inner);

				if kind == NativeKind::Button {
					return measurer.measure_button(
						&current.content,
						current.style,
						available,
						current.native_chrome,
					);
				}

				if single_line {
					available.width = AvailableSpace::MaxContent;
				}

				let mut size = measurer.measure(&current.content, current.style, available);
				if current.native_chrome {
					size.height += TEXT_FIELD_CHROME_HEIGHT;
				}

				size
			})),
		);
		self.shadows[id.index()].text_cell = Some(cell);
	}

	/// Refreshes cached text only when the existing measurement inputs change.
	pub(super) fn update_text_measure(&mut self, id: NodeId, element: &Element) {
		let style = element.props.text_style();
		let native_chrome = has_native_chrome(element);
		match (
			measured_content(element),
			self.shadows[id.index()].text_cell.clone(),
		) {
			(Some(text), Some(cell)) => {
				self.refresh_cached_measurement(id, text, style, native_chrome, &cell);
			}
			(Some(text), None) => {
				self.attach_text_measure(
					id,
					MeasuredText {
						content: text.to_owned(),
						style,
						native_chrome,
					},
				);
			}
			(None, Some(_)) => {
				self.yoga.set_measure_func(id, None);
				self.shadows[id.index()].text_cell = None;
			}
			(None, None) => {}
		}
	}

	/// Invalidates layout after replacing only the text inputs that affect
	/// size.
	fn refresh_cached_measurement(
		&mut self,
		id: NodeId,
		text: &str,
		style: crate::props::TextStyle,
		native_chrome: bool,
		cell: &Arc<RwLock<MeasuredText>>,
	) {
		let mut measured = cell.write().unwrap_or_else(PoisonError::into_inner);
		// Colors and the like do not change the text's size, so
		// only a new string, font, or native chrome invalidates
		// layout.
		if measured.content != text
			|| measured.style != style
			|| measured.native_chrome != native_chrome
		{
			text.clone_into(&mut measured.content);
			measured.style = style;
			measured.native_chrome = native_chrome;
			drop(measured);

			// The measure function reads the cell lazily, so the
			// node and its cached measurement must be invalidated
			// by hand.
			self.yoga.mark_dirty(id);
		}
	}
}
