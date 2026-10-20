//! UI Automation exposes shared slider values and hides decorative content.
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
		if !self
			.widget
			.upgrade()
			.is_some_and(|widget| widget.alive.get())
		{
			return Err(HRESULT(UIA_E_ELEMENTNOTAVAILABLE as i32).into());
		}

		if property == UIA_IsControlElementPropertyId || property == UIA_IsContentElementPropertyId
		{
			return Ok(VARIANT::from(false));
		}

		if property == UIA_NamePropertyId {
			return Ok(VARIANT::from(""));
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
				widget
					.props
					.borrow()
					.accessibility_label
					.as_deref()
					.unwrap_or(""),
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
