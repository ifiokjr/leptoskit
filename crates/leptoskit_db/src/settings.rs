//! Settings: request values that Postgres can see, so the same policy rules
//! the database layer applies also run as row-level security.
//!
//! A rule that reads a global through `cx.global(|viewer: &Viewer| …)`
//! runs Rust code with the request's value, which no database can repeat.
//! A rule that reads it through `cx.setting::<Viewer>()` compares a column
//! with a value the database layer binds, or, in a row-level security
//! policy, with `current_setting('leptoskit.viewer')`, which each
//! transaction sets for its request.

use std::any::TypeId;
use std::marker::PhantomData;

use leptoskit_db_schema::ColumnType;
#[cfg(feature = "postgres")]
use leptoskit_db_schema::SETTING_PREFIX;

use crate::Column;
use crate::Context;
use crate::EncodeError;
use crate::Operand;
use crate::Value;

/// A global whose value the database can see: the signed-in user, the
/// tenant. Policies read it with `cx.setting::<G>()`, which works with and
/// without row-level security; register each one with
/// [`register_setting!`](crate::register_setting) so every transaction sets
/// it for Postgres.
///
/// ```ignore
/// /// The signed-in user.
/// pub struct Viewer(pub Id<User>);
///
/// impl db::Setting for Viewer {
/// 	const NAME: &'static str = "viewer";
/// 	type Value = Id<User>;
///
/// 	fn value(&self) -> Id<User> {
/// 		self.0
/// 	}
/// }
///
/// db::register_setting!(Viewer);
///
/// #[db::table(name = "posts")]
/// #[db::policy(select, |post, cx| post.published.eq(true) | post.author_id.eq(cx.setting::<Viewer>()))]
/// pub struct Post { … }
/// ```
pub trait Setting: Send + Sync + 'static {
	/// Its name, lowercase snake case. Postgres sees it as
	/// `leptoskit.<NAME>`.
	const NAME: &'static str;
	/// What it holds.
	type Value: Column;
	/// Its value for this request. Empty text is no value at all: Postgres
	/// reads an empty setting as `NULL`, so the database layer does too.
	fn value(&self) -> Self::Value;
}

/// The value of setting `V` for a request, inside a policy's rule. Compare
/// a column with it: [`Context::setting`] makes one. A request without the
/// setting compares with `NULL`, so the comparison matches no row, even
/// negated.
pub struct SettingValue<V> {
	name: &'static str,
	ty: ColumnType,
	value: Result<Option<Value>, String>,
	column: PhantomData<fn() -> V>,
}

/// A setting a condition compares a column with.
#[doc(hidden)]
#[derive(Clone, Debug, PartialEq)]
pub struct SettingRef {
	/// Its name, without the `leptoskit.` prefix.
	pub(crate) name: &'static str,
	/// The type Postgres reads it as.
	pub(crate) ty: ColumnType,
	/// The request's value, or `None` when it has none or when a policy is
	/// being rendered for row-level security.
	pub(crate) value: Option<Value>,
}

impl<V: Column> SettingValue<V> {
	pub(crate) fn of<G: Setting<Value = V>>(cx: &Context) -> Self {
		let value = match cx.setting_global::<G>() {
			// Postgres can't tell an empty setting from a missing one, so
			// neither can the database layer.
			Some(global) => {
				global
					.value()
					.to_value()
					.map(|value| (value != Value::Text(String::new())).then_some(value))
					.map_err(|error| error.to_string())
			}
			None => Ok(None),
		};

		Self {
			name: G::NAME,
			ty: V::column_type(),
			value,
			column: PhantomData,
		}
	}

	/// Whether the request holds this setting at all, as a condition: a
	/// policy uses it to ask for a signed-in person.
	pub fn is_present(&self) -> crate::Expr {
		crate::Expr::SettingPresent {
			setting: SettingRef {
				name: self.name,
				ty: self.ty.clone(),
				value: self.value.as_ref().ok().cloned().flatten(),
			},
		}
	}

	fn reference(&self) -> Result<SettingRef, String> {
		Ok(SettingRef {
			name: self.name,
			ty: self.ty.clone(),
			value: self.value.clone()?,
		})
	}
}

fn not_a_value(name: &str) -> EncodeError {
	EncodeError::new(format!(
		"setting `{name}` has no value of its own outside a policy's comparison"
	))
}

impl<V: Column> Operand<V> for SettingValue<V> {
	fn into_value(self) -> Result<Value, EncodeError> {
		Err(not_a_value(self.name))
	}

	fn setting(&self) -> Option<Result<SettingRef, String>> {
		Some(self.reference())
	}
}

impl<V: Column> Operand<Option<V>> for SettingValue<V> {
	fn into_value(self) -> Result<Value, EncodeError> {
		Err(not_a_value(self.name))
	}

	fn setting(&self) -> Option<Result<SettingRef, String>> {
		Some(self.reference())
	}
}

/// A setting that holds a list, such as the signed-in person's wallets or
/// teams. Policies ask whether a column is one of its items with
/// `column.in_list(cx.setting_list::<G>())`, which, like
/// [`Context::setting`], also runs as row-level security. Register each
/// one with [`register_list_setting!`](crate::register_list_setting).
///
/// ```ignore
/// /// The teams the signed-in user belongs to.
/// pub struct Teams(pub Vec<Id<Team>>);
///
/// impl db::ListSetting for Teams {
/// 	const NAME: &'static str = "teams";
/// 	type Item = Id<Team>;
///
/// 	fn values(&self) -> Vec<Id<Team>> {
/// 		self.0.clone()
/// 	}
/// }
///
/// db::register_list_setting!(Teams);
///
/// #[db::table(name = "projects")]
/// #[db::policy(select, |project, cx| project.team_id.in_list(cx.setting_list::<Teams>()))]
/// pub struct Project { … }
/// ```
pub trait ListSetting: Send + Sync + 'static {
	/// Its name, lowercase snake case. Postgres sees it as
	/// `leptoskit.<NAME>`, an array literal.
	const NAME: &'static str;
	/// What each item is.
	type Item: Column;
	/// Its items for this request.
	fn values(&self) -> Vec<Self::Item>;
}

/// The items of list setting `V` for a request, inside a policy's rule.
/// [`Context::setting_list`] makes one, and a column's `in_list` takes it.
/// A request without the setting matches no row, even negated, as
/// [`SettingValue`] does; one with an empty list is in no list, so a
/// negated rule matches every row.
pub struct SettingList<V> {
	name: &'static str,
	ty: ColumnType,
	values: Result<Option<Vec<Value>>, String>,
	column: PhantomData<fn() -> V>,
}

/// A list setting a condition looks a column up in.
#[doc(hidden)]
#[derive(Clone, Debug, PartialEq)]
pub struct ListSettingRef {
	/// Its name, without the `leptoskit.` prefix.
	pub(crate) name: &'static str,
	/// The type Postgres reads each item as.
	pub(crate) ty: ColumnType,
	/// The request's items, or `None` when it has none or when a policy is
	/// being rendered for row-level security.
	pub(crate) values: Option<Vec<Value>>,
}

impl<V: Column> SettingList<V> {
	pub(crate) fn of<G: ListSetting<Item = V>>(cx: &Context) -> Self {
		let values = match cx.setting_global::<G>() {
			Some(global) => {
				global
					.values()
					.iter()
					.map(Column::to_value)
					.collect::<Result<Vec<_>, _>>()
					.map(Some)
					.map_err(|error| error.to_string())
			}
			None => Ok(None),
		};

		Self {
			name: G::NAME,
			ty: V::column_type(),
			values,
			column: PhantomData,
		}
	}

	pub(crate) fn reference(self) -> Result<ListSettingRef, String> {
		Ok(ListSettingRef {
			name: self.name,
			ty: self.ty,
			values: self.values?,
		})
	}
}

/// One registered setting: how to read it from a request.
#[doc(hidden)]
pub struct SettingEntry {
	pub(crate) name: &'static str,
	pub(crate) type_id: fn() -> TypeId,
	pub(crate) type_name: fn() -> &'static str,
	#[cfg_attr(
		not(feature = "postgres"),
		expect(dead_code, reason = "only Postgres has settings to make")
	)]
	read: fn(&Context) -> Option<Result<String, EncodeError>>,
}

impl SettingEntry {
	pub const fn of<G: Setting>() -> Self {
		Self {
			name: G::NAME,
			type_id: TypeId::of::<G>,
			type_name: std::any::type_name::<G>,
			read: |cx| {
				cx.setting_global::<G>()
					.map(|global| global.value().to_value().map(|value| text(&value)))
			},
		}
	}

	pub const fn list<G: ListSetting>() -> Self {
		Self {
			name: G::NAME,
			type_id: TypeId::of::<G>,
			type_name: std::any::type_name::<G>,
			read: |cx| {
				cx.setting_global::<G>().map(|global| {
					global
						.values()
						.iter()
						.map(|item| item.to_value().map(|value| array_item(&value)))
						.collect::<Result<Vec<_>, _>>()
						.map(|items| format!("{{{}}}", items.join(",")))
				})
			},
		}
	}
}

inventory::collect!(SettingEntry);

/// Every registered setting.
pub(crate) fn entries() -> impl Iterator<Item = &'static SettingEntry> {
	inventory::iter::<SettingEntry>()
}

/// The registered setting of type `id`, if there is one.
pub(crate) fn registered(id: TypeId) -> bool {
	entries().any(|entry| (entry.type_id)() == id)
}

/// The settings a transaction makes for `access`, as `(name, text)`:
/// every registered setting, empty when the request has none, and whether
/// it bypasses policies. Postgres reads an empty setting as `NULL`.
#[cfg(feature = "postgres")]
pub(crate) fn for_access(
	access: &crate::policy::Access,
) -> Result<Vec<(String, String)>, EncodeError> {
	let mut settings = vec![(
		leptoskit_db_schema::BYPASS_SETTING.to_string(),
		match access {
			crate::policy::Access::System { .. } => "on".to_string(),
			crate::policy::Access::Context(_) => String::new(),
		},
	)];

	for entry in entries() {
		let value = match access {
			crate::policy::Access::Context(cx) => (entry.read)(cx).transpose()?,
			crate::policy::Access::System { .. } => None,
		};

		settings.push((
			format!("{SETTING_PREFIX}{}", entry.name),
			value.unwrap_or_default(),
		));
	}

	Ok(settings)
}

/// A value as the text `current_setting` returns and a cast reads back.
fn text(value: &Value) -> String {
	match value {
		Value::Null => String::new(),
		Value::Bool(value) => value.to_string(),
		Value::Int(value) => value.to_string(),
		Value::Float(value) => float_text(*value),
		Value::Text(value) => value.clone(),
		Value::Bytes(value) => format!("\\x{}", hex(value)),
		Value::Uuid(value) => value.to_string(),
		Value::Timestamp(value) => value.to_rfc3339(),
		Value::Date(value) => value.to_string(),
		Value::Json(value) => value.to_string(),
	}
}

/// One item of a Postgres array literal: quoted, so commas, braces, spaces
/// and the word `NULL` are text like any other, unless it is `NULL`.
fn array_item(value: &Value) -> String {
	if *value == Value::Null {
		return "NULL".into();
	}

	let text = text(value);
	format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

fn float_text(value: f64) -> String {
	if value.is_nan() {
		"NaN".into()
	} else if value.is_infinite() {
		if value > 0.0 { "Infinity" } else { "-Infinity" }.into()
	} else {
		value.to_string()
	}
}

fn hex(bytes: &[u8]) -> String {
	bytes
		.iter()
		.fold(String::with_capacity(bytes.len() * 2), |mut hex, byte| {
			use std::fmt::Write;
			let _ = write!(hex, "{byte:02x}");
			hex
		})
}

/// A value as a Postgres literal, for a policy's SQL, which has no bound
/// values. Text is an escape string, so backslashes mean the same whatever
/// `standard_conforming_strings` says.
pub(crate) fn postgres_literal(value: &Value) -> String {
	let quoted = |text: &str| format!("E'{}'", text.replace('\\', "\\\\").replace('\'', "''"));

	match value {
		Value::Null => "NULL".into(),
		Value::Bool(true) => "TRUE".into(),
		Value::Bool(false) => "FALSE".into(),
		Value::Int(value) => value.to_string(),
		Value::Float(value) => format!("{}::float8", quoted(&float_text(*value))),
		Value::Text(value) => quoted(value),
		Value::Bytes(value) => format!("{}::bytea", quoted(&format!("\\x{}", hex(value)))),
		Value::Uuid(value) => format!("'{value}'::uuid"),
		Value::Timestamp(value) => format!("'{}'::timestamptz", value.to_rfc3339()),
		Value::Date(value) => format!("'{value}'::date"),
		Value::Json(value) => format!("{}::jsonb", quoted(&value.to_string())),
	}
}

/// Registers settings, so that each transaction under row-level security
/// sets them for Postgres. Write it beside each type that implements
/// [`Setting`]:
///
/// ```ignore
/// leptoskit::db::register_setting!(Viewer, Tenant);
/// ```
///
/// Exporting the schema for row-level security fails when a policy reads a
/// setting that isn't registered.
#[macro_export]
macro_rules! register_setting {
	($($setting:ty),+ $(,)?) => {
		$crate::__server! {
			$($crate::__private::inventory::submit! {
				$crate::__private::SettingEntry::of::<$setting>()
			})+
		}
	};
}

/// Registers list settings, as [`register_setting!`](crate::register_setting)
/// does settings. Write it beside each type that implements
/// [`ListSetting`]:
///
/// ```ignore
/// leptoskit::db::register_list_setting!(Teams);
/// ```
#[macro_export]
macro_rules! register_list_setting {
	($($setting:ty),+ $(,)?) => {
		$crate::__server! {
			$($crate::__private::inventory::submit! {
				$crate::__private::SettingEntry::list::<$setting>()
			})+
		}
	};
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn literals_are_escaped_for_any_server_setting() {
		assert_eq!(
			postgres_literal(&Value::Text("it's \\ok".into())),
			r"E'it''s \\ok'"
		);
		assert_eq!(postgres_literal(&Value::Bool(true)), "TRUE");
		assert_eq!(postgres_literal(&Value::Bool(false)), "FALSE");
		assert_eq!(postgres_literal(&Value::Null), "NULL");
		assert_eq!(postgres_literal(&Value::Int(-3)), "-3");
		assert_eq!(postgres_literal(&Value::Float(1.5)), "E'1.5'::float8");
		assert_eq!(postgres_literal(&Value::Float(f64::NAN)), "E'NaN'::float8");
		assert_eq!(
			postgres_literal(&Value::Float(f64::NEG_INFINITY)),
			"E'-Infinity'::float8"
		);

		assert_eq!(
			postgres_literal(&Value::Bytes(vec![0, 255])),
			r"E'\\x00ff'::bytea"
		);
		let uuid = uuid::Uuid::nil();
		assert_eq!(
			postgres_literal(&Value::Uuid(uuid)),
			"'00000000-0000-0000-0000-000000000000'::uuid"
		);
		assert_eq!(
			postgres_literal(&Value::Json(serde_json::json!({"a": "b'c"}))),
			r#"E'{"a":"b''c"}'::jsonb"#
		);
		let date = chrono::NaiveDate::from_ymd_opt(2026, 1, 2).unwrap();
		assert_eq!(postgres_literal(&Value::Date(date)), "'2026-01-02'::date");
		let at = date.and_hms_opt(3, 4, 5).unwrap().and_utc();
		assert_eq!(
			postgres_literal(&Value::Timestamp(at)),
			"'2026-01-02T03:04:05+00:00'::timestamptz"
		);
	}

	#[cfg(feature = "postgres")]
	#[test]
	fn settings_travel_as_text() {
		let uuid = uuid::Uuid::nil();
		assert_eq!(text(&Value::Uuid(uuid)), uuid.to_string());
		assert_eq!(text(&Value::Null), "");
		assert_eq!(text(&Value::Bool(true)), "true");
		assert_eq!(text(&Value::Int(7)), "7");
		assert_eq!(text(&Value::Float(f64::INFINITY)), "Infinity");
		assert_eq!(text(&Value::Text("a".into())), "a");
		assert_eq!(text(&Value::Bytes(vec![1])), "\\x01");

		assert_eq!(text(&Value::Json(serde_json::json!([1]))), "[1]");
		let date = chrono::NaiveDate::from_ymd_opt(2026, 1, 2).unwrap();
		assert_eq!(text(&Value::Date(date)), "2026-01-02");
		assert_eq!(
			text(&Value::Timestamp(
				date.and_hms_opt(0, 0, 0).unwrap().and_utc()
			)),
			"2026-01-02T00:00:00+00:00"
		);
	}

	/// A text setting.
	struct Label(&'static str);

	impl Setting for Label {
		type Value = String;

		const NAME: &'static str = "label";

		fn value(&self) -> String {
			self.0.to_string()
		}
	}

	#[test]
	fn empty_text_is_no_setting_as_postgres_reads_it() {
		let value = |label| {
			SettingValue::<String>::of::<Label>(&Context::new().with(Label(label)))
				.reference()
				.unwrap()
				.value
		};

		assert_eq!(value("a"), Some(Value::Text("a".into())));
		assert_eq!(value(""), None);
	}

	#[test]
	fn list_items_are_quoted_for_an_array_literal() {
		assert_eq!(array_item(&Value::Text("a".into())), r#""a""#);
		assert_eq!(
			array_item(&Value::Text(r#"{x, "y"} \ NULL"#.into())),
			r#""{x, \"y\"} \\ NULL""#
		);
		assert_eq!(array_item(&Value::Text(String::new())), r#""""#);
		assert_eq!(array_item(&Value::Null), "NULL");
		assert_eq!(array_item(&Value::Bytes(vec![1])), r#""\\x01""#);
	}
}
