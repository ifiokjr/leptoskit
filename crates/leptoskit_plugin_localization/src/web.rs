use js_sys::Array;
use js_sys::Function;
use js_sys::Object;
use js_sys::Reflect;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;

use super::Error;
use super::Locale;
use super::Snapshot;

fn property(object: &JsValue, key: &str) -> Result<JsValue, Error> {
	Reflect::get(object, &JsValue::from_str(key)).map_err(|_| Error::Native)
}
fn text(object: &JsValue, key: &str) -> Result<Option<String>, Error> {
	Ok(property(object, key)?
		.as_string()
		.filter(|value| !value.is_empty()))
}
fn construct(intl: &JsValue, kind: &str, arguments: &Array) -> Result<JsValue, Error> {
	let constructor = property(intl, kind)?
		.dyn_into::<Function>()
		.map_err(|_| Error::Unavailable)?;
	Reflect::construct(&constructor, arguments).map_err(|_| Error::Native)
}
fn invoke(object: &JsValue, method: &str) -> Result<JsValue, Error> {
	property(object, method)?
		.dyn_into::<Function>()
		.map_err(|_| Error::Unavailable)?
		.call0(object)
		.map_err(|_| Error::Native)
}
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn weekday(info: &JsValue) -> Result<Option<u8>, Error> {
	let value = property(info, "firstDay")?;

	match value.as_f64() {
		Some(day) if (1.0..=7.0).contains(&day) && day.fract() == 0.0 => Ok(Some(day as u8)),

		None if value.is_undefined() => Ok(None),
		_ => Err(Error::Native),
	}
}

/// Reads ordered Navigator languages and Intl calendar preferences. Language
/// and region come from Intl.Locale, never inferred from the browser's user
/// agent. Clock preference reflects Intl's locale default, which may differ
/// from a separately configured OS clock setting. Missing week-info support
/// stays unknown.
pub fn snapshot() -> Result<Snapshot, Error> {
	let navigator = web_sys::window().ok_or(Error::Unavailable)?.navigator();
	let languages = navigator.languages();

	if languages.length() > 64 {
		return Err(Error::TooLarge);
	}

	let intl = property(&js_sys::global(), "Intl")?;
	let mut locales = Vec::with_capacity(languages.length() as usize);
	let mut first_weekday = None;

	for (index, value) in languages.iter().enumerate() {
		let tag = value.as_string().ok_or(Error::Native)?;
		let locale = construct(&intl, "Locale", &Array::of1(&value))?;

		if index == 0 {
			let getter = property(&locale, "getWeekInfo")?;
			let info = if getter.is_function() {
				invoke(&locale, "getWeekInfo")?
			} else {
				property(&locale, "weekInfo")?
			};

			if info.is_object() {
				first_weekday = weekday(&info)?;
			}
		}

		locales.push(Locale {
			tag,
			language: text(&locale, "language")?,
			region: text(&locale, "region")?,
		});
	}

	let options = Object::new();
	Reflect::set(
		&options,
		&JsValue::from_str("hour"),
		&JsValue::from_str("numeric"),
	)
	.map_err(|_| Error::Native)?;
	let formatter = construct(&intl, "DateTimeFormat", &Array::of2(&languages, &options))?;
	let resolved = invoke(&formatter, "resolvedOptions")?;
	let result = Snapshot {
		locales,
		calendar: text(&resolved, "calendar")?,
		time_zone: text(&resolved, "timeZone")?,
		hour_24: property(&resolved, "hour12")?
			.as_bool()
			.map(|hour12| !hour12),
		first_weekday,
	};
	Ok(result)
}
