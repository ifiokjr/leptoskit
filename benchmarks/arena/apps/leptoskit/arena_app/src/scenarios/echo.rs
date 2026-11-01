//! `echo`: round trips to the app's native `ArenaEcho` module.

use crate::host::Host;
use crate::report::Report;
use crate::services::Arena;

/// Runs the async tier. The browser has no native layer, so it fails there.
#[cfg(not(feature = "native"))]
pub async fn run(host: Host, _arena: &Arena) -> Result<Report, String> {
	match host.echo() {
		Ok(never) => match never {},
		Err(error) => Err(error),
	}
}

#[cfg(feature = "native")]
pub use native::run;

#[cfg(feature = "native")]
mod native {
	use std::future::Future;

	use arena_bridge::guest::Arena as Echo;
	use leptoskit::prelude::Set;
	use serde_json::Map;
	use serde_json::Value;

	use super::*;
	use crate::report;
	use crate::services::Phase;
	use crate::services::Scenario;
	use crate::services::Screen;

	const WARM_UP: usize = 200;
	const TIMED: usize = 2_000;
	const BATCH: usize = 100;

	/// Runs every payload through the async tier.
	pub async fn run(host: Host, arena: &Arena) -> Result<Report, String> {
		let echo = host.echo()?;
		let mut report = Report::new(Scenario::Echo);
		let text: String = (0..1024)
			.map(|index| char::from(b'a' + (index % 26) as u8))
			.collect();
		let bytes: Vec<u8> = (0..65_536_u32).map(|index| (index % 251) as u8).collect();
		let object = Value::Object(
			(0..100)
				.map(|field| (format!("f{field}"), Value::from(field)))
				.collect::<Map<_, _>>(),
		);
		let object_json = object.to_string();

		arena.update(|model| {
			model.phase.set(Phase::Running);
			model.screen = Screen::Echo("timing the async tier".to_owned());
		});

		let clock = clock_overhead(host).await?;
		report.metric("clock_p50_us", report::median(&clock));
		report.samples("clock_p50_us", &report::batch_means(&clock, BATCH));

		measure(&mut report, host, "async_none", || {
			async move { echo.echo().await.map_err(describe) }
		})
		.await?;
		measure(&mut report, host, "async_int", || {
			async move {
				let value = echo.add_one(41).await.map_err(describe)?;
				expect(value == 42, "addOne(41)")
			}
		})
		.await?;
		measure(&mut report, host, "async_str1k", || {
			let text = text.clone();
			async move {
				let reply = echo.echo_string(&text).await.map_err(describe)?;
				expect(reply == text, "str1k")
			}
		})
		.await?;
		measure(&mut report, host, "async_bytes64k", || {
			let bytes = &bytes;
			async move {
				let reply = echo.echo_bytes(bytes).await.map_err(describe)?;
				expect(reply.as_ref() == bytes.as_slice(), "bytes64k")
			}
		})
		.await?;
		measure(&mut report, host, "async_obj", || {
			let (object, object_json) = (&object, &object_json);
			async move { echo_object(echo, object, object_json).await }
		})
		.await?;

		arena.update(|model| {
			model.phase.set(Phase::Done);
			model.screen = Screen::Echo("done".to_owned());
		});
		host.next_frame().await?;

		report.note(
			"no synchronous guest-to-native call exists: every plugin call is submitted, then \
			 resumed after the host pumps it, so there is no sync tier",
		);
		report.note(
			"each sample includes one call to the app's Rust clock plugin, because the guest has \
			 no clock; clock_p50_us is that call's own cost",
		);
		report.note("bytes64k crosses the bridge as binary");
		report.note(
			"obj is JSON-encoded in the guest, decoded and re-encoded natively, and decoded in \
			 the guest; the bridge carries only bytes",
		);
		Ok(report)
	}

	/// Back-to-back clock readings: what one reading adds to every sample.
	async fn clock_overhead(host: Host) -> Result<Vec<f64>, String> {
		let mut previous = host.now_ms().await?;
		let mut gaps = Vec::with_capacity(TIMED);

		for _ in 0..TIMED {
			let now = host.now_ms().await?;
			gaps.push((now - previous) * 1000.0);
			previous = now;
		}

		Ok(gaps)
	}

	/// Warms up, then times 2,000 sequential calls, each verified.
	async fn measure<F, C>(
		report: &mut Report,
		host: Host,
		key: &str,
		call: C,
	) -> Result<(), String>
	where
		C: Fn() -> F,
		F: Future<Output = Result<(), String>>,
	{
		for _ in 0..WARM_UP {
			call().await?;
		}

		let mut micros = Vec::with_capacity(TIMED);
		let mut previous = host.now_ms().await?;

		for _ in 0..TIMED {
			call().await?;
			let now = host.now_ms().await?;
			micros.push((now - previous) * 1000.0);
			previous = now;
		}

		let total_seconds = micros.iter().sum::<f64>() / 1e6;
		report.metric(&format!("{key}_p50_us"), report::median(&micros));
		report.metric(&format!("{key}_p99_us"), report::percentile(&micros, 99.0));
		report.metric(&format!("{key}_per_s"), TIMED as f64 / total_seconds);
		// The contract keys samples like the metric they stand behind; for
		// echo they are the means of consecutive batches of 100 calls.
		report.samples(
			&format!("{key}_p50_us"),
			&report::batch_means(&micros, BATCH),
		);
		Ok(())
	}

	async fn echo_object(echo: Echo, object: &Value, json: &str) -> Result<(), String> {
		let reply = echo.echo_object(json.as_bytes()).await.map_err(describe)?;
		let decoded: Value = serde_json::from_slice(&reply).map_err(|error| error.to_string())?;
		expect(&decoded == object, "obj")
	}

	fn expect(matched: bool, payload: &str) -> Result<(), String> {
		matched
			.then_some(())
			.ok_or_else(|| format!("{payload} came back changed"))
	}

	fn describe(error: leptoskit_bridge::Error) -> String {
		format!("echo: {error:?}")
	}
}
