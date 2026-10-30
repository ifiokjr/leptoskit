//! Browser posture from the Device Posture and Viewport Segments APIs.
//!
//! `navigator.devicePosture.type` is `"folded"` while a foldable is partly
//! open and `"continuous"` otherwise, so a browser cannot tell a closed device
//! from an open one without segments. `window.viewport.segments` lists the
//! viewport's regions on either side of a fold; the gaps between them become
//! hinges. Browsers without either API report an unknown posture. Neither API
//! reports hinge angles.

use std::cell::RefCell;
use std::convert::Infallible;
use std::rc::Rc;

use futures::channel::oneshot;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;

use super::*;

fn property(value: &JsValue, key: &str) -> Result<JsValue, Error> {
	js_sys::Reflect::get(value, &JsValue::from_str(key)).map_err(|_| Error::Native)
}

fn present(value: &JsValue) -> bool {
	!value.is_undefined() && !value.is_null()
}

/// Reads the viewport's segments in CSS pixels, if the browser reports any.
fn segments(window: &web_sys::Window) -> Result<Vec<web_sys::DomRect>, Error> {
	let viewport = property(window.as_ref(), "viewport")?;
	if !present(&viewport) {
		return Ok(Vec::new());
	}
	let segments = property(&viewport, "segments")?;
	if !present(&segments) {
		return Ok(Vec::new());
	}
	js_sys::Array::from(&segments)
		.iter()
		.map(|segment| {
			segment
				.dyn_into::<web_sys::DomRect>()
				.map_err(|_| Error::Native)
		})
		.collect()
}

/// The gap between two neighbouring segments, as a hinge.
fn hinge_between(
	first: &web_sys::DomRect,
	second: &web_sys::DomRect,
	fold: Fold,
	width: f64,
	height: f64,
) -> Hinge {
	let (width, height) = (width as f32, height as f32);
	let (axis, gap, region) = if second.left() >= first.right() {
		let gap = (second.left() - first.right()) as f32;
		let region = Region::within(first.right() as f32, 0.0, gap, height, width, height);
		(Axis::Vertical, gap, region)
	} else {
		let gap = (second.top() - first.bottom()) as f32;
		let region = Region::within(0.0, first.bottom() as f32, width, gap, width, height);
		(Axis::Horizontal, gap, region)
	};
	Hinge {
		fold,
		axis,
		region,
		angle: None,
		// Content in the gap between segments is not visible.
		occluding: Some(gap > 0.0),
	}
}

/// Reads the current posture.
pub fn snapshot() -> Result<Posture, Error> {
	let window = web_sys::window().ok_or(Error::Unavailable)?;
	let posture = property(window.navigator().as_ref(), "devicePosture")?;
	let folded = if present(&posture) {
		Some(property(&posture, "type")?.as_string().as_deref() == Some("folded"))
	} else {
		None
	};
	let segments = segments(&window)?;
	let width = window
		.inner_width()
		.map_err(|_| Error::Native)?
		.as_f64()
		.ok_or(Error::Native)?;
	let height = window
		.inner_height()
		.map_err(|_| Error::Native)?
		.as_f64()
		.ok_or(Error::Native)?;
	let hinge_fold = if folded == Some(true) {
		Fold::HalfOpen
	} else {
		Fold::Open
	};
	let hinges: Vec<Hinge> = segments
		.windows(2)
		.take(MAX_HINGES)
		.map(|pair| hinge_between(&pair[0], &pair[1], hinge_fold, width, height))
		.collect();
	let fold = match folded {
		Some(true) => Fold::HalfOpen,
		_ if !hinges.is_empty() => Fold::Open,
		_ => Fold::Unknown,
	};
	Ok(Posture {
		foldable: (folded == Some(true) || !hinges.is_empty()).then_some(true),
		fold,
		hinges,
	})
}

/// Waits for a posture that differs from `previous`, listening for posture
/// changes and viewport resizes. Dropping this future removes the listeners.
pub async fn wait_for_change(previous: &Posture) -> Result<Posture, Error> {
	let window = web_sys::window().ok_or(Error::Unavailable)?;
	let (sender, receiver) = oneshot::channel();
	let sender = Rc::new(RefCell::new(Some(sender)));
	let changed = {
		let sender = Rc::clone(&sender);
		let previous = previous.clone();
		move || {
			let current = snapshot();
			if current.as_ref() != Ok(&previous)
				&& let Some(sender) = sender.borrow_mut().take()
			{
				// A dispatched callback can outlive a cancelled receiver.
				let _ = sender.send(current);
			}
		}
	};
	let mut listeners = Vec::with_capacity(2);
	{
		let changed = changed.clone();
		listeners.push(gloo_events::EventListener::new(
			&window,
			"resize",
			move |_| changed(),
		));
	}
	let posture = property(window.navigator().as_ref(), "devicePosture")?;
	if present(&posture) {
		let target = posture
			.dyn_into::<web_sys::EventTarget>()
			.map_err(|_| Error::Native)?;
		let changed = changed.clone();
		listeners.push(gloo_events::EventListener::new(
			&target,
			"change",
			move |_| changed(),
		));
	}
	// Listen before sampling so a change between the two is not missed.
	changed();
	let result = receiver.await.map_err(|_| Error::Cancelled)?;
	drop(listeners);
	result
}

/// Calls `on_change` with the current posture and again after every change.
/// Drop the future to stop watching.
pub async fn watch(mut on_change: impl FnMut(&Posture)) -> Result<Infallible, Error> {
	let mut current = snapshot()?;
	on_change(&current);
	loop {
		current = wait_for_change(&current).await?;
		on_change(&current);
	}
}
