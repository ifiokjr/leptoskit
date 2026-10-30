//! Alarms: waking the worker later, once or repeatedly. Browsers round
//! periods up to at least 30 seconds.

use serde_json::Value;
use serde_json::json;

use crate::Result;
use crate::js;

/// Wakes the worker every `minutes`, starting `minutes` from now, with an
/// [`Alarm`](crate::background::Alarm) named `name`. Creating an alarm with
/// an existing name replaces it.
///
/// # Errors
///
/// Without the `alarms` permission.
pub async fn every(name: &str, minutes: f64) -> Result<()> {
	js::call::<Value>(
		"alarms.create",
		&[
			&name,
			&json!({ "delayInMinutes": minutes, "periodInMinutes": minutes }),
		],
	)
	.await
	.map(drop)
}

/// Wakes the worker once, `minutes` from now.
///
/// # Errors
///
/// Without the `alarms` permission.
pub async fn once(name: &str, minutes: f64) -> Result<()> {
	js::call::<Value>(
		"alarms.create",
		&[&name, &json!({ "delayInMinutes": minutes })],
	)
	.await
	.map(drop)
}

/// Cancels the alarm `name`. Returns whether there was one.
///
/// # Errors
///
/// Without the `alarms` permission.
pub async fn clear(name: &str) -> Result<bool> {
	js::call("alarms.clear", &[&name]).await
}
