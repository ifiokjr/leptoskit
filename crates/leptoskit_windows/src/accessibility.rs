//! UI Automation exposes shared slider values and hides decorative content.
//! ARIA properties ride along: declared roles become control types, names
//! and descriptions flow through, and live regions raise screen-reader
//! notifications.
// windows-core's COM implementation macro emits these pointer conversions
// and inline attributes; the generated code is outside this crate's control.
#![allow(
	clippy::ptr_as_ptr,
	clippy::borrow_as_ptr,
	clippy::ref_as_ptr,
	clippy::inline_always
)]

use std::rc::Rc;
use std::rc::Weak;

use windows::Win32::Foundation::E_INVALIDARG;
use windows::Win32::Foundation::E_NOINTERFACE;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Accessibility::*;
use windows::core::BOOL;
use windows::core::HRESULT;
use windows::core::IUnknown;
use windows::core::IUnknownImpl;
use windows::core::Interface;
use windows::core::implement;

use crate::backend::Widget;

/// The UIA control type an explicitly declared ARIA role maps onto, for
/// owned frames; `None` lets the annotated MSAA role speak instead.
fn control_type(role: leptoskit_native::Role) -> Option<i32> {
	use leptoskit_native::Role;

	Some(
		match role {
			Role::Button
			| Role::MenuItem
			| Role::MenuItemCheckbox
			| Role::MenuItemRadio
			| Role::Tab
			| Role::TreeItem => UIA_ButtonControlTypeId,
			Role::Checkbox => UIA_CheckBoxControlTypeId,
			Role::Radio => UIA_RadioButtonControlTypeId,
			Role::Link => UIA_HyperlinkControlTypeId,
			Role::Img => UIA_ImageControlTypeId,
			Role::Slider => UIA_SliderControlTypeId,
			Role::SpinButton => UIA_SpinnerControlTypeId,
			Role::ProgressBar | Role::Meter => UIA_ProgressBarControlTypeId,
			Role::ScrollBar => UIA_ScrollBarControlTypeId,
			Role::TextBox | Role::SearchBox => UIA_EditControlTypeId,
			Role::List | Role::ListBox => UIA_ListControlTypeId,
			Role::ListItem | Role::Row => UIA_ListItemControlTypeId,
			Role::Tree => UIA_TreeControlTypeId,
			Role::Grid | Role::TreeGrid => UIA_DataGridControlTypeId,
			Role::Table => UIA_TableControlTypeId,
			Role::Cell | Role::GridCell | Role::Option => UIA_DataItemControlTypeId,
			Role::ColumnHeader => UIA_HeaderItemControlTypeId,
			Role::RowHeader => UIA_HeaderControlTypeId,
			Role::Group => UIA_GroupControlTypeId,
			Role::Heading
			| Role::Alert
			| Role::Log
			| Role::Timer
			| Role::Marquee
			| Role::Time
			| Role::Paragraph => UIA_TextControlTypeId,
			Role::Toolbar => UIA_ToolBarControlTypeId,
			Role::Menu => UIA_MenuControlTypeId,
			Role::MenuBar => UIA_MenuBarControlTypeId,
			Role::Status => UIA_StatusBarControlTypeId,
			Role::Separator => UIA_SeparatorControlTypeId,
			Role::Document => UIA_DocumentControlTypeId,
			_ => return None,
		}
		.0,
	)
}

/// The UIA heading level for an `aria-level`, one to nine.
fn heading_level(level: u32) -> i32 {
	match level {
		1 => HeadingLevel1,
		2 => HeadingLevel2,
		3 => HeadingLevel3,
		4 => HeadingLevel4,
		5 => HeadingLevel5,
		6 => HeadingLevel6,
		7 => HeadingLevel7,
		8 => HeadingLevel8,
		_ => HeadingLevel9,
	}
	.0
}

#[implement(IRawElementProviderSimple, Agile = false)]
pub(crate) struct DecorativeProvider {
	pub widget: Weak<Widget>,
	pub hwnd: HWND,
}

impl IRawElementProviderSimple_Impl for DecorativeProvider_Impl {
	fn ProviderOptions(&self) -> windows::core::Result<ProviderOptions> {
		Ok(ProviderOptions_ServerSideProvider | ProviderOptions_UseComThreading)
	}

	fn GetPatternProvider(&self, _pattern: UIA_PATTERN_ID) -> windows::core::Result<IUnknown> {
		Err(E_NOINTERFACE.into())
	}

	fn GetPropertyValue(&self, property: UIA_PROPERTY_ID) -> windows::core::Result<VARIANT> {
		let widget = self
			.widget
			.upgrade()
			.filter(|widget| widget.alive.get())
			.ok_or_else(|| windows::core::Error::from(HRESULT(UIA_E_ELEMENTNOTAVAILABLE as i32)))?;

		if property == UIA_IsControlElementPropertyId || property == UIA_IsContentElementPropertyId
		{
			// A named or roled frame is a real element for assistive
			// technology; the shared resolver folds labels and relationships
			// into the label before this runs.
			let props = widget.props.borrow();
			let element = !props.aria.is_hidden()
				&& (props.aria.role.is_some() || props.aria.label.is_some());
			return Ok(VARIANT::from(element));
		}

		if property == UIA_NamePropertyId {
			let props = widget.props.borrow();
			return Ok(VARIANT::from(props.accessible_name().unwrap_or_default()));
		}

		let props = widget.props.borrow();
		let aria = &props.aria;

		if property == UIA_ControlTypePropertyId
			&& let Some(role) = aria.role
			&& let Some(control) = control_type(role)
		{
			return Ok(VARIANT::from(control));
		}

		if property == UIA_LocalizedControlTypePropertyId
			&& let Some(description) = aria.role_description.as_deref()
		{
			return Ok(VARIANT::from(description));
		}

		if property == UIA_HelpTextPropertyId
			&& let Some(description) = aria.description.as_deref()
		{
			return Ok(VARIANT::from(description));
		}

		if property == UIA_ItemStatusPropertyId
			&& let Some(text) = aria.value_text.as_deref()
		{
			return Ok(VARIANT::from(text));
		}

		if property == UIA_HeadingLevelPropertyId
			&& let Some(level) = aria.level
		{
			return Ok(VARIANT::from(heading_level(level)));
		}

		if property == UIA_LiveSettingPropertyId
			&& aria.effective_live(widget.kind) != leptoskit_native::Live::Off
		{
			let setting = match aria.effective_live(widget.kind) {
				leptoskit_native::Live::Assertive => Assertive,
				_ => Polite,
			};
			return Ok(VARIANT::from(setting.0));
		}

		if property == UIA_PositionInSetPropertyId
			&& let Some(position) = aria.pos_in_set
		{
			return Ok(VARIANT::from(position as i32));
		}

		if property == UIA_SizeOfSetPropertyId
			&& let Some(size) = aria.set_size
		{
			return Ok(VARIANT::from(size as i32));
		}

		if property == UIA_IsRequiredForFormPropertyId
			&& let Some(required) = aria.required
		{
			return Ok(VARIANT::from(required));
		}

		Ok(VARIANT::default())
	}

	fn HostRawElementProvider(&self) -> windows::core::Result<IRawElementProviderSimple> {
		if !self
			.widget
			.upgrade()
			.is_some_and(|widget| widget.alive.get())
		{
			return Err(HRESULT(UIA_E_ELEMENTNOTAVAILABLE as i32).into());
		}

		// SAFETY: A live retained widget owns this HWND on its COM apartment.
		unsafe { UiaHostProviderFromHwnd(self.hwnd) }
	}
}

/// Raises a screen-reader notification when a live region's resolved text
/// changed. Call before committing the new props, so the widget still holds
/// the previously announced text.
pub(crate) fn announce_live(widget: &Rc<Widget>, props: &leptoskit_native::NativeProps) {
	let live = props.aria.effective_live(widget.kind);
	if live == leptoskit_native::Live::Off {
		return;
	}

	let Some(text) = props
		.aria
		.live_text
		.as_deref()
		.filter(|text| !text.is_empty())
	else {
		return;
	};

	if widget.props.borrow().aria.live_text.as_deref() == Some(text) {
		return;
	}

	let provider: IRawElementProviderSimple = DecorativeProvider {
		widget: Rc::downgrade(widget),
		hwnd: widget.frame,
	}
	.into();
	let processing = if live == leptoskit_native::Live::Assertive {
		NotificationProcessing_ImportantAll
	} else {
		NotificationProcessing_All
	};
	let text = windows_core::BSTR::from(text);
	let activity = windows_core::BSTR::from("leptoskit aria-live");
	// SAFETY: the provider is a live COM object retained for this call, and
	// the strings outlive it.
	let _ = unsafe {
		UiaRaiseNotificationEvent(
			&provider,
			NotificationKind_Other,
			processing,
			&text,
			&activity,
		)
	};
}

#[implement(IRawElementProviderSimple, IRangeValueProvider, Agile = false)]
pub(crate) struct SliderProvider {
	pub widget: Weak<Widget>,
	pub hwnd: HWND,
}

impl SliderProvider_Impl {
	fn widget(&self) -> windows::core::Result<Rc<Widget>> {
		self.widget
			.upgrade()
			.filter(|widget| widget.alive.get())
			.ok_or_else(|| HRESULT(UIA_E_ELEMENTNOTAVAILABLE as i32).into())
	}

	fn state(&self) -> windows::core::Result<leptoskit_native::SliderState> {
		Ok(self.widget()?.props.borrow().slider.unwrap_or_default())
	}
}

impl IRawElementProviderSimple_Impl for SliderProvider_Impl {
	fn ProviderOptions(&self) -> windows::core::Result<ProviderOptions> {
		Ok(ProviderOptions_ServerSideProvider | ProviderOptions_UseComThreading)
	}

	fn GetPatternProvider(&self, pattern: UIA_PATTERN_ID) -> windows::core::Result<IUnknown> {
		if pattern == UIA_RangeValuePatternId {
			return self.to_interface::<IRangeValueProvider>().cast();
		}
		Err(E_NOINTERFACE.into())
	}

	fn GetPropertyValue(&self, property: UIA_PROPERTY_ID) -> windows::core::Result<VARIANT> {
		let widget = self.widget()?;

		if property == UIA_ControlTypePropertyId {
			return Ok(VARIANT::from(UIA_SliderControlTypeId.0));
		}
		if property == UIA_IsRangeValuePatternAvailablePropertyId {
			return Ok(VARIANT::from(true));
		}
		if property == UIA_IsControlElementPropertyId || property == UIA_IsContentElementPropertyId
		{
			return Ok(VARIANT::from(true));
		}
		if property == UIA_IsEnabledPropertyId {
			return Ok(VARIANT::from(widget.interactive()));
		}
		if property == UIA_NamePropertyId {
			return Ok(VARIANT::from(
				widget.props.borrow().aria.label.as_deref().unwrap_or(""),
			));
		}

		Ok(VARIANT::default())
	}

	fn HostRawElementProvider(&self) -> windows::core::Result<IRawElementProviderSimple> {
		self.widget()?;
		// SAFETY: The retained widget verifies this HWND is still live.
		unsafe { UiaHostProviderFromHwnd(self.hwnd) }
	}
}

impl IRangeValueProvider_Impl for SliderProvider_Impl {
	fn SetValue(&self, value: f64) -> windows::core::Result<()> {
		let widget = self.widget()?;

		if !widget.interactive() {
			return Err(HRESULT(UIA_E_ELEMENTNOTENABLED as i32).into());
		}

		let state = self.state()?;

		if !value.is_finite() || value < state.min() || value > state.max() {
			return Err(E_INVALIDARG.into());
		}

		let value = state
			.proposed_value(value)
			.expect("finite proposal has a canonical slider value");
		widget.sink.emit(
			&widget,
			leptoskit_native::NativeEvent::ValueChange { value },
		);
		widget.restore_control();
		Ok(())
	}

	fn Value(&self) -> windows::core::Result<f64> {
		Ok(self.state()?.value())
	}

	fn IsReadOnly(&self) -> windows::core::Result<BOOL> {
		Ok((!self.widget()?.interactive()).into())
	}

	fn Maximum(&self) -> windows::core::Result<f64> {
		Ok(self.state()?.max())
	}

	fn Minimum(&self) -> windows::core::Result<f64> {
		Ok(self.state()?.min())
	}

	fn LargeChange(&self) -> windows::core::Result<f64> {
		let state = self.state()?;
		let span = state.max() - state.min();
		Ok(state
			.step()
			.map_or(span / 10.0, |step| step.min(span / 10.0) * 10.0))
	}

	fn SmallChange(&self) -> windows::core::Result<f64> {
		let state = self.state()?;
		Ok(state.step().unwrap_or((state.max() - state.min()) / 100.0))
	}
}
