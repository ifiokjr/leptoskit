//! `#[rpc::cron("…")]`: checks a cron expression when it compiles, and
//! registers the job above which it sits to be enqueued on its ticks.

use proc_macro2::Span;
use proc_macro2::TokenStream;
use quote::quote;
use syn::ItemFn;
use syn::LitStr;
use syn::Meta;
use syn::Path;
use syn::Token;
use syn::parse::Parser;
use syn::punctuated::Punctuated;

/// One parsed expression, as the bit sets `leptoskit_rpc`'s `Schedule`
/// holds.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Schedule {
	pub minutes: u64,
	pub hours: u64,
	pub days: u64,
	pub months: u64,
	pub weekdays: u64,
	pub any_day: bool,
	pub any_weekday: bool,
}

/// The fields of an expression, with their bounds.
const FIELDS: [(&str, u32, u32); 5] = [
	("minute", 0, 59),
	("hour", 0, 23),
	("day of the month", 1, 31),
	("month", 1, 12),
	("day of the week", 0, 7),
];

/// Parses a five-field cron expression, or a shortcut such as `@daily`.
pub(crate) fn parse(expression: &str) -> Result<Schedule, String> {
	let expanded = match expression.trim() {
		"@hourly" => "0 * * * *",
		"@daily" | "@midnight" => "0 0 * * *",
		"@weekly" => "0 0 * * 0",
		"@monthly" => "0 0 1 * *",
		"@yearly" | "@annually" => "0 0 1 1 *",

		other if other.starts_with('@') => {
			return Err(format!(
				"`{other}` isn't a shortcut; use `@hourly`, `@daily`, `@weekly`, `@monthly`, or \
				 `@yearly`"
			));
		}

		other => other,
	};

	let fields: Vec<&str> = expanded.split_whitespace().collect();

	if fields.len() != 5 {
		return Err(format!(
			"a cron expression has five fields, minute hour day-of-month month day-of-week, such \
			 as `*/5 * * * *`; this has {}",
			fields.len()
		));
	}

	let mut masks = [0_u64; 5];

	for (index, ((name, min, max), text)) in FIELDS.iter().zip(&fields).enumerate() {
		masks[index] = field(text, name, *min, *max)?;
	}

	// Sunday is 0 or 7.
	if masks[4] & (1 << 7) != 0 {
		masks[4] = (masks[4] | 1) & !(1 << 7);
	}

	Ok(Schedule {
		minutes: masks[0],
		hours: masks[1],
		days: masks[2],
		months: masks[3],
		weekdays: masks[4],
		any_day: fields[2] == "*",
		any_weekday: fields[4] == "*",
	})
}

/// One field's values as a bit set.
fn field(text: &str, name: &str, min: u32, max: u32) -> Result<u64, String> {
	let mut mask = 0;

	for part in text.split(',') {
		let (range, step) = field_step(part, name)?;
		let (from, to) = field_bounds(range, name, min, max, step.is_some())?;

		if from > to {
			return Err(format!(
				"the {name} field's range {from}-{to} runs backwards"
			));
		}

		let step = usize::try_from(step.unwrap_or(1)).unwrap_or(usize::MAX);

		for value in (from..=to).step_by(step) {
			mask |= 1 << value;
		}
	}

	Ok(mask)
}

/// A part's range and the step it takes through it.
fn field_step<'a>(part: &'a str, name: &str) -> Result<(&'a str, Option<u32>), String> {
	let Some((range, step)) = part.split_once('/') else {
		return Ok((part, None));
	};

	let step: u32 =
		step.parse().ok().filter(|step| *step > 0).ok_or_else(|| {
			format!("the {name} field's step `{step}` isn't a whole number above 0")
		})?;
	Ok((range, Some(step)))
}

/// A range's first and last value, where `*` spans the field.
fn field_bounds(
	range: &str,
	name: &str,
	min: u32,
	max: u32,
	stepped: bool,
) -> Result<(u32, u32), String> {
	match range {
		"*" => Ok((min, max)),
		range => {
			if let Some((from, to)) = range.split_once('-') {
				Ok((
					field_number(from, name, min, max)?,
					field_number(to, name, min, max)?,
				))
			} else {
				let from = field_number(range, name, min, max)?;

				Ok((from, if stepped { max } else { from }))
			}
		}
	}
}

/// A number in the field's range.
fn field_number(value: &str, name: &str, min: u32, max: u32) -> Result<u32, String> {
	let value: u32 = value.parse().map_err(|_| {
		format!("the {name} field has `{value}`; write numbers from {min} to {max}")
	})?;

	if !(min..=max).contains(&value) {
		return Err(format!(
			"the {name} field has {value}, outside {min} to {max}"
		));
	}

	Ok(value)
}
/// Expands `#[rpc::cron("…")]`. `server` decides whether the schedule is
/// registered.
pub(crate) fn expand(
	args: TokenStream,
	item: TokenStream,
	server: bool,
) -> syn::Result<TokenStream> {
	let mut parts = Punctuated::<syn::Expr, Token![,]>::parse_terminated
		.parse2(args)?
		.into_iter();
	let Some(syn::Expr::Lit(syn::ExprLit {
		lit: syn::Lit::Str(expression),
		..
	})) = parts.next()
	else {
		return Err(syn::Error::new(
			Span::call_site(),
			"#[rpc::cron] takes a cron expression, such as #[rpc::cron(\"*/5 * * * *\")]",
		));
	};
	let mut krate: Path = syn::parse_quote!(::leptoskit_rpc);

	for part in parts {
		let meta: Meta = syn::parse2(quote!(#part))?;

		match &meta {
			Meta::NameValue(pair) if pair.path.is_ident("crate") => {
				let path = crate::string_value(&pair.value, "crate")?;
				krate = path.parse::<Path>().map_err(|_| {
					syn::Error::new_spanned(
						&path,
						"`crate` expects the path to `leptoskit_rpc`, such as `crate = \
						 \"leptoskit::rpc\"`",
					)
				})?;
			}

			other => {
				return Err(syn::Error::new_spanned(
					other,
					"unknown #[rpc::cron] option; it takes the expression and `crate = \"...\"`",
				));
			}
		}
	}

	let schedule = parse(&expression.value())
		.map_err(|message| syn::Error::new_spanned(&expression, message))?;

	let function: ItemFn = syn::parse2(item)?;
	let is_job = function.attrs.iter().any(|attr| {
		attr.path()
			.segments
			.last()
			.is_some_and(|segment| segment.ident == "job")
	});

	if !is_job {
		return Err(syn::Error::new_spanned(
			&function.sig.ident,
			"#[rpc::cron] schedules a job: put it above #[rpc::job]",
		));
	}

	if function.sig.inputs.len() != 1 {
		return Err(syn::Error::new_spanned(
			&function.sig.inputs,
			"a cron job takes only its context, `cx: &rpc::JobCx`: nothing enqueues it with \
			 arguments",
		));
	}

	if !server {
		return Ok(quote!(#function));
	}

	let struct_ident = crate::operation::struct_ident(&function);
	let Schedule {
		minutes,
		hours,
		days,
		months,
		weekdays,
		any_day,
		any_weekday,
	} = schedule;
	let hours = u32::try_from(hours).unwrap_or(u32::MAX);
	let days = u32::try_from(days).unwrap_or(u32::MAX);
	let months = u16::try_from(months).unwrap_or(u16::MAX);
	let weekdays = u8::try_from(weekdays).unwrap_or(u8::MAX);
	let text = LitStr::new(&expression.value(), expression.span());
	Ok(quote! {
		#function

		#[allow(deprecated)]
		const _: () = {
			#krate::__private::inventory::submit! {
				#krate::server::jobs::cron::CronRegistration {
					name: <#struct_ident as #krate::Job>::NAME,
					expression: #text,
					schedule: #krate::server::jobs::cron::Schedule {
						minutes: #minutes,
						hours: #hours,
						days: #days,
						months: #months,
						weekdays: #weekdays,
						any_day: #any_day,
						any_weekday: #any_weekday,
					},
					enqueue: |db, key, tick| {
						#krate::__private::enqueue_tick(#struct_ident {}, db, key, tick)
					},
				}
			}
		};
	})
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn expressions_become_bit_sets() {
		let every_five = parse("*/5 * * * *").unwrap();
		assert_eq!(every_five.minutes, 0x0084_2108_4210_8421);
		assert_eq!(every_five.hours, (1 << 24) - 1);
		assert!(every_five.any_day && every_five.any_weekday);

		let workdays = parse("30 9 * * 1-5").unwrap();
		assert_eq!(workdays.minutes, 1 << 30);
		assert_eq!(workdays.hours, 1 << 9);
		assert_eq!(workdays.weekdays, 0b11_1110);
		assert!(workdays.any_day && !workdays.any_weekday);

		let sundays = parse("0 0 * * 7").unwrap();
		assert_eq!(sundays.weekdays, 1);
		assert_eq!(parse("@weekly").unwrap(), sundays);
		assert_eq!(parse("@daily").unwrap(), parse("0 0 * * *").unwrap());
		assert_eq!(parse("@hourly").unwrap().minutes, 1);
		assert_eq!(parse("@monthly").unwrap().days, 1 << 1);
		assert_eq!(parse("@yearly").unwrap().months, 1 << 1);

		let listed = parse("0,15,40-45/5 0 1,15 */3 *").unwrap();
		assert_eq!(listed.minutes, 1 | (1 << 15) | (1 << 40) | (1 << 45));
		assert_eq!(listed.days, (1 << 1) | (1 << 15));
		assert_eq!(listed.months, (1 << 1) | (1 << 4) | (1 << 7) | (1 << 10));
		assert_eq!(
			parse("10/20 * * * *").unwrap().minutes,
			(1 << 10) | (1 << 30) | (1 << 50)
		);
	}

	#[test]
	fn bad_expressions_say_why() {
		for (expression, message) in [
			("* * * *", "five fields"),
			("60 * * * *", "outside 0 to 59"),
			("* * 0 * *", "outside 1 to 31"),
			("*/0 * * * *", "above 0"),
			("5-1 * * * *", "runs backwards"),
			("* * * jan *", "write numbers"),
			("@often", "isn't a shortcut"),
		] {
			let error = parse(expression).unwrap_err();
			assert!(error.contains(message), "{expression}: {error}");
		}
	}
}
