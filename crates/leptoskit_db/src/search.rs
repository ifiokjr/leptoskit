//! Full-text search over a table's `#[db(search)]` columns.
//!
//! Postgres matches a generated `tsvector` column through its GIN index and
//! ranks with `ts_rank`. SQLite matches an FTS5 table that triggers keep in
//! step with the table, and ranks with `bm25`. Both read the same query
//! syntax, parsed here, and both filter through the table's `select` policy
//! like any other query.

use std::marker::PhantomData;

use leptoskit_db_schema::Dialect;
use leptoskit_db_schema::SEARCH_COLUMN;
use leptoskit_db_schema::SEARCH_ROW;
use leptoskit_db_schema::Search as SearchIndex;
use leptoskit_db_schema::SearchWeight;
use leptoskit_db_schema::quote_identifier;
use leptoskit_db_schema::quote_literal;

use crate::Action;
use crate::Col;
use crate::Column;
use crate::DbError;
use crate::Expr;
use crate::Order;
use crate::Query;
use crate::Read;
use crate::Result;
use crate::Shape;
use crate::Table;
use crate::TextColumn;
use crate::Value;
use crate::policy;
use crate::shape::Selected;
use crate::sql::Sql;
use crate::tracked::ReadKind;
use crate::tracked::Reads;

/// A table with a full-text search index: `#[db(search)]` on some of its
/// text columns. The attribute implements it; you don't.
pub trait Searchable: Table {
	/// How words are split and folded: `simple`, `english`, or on Postgres
	/// any text search configuration it has.
	const LANGUAGE: &'static str;
	/// The searched columns, in declaration order, each with its weight.
	const SEARCHED: &'static [(&'static str, SearchWeight)];
}

/// The character a highlighted match starts with, while it travels from the
/// database. Searched text is indexed without it.
const OPEN: char = '\u{E000}';
/// The character a highlighted match ends with.
const CLOSE: char = '\u{E001}';

/// The most terms one search may have, which keeps the statement small.
const MAX_TERMS: usize = 32;

/// The most words an excerpt may have: SQLite's `snippet` limit.
const MAX_EXCERPT: u32 = 64;

impl<T: Searchable> Query<T> {
	/// The rows whose searched columns match `text`, best match first:
	///
	/// ```ignore
	/// let hits = Article::query().search("passkeys rust").limit(10).all(&db).await?;
	/// ```
	///
	/// `text` reads like a web search box: every word must match, `"a
	/// phrase"` must match as written, `-word` must not match, `OR` between
	/// words matches either side, and a trailing `*` on a word of letters
	/// and digits matches any word it begins. Words are matched as the
	/// table's search language splits and folds them. Punctuation never
	/// makes the query invalid; a search with no words to match finds
	/// nothing.
	///
	/// The query's filters, `select` policy, and soft-delete filter apply
	/// as they do to [`Query::all`], and its orderings break ties between
	/// equally relevant rows.
	pub fn search(self, text: &str) -> Search<T> {
		Search {
			query: self,
			terms: Terms::parse(text),
			marks: Vec::new(),
			problem: None,
			table: PhantomData,
		}
	}
}

/// A search of table `T`, ranked by relevance. Start one with
/// [`Query::search`].
#[must_use = "a search does nothing until it runs"]
pub struct Search<T> {
	query: Query<T>,
	terms: std::result::Result<Terms, String>,
	marks: Vec<Mark>,
	/// A highlight that can't be made, reported when the search runs.
	problem: Option<String>,
	table: PhantomData<fn() -> T>,
}

/// A searched column to return with each hit, its matches marked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Mark {
	/// The column's position in [`Searchable::SEARCHED`].
	column: usize,
	/// The most words of an excerpt, or `None` for the whole text.
	words: Option<u32>,
}

impl<T: Searchable> Search<T> {
	/// Keeps only rows matching `condition` as well.
	pub fn filter(mut self, condition: impl FnOnce(&T::Columns) -> Expr) -> Self {
		self.query = self.query.filter(condition);
		self
	}

	/// Orders equally relevant rows. Later orderings break ties in earlier
	/// ones; the key breaks the rest.
	pub fn order_by(mut self, order: impl FnOnce(&T::Columns) -> Order) -> Self {
		self.query = self.query.order_by(order);
		self
	}

	/// At most this many rows.
	pub fn limit(mut self, limit: u64) -> Self {
		self.query = self.query.limit(limit);
		self
	}

	/// Skips this many rows.
	pub fn offset(mut self, offset: u64) -> Self {
		self.query = self.query.offset(offset);
		self
	}

	/// Includes soft-deleted rows.
	pub fn with_deleted(mut self) -> Self {
		self.query = self.query.with_deleted();
		self
	}

	/// Returns a searched column's whole text with each hit, its matching
	/// words marked: see [`Hit::highlights`]. Highlights come in the order
	/// they're asked for.
	pub fn highlight<V: Column + TextColumn>(
		self,
		column: impl FnOnce(&T::Columns) -> Col<T, V>,
	) -> Self {
		self.mark(column(T::columns()).name(), None)
	}

	/// Returns an excerpt of a searched column with each hit: about `words`
	/// words (2 to 64) around its best matches, the matches marked. Postgres
	/// and SQLite choose excerpts differently, so the same row's excerpt
	/// may differ between them.
	pub fn excerpt<V: Column + TextColumn>(
		mut self,
		column: impl FnOnce(&T::Columns) -> Col<T, V>,
		words: u32,
	) -> Self {
		if !(2..=MAX_EXCERPT).contains(&words) {
			self.problem.get_or_insert_with(|| {
				format!("an excerpt has 2 to {MAX_EXCERPT} words, not {words}")
			});
		}

		self.mark(column(T::columns()).name(), Some(words))
	}

	fn mark(mut self, name: &str, words: Option<u32>) -> Self {
		match T::SEARCHED
			.iter()
			.position(|(searched, _)| *searched == name)
		{
			Some(column) => self.marks.push(Mark { column, words }),
			None => {
				self.problem.get_or_insert_with(|| {
					format!(
						"`{name}` isn't searched, so it has no matches to highlight; add \
						 `#[db(search)]` to it"
					)
				});
			}
		}

		self
	}

	/// The matching rows, best first.
	pub async fn all(self, connection: &impl Read) -> Result<Vec<T>> {
		Ok(self
			.hits(connection)
			.await?
			.into_iter()
			.map(|hit| hit.row)
			.collect())
	}

	/// The best matching row, if any.
	pub async fn first(self, connection: &impl Read) -> Result<Option<T>> {
		Ok(self.limit(1).all(connection).await?.into_iter().next())
	}

	/// The matching rows, best first, each with its rank and the
	/// highlights asked for.
	pub async fn hits(self, connection: &impl Read) -> Result<Vec<Hit<T>>> {
		self.found(connection, &Output::Rows, T::from_values, |_, reads| reads)
			.await
	}

	/// Each match as shape `S`, best first, with the search's filters,
	/// orderings, limit, and highlights:
	///
	/// ```ignore
	/// let cards: Vec<ArticleCard> = Article::query()
	/// 	.search("passkeys rust")
	/// 	.limit(10)
	/// 	.shape::<ArticleCard>()
	/// 	.all(&db)
	/// 	.await?;
	/// ```
	///
	/// A shape's nested rows follow their own tables' policies, as they do
	/// in [`Query::shape`].
	pub fn shape<S: Shape<From = T>>(self) -> ShapedSearch<S> {
		ShapedSearch { search: self }
	}

	/// The matching rows, each read by `read` from what `output` selects,
	/// with its rank and highlights. `shaped` adds what a shape's nested
	/// rows read to the search's own read.
	async fn found<R, E>(
		&self,
		connection: &impl Read,
		output: &Output,
		read: impl Fn(Vec<Value>) -> std::result::Result<R, E>,
		shaped: fn(&policy::Access, Reads) -> Reads,
	) -> Result<Vec<Hit<R>>>
	where
		DbError: From<E>,
	{
		let Some(terms) = self.prepared()? else {
			return Ok(Vec::new());
		};
		let mut sql = Sql::new(connection.dialect());
		self.write(&mut sql, connection, terms, output)?;
		let marks = self.marks.len();
		let width = output.width::<T>();

		connection
			.read(sql.finish(), || {
				Ok(shaped(
					connection.access(),
					self.reads(connection, ReadKind::Rows)?,
				))
			})
			.await?
			.into_iter()
			.map(|mut values| {
				let extra = values.split_off(width);
				let row = read(values)?;
				let mut extra = extra.into_iter();
				let rank = match extra.next() {
					Some(Value::Float(rank)) => rank,
					Some(Value::Int(rank)) => rank as f64,
					_ => return Err(decode("the search returned no rank")),
				};
				let highlights = extra
					.by_ref()
					.take(marks)
					.map(|value| {
						match value {
							Value::Text(text) => Ok(Highlighted::parse(&text)),
							Value::Null => Ok(Highlighted::default()),
							_ => Err(decode("a highlight isn't text")),
						}
					})
					.collect::<Result<Vec<_>>>()?;

				Ok(Hit {
					row,
					rank,
					highlights,
				})
			})
			.collect()
	}

	/// How many rows match.
	pub async fn count(self, connection: &impl Read) -> Result<u64> {
		let Some(terms) = self.prepared()? else {
			return Ok(0);
		};
		let mut sql = Sql::new(connection.dialect());
		self.write(&mut sql, connection, terms, &Output::Count)?;
		let rows = connection
			.read(sql.finish(), || self.reads(connection, ReadKind::Count))
			.await?;
		crate::sql::count_of(&rows)
	}

	/// What the search reads, for a read set: the query's rows, narrowed by
	/// a match only the database can decide.
	fn reads(&self, connection: &impl Read, kind: ReadKind) -> Result<Reads> {
		let reads = self
			.query
			.reads(connection, kind)?
			.narrowed(Expr::Opaque("a full-text search's match"));

		// The rank orders the rows before any column does.
		Ok(match kind {
			ReadKind::Count => reads,
			_ => reads.tail(&[], self.query.limit, self.query.offset),
		})
	}

	/// The search's terms, or `None` when it has none to match, so it finds
	/// nothing; or the reason it can't run.
	fn prepared(&self) -> Result<Option<&Terms>> {
		if let Some(problem) = &self.problem {
			return Err(crate::expr::invalid(problem));
		}

		match &self.terms {
			Ok(terms) if terms.groups.is_empty() => Ok(None),
			Ok(terms) => Ok(Some(terms)),
			Err(problem) => Err(crate::expr::invalid(problem)),
		}
	}

	fn write(
		&self,
		sql: &mut Sql,
		connection: &impl Read,
		terms: &Terms,
		output: &Output,
	) -> Result<()> {
		let access = connection.access();
		let policy = policy::condition::<T>(access, Action::Select);
		// A shape's nested rows name the searched row by an alias.
		let row = match output {
			Output::Shape(_) => {
				let (table, alias) = sql.open(&T::table_name(), "shape");
				Row::Aliased { table, alias }
			}
			Output::Count | Output::Rows => Row::Named,
		};

		match sql.dialect() {
			Dialect::Postgres => self.write_postgres(sql, access, terms, output, &row)?,
			Dialect::Sqlite => self.write_sqlite(sql, access, terms, output, &row)?,
		}

		crate::query::write_conditions::<T>(
			sql,
			self.query.filter.as_ref(),
			policy.as_ref(),
			self.query.with_deleted,
			sql.dialect() == Dialect::Postgres,
		)?;

		if !matches!(output, Output::Count) {
			// The rank follows the row's own columns, or the shape's fields.
			sql.push(&format!(" ORDER BY {} DESC", output.width::<T>() + 1));

			for order in &self.query.order {
				sql.push(", ");
				order.write(sql);
			}

			for key in T::KEY {
				sql.push(", ");

				// A shape's fields may share the key's name, and an `ORDER
				// BY` name means the output column first.
				if let Row::Aliased { alias, .. } = &row {
					sql.push(alias).push(".");
				}

				sql.identifier(key).push(" ASC");
			}

			crate::query::write_order(sql, &[], self.query.limit, self.query.offset);
		}

		if matches!(row, Row::Aliased { .. }) {
			sql.leave();
		}

		Ok(())
	}

	/// `SELECT count(*)`, the row's columns, or the shape's fields.
	fn write_select(
		sql: &mut Sql,
		access: &policy::Access,
		output: &Output,
		row: &Row,
	) -> Result<()> {
		sql.push("SELECT ");

		match (output, row) {
			(Output::Count, _) => {
				sql.push("count(*)");
			}
			(Output::Shape(fields), Row::Aliased { alias, .. }) => {
				crate::shape::write_fields(sql, access, fields, alias, false)?;
			}
			(Output::Rows | Output::Shape(_), _) => {
				sql.columns(T::COLUMNS);
			}
		}

		Ok(())
	}

	/// `SELECT …, ts_rank(…), ts_headline(…) FROM table WHERE lk_search @@
	/// query`, and the caller's conditions after it.
	fn write_postgres(
		&self,
		sql: &mut Sql,
		access: &policy::Access,
		terms: &Terms,
		output: &Output,
		row: &Row,
	) -> Result<()> {
		let language = quote_literal(T::LANGUAGE);
		let vector = quote_identifier(SEARCH_COLUMN);
		Self::write_select(sql, access, output, row)?;

		if !matches!(output, Output::Count) {
			sql.push(&format!(", ts_rank({vector}, "));
			terms.write_tsquery(sql, &language);
			sql.push(")");

			for mark in &self.marks {
				let (column, _) = T::SEARCHED[mark.column];
				let options = match mark.words {
					None => format!("StartSel={OPEN}, StopSel={CLOSE}, HighlightAll=true"),
					Some(words) => {
						format!(
							"StartSel={OPEN}, StopSel={CLOSE}, MaxWords={words}, MinWords={}, \
							 MaxFragments=1",
							(words / 2).max(1)
						)
					}
				};
				sql.push(&format!(
					", ts_headline({language}, replace(replace(coalesce({}, ''), chr(57344), ''), \
					 chr(57345), ''), ",
					quote_identifier(column)
				));
				terms.write_tsquery(sql, &language);
				sql.push(", ").value(Value::Text(options)).push(")");
			}
		}

		sql.push(" FROM ");
		row.write_from::<T>(sql);
		sql.push(&format!(" WHERE {vector} @@ "));
		terms.write_tsquery(sql, &language);
		Ok(())
	}

	/// `SELECT …, rank, highlights FROM table JOIN (the FTS5 matches) ON
	/// key`, numbering each match's row through the companion table.
	fn write_sqlite(
		&self,
		sql: &mut Sql,
		access: &policy::Access,
		terms: &Terms,
		output: &Output,
		row: &Row,
	) -> Result<()> {
		let name = T::table_name();
		let table = match row {
			Row::Named => quote_identifier(&name.flat()),
			Row::Aliased { alias, .. } => alias.clone(),
		};
		let text = quote_identifier(&SearchIndex::text_table(&name));
		let rows = quote_identifier(&SearchIndex::rows_table(&name));
		let hits = quote_identifier("lk_hits");
		let keys: Vec<(String, String)> = T::KEY
			.iter()
			.enumerate()
			.map(|(index, key)| {
				(
					quote_identifier(key),
					quote_identifier(&format!("lk_key_{index}")),
				)
			})
			.collect();
		let count = matches!(output, Output::Count);
		Self::write_select(sql, access, output, row)?;

		if !count {
			sql.push(&format!(", {hits}.\"lk_rank\""));

			for index in 0..self.marks.len() {
				sql.push(&format!(", {hits}.\"lk_mark_{index}\""));
			}
		}

		sql.push(" FROM ");
		row.write_from::<T>(sql);
		sql.push(" JOIN (SELECT ");
		let mut selected: Vec<String> = keys
			.iter()
			.map(|(key, alias)| format!("\"lk_rows\".{key} AS {alias}"))
			.collect();

		if !count {
			let weights: Vec<String> = T::SEARCHED
				.iter()
				.map(|(_, weight)| format!("{:?}", weight.factor()))
				.collect();
			// `bm25` is lower for better matches; ranks are higher.
			selected.push(format!(
				"-bm25({text}, {}) AS \"lk_rank\"",
				weights.join(", ")
			));

			for (index, mark) in self.marks.iter().enumerate() {
				let function = match mark.words {
					None => {
						format!(
							"highlight({text}, {}, char(57344), char(57345))",
							mark.column
						)
					}
					Some(words) => {
						format!(
							"snippet({text}, {}, char(57344), char(57345), '', {words})",
							mark.column
						)
					}
				};
				selected.push(format!("{function} AS \"lk_mark_{index}\""));
			}
		}

		let joined: Vec<String> = keys
			.iter()
			.map(|(key, alias)| format!("{hits}.{alias} = {table}.{key}"))
			.collect();
		sql.push(&selected.join(", "))
			.push(&format!(
				" FROM {text} JOIN {rows} AS \"lk_rows\" ON \"lk_rows\".{} = {text}.\"rowid\" \
				 WHERE {text} MATCH ",
				quote_identifier(SEARCH_ROW)
			))
			.value(Value::Text(terms.fts5()))
			.push(&format!(") AS {hits} ON {}", joined.join(" AND ")));
		Ok(())
	}
}

/// What a search returns for each match.
enum Output {
	/// How many rows match.
	Count,
	/// Each row's own columns.
	Rows,
	/// A shape's fields.
	Shape(Vec<Selected>),
}

impl Output {
	/// How many values each match has before its rank.
	fn width<T: Table>(&self) -> usize {
		match self {
			Self::Count => 1,
			Self::Rows => T::COLUMNS.len(),
			Self::Shape(fields) => fields.len(),
		}
	}
}

/// How a search's statement names the searched row: by its table's name, or,
/// for a shape whose nested rows refer to it, by an alias.
enum Row {
	Named,
	Aliased { table: String, alias: String },
}

impl Row {
	fn write_from<T: Table>(&self, sql: &mut Sql) {
		match self {
			Self::Named => {
				sql.table::<T>();
			}
			Self::Aliased { table, alias } => {
				sql.push(table).push(" AS ").push(alias);
			}
		}
	}
}

/// A search that returns each match as shape `S`: [`Search::shape`] starts
/// one.
#[must_use = "a search does nothing until it runs"]
pub struct ShapedSearch<S: Shape> {
	search: Search<S::From>,
}

impl<S: Shape> ShapedSearch<S>
where
	S::From: Searchable,
{
	/// The matching rows' shapes, best first.
	pub async fn all(self, connection: &impl Read) -> Result<Vec<S>> {
		Ok(self
			.hits(connection)
			.await?
			.into_iter()
			.map(|hit| hit.row)
			.collect())
	}

	/// The best matching row's shape, if any.
	pub async fn first(self, connection: &impl Read) -> Result<Option<S>> {
		let first = ShapedSearch {
			search: self.search.limit(1),
		};
		Ok(first.all(connection).await?.into_iter().next())
	}

	/// The matching rows' shapes, best first, each with its rank and the
	/// highlights asked for.
	pub async fn hits(self, connection: &impl Read) -> Result<Vec<Hit<S>>> {
		self.search
			.found(
				connection,
				&Output::Shape(S::selection()),
				S::from_values,
				crate::shape::shaped_reads::<S>,
			)
			.await
	}
}

fn decode(message: &str) -> DbError {
	DbError::Decode(crate::DecodeError::custom(message))
}

/// One row a search found.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit<T> {
	/// The row.
	pub row: T,
	/// How well it matched: higher is better. Postgres ranks with
	/// `ts_rank` and SQLite with `bm25`, so ranks order rows alike on both
	/// but their values differ, and they only compare within one search.
	pub rank: f64,
	/// The highlights the search asked for, in the order it asked:
	/// [`Search::highlight`] and [`Search::excerpt`].
	pub highlights: Vec<Highlighted>,
}

/// Text with the words a search matched marked. The text is never HTML:
/// escape each segment as you render it.
///
/// ```ignore
/// for segment in hit.highlights[0].segments() {
/// 	if segment.matched { view! { <mark>{segment.text.clone()}</mark> } } else { … }
/// }
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Highlighted {
	segments: Vec<Segment>,
}

/// A run of highlighted text: matched words, or the text between them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Segment {
	/// The text.
	pub text: String,
	/// Whether the search matched it.
	pub matched: bool,
}

impl Highlighted {
	/// The text in runs, matched or not, in order. Neighbouring runs always
	/// differ in whether they matched.
	pub fn segments(&self) -> &[Segment] {
		&self.segments
	}

	/// The text with nothing marked.
	pub fn text(&self) -> String {
		self.segments
			.iter()
			.map(|segment| segment.text.as_str())
			.collect()
	}

	/// The matched runs.
	pub fn matches(&self) -> impl Iterator<Item = &str> {
		self.segments
			.iter()
			.filter(|segment| segment.matched)
			.map(|segment| segment.text.as_str())
	}

	/// Splits text the database marked: [`OPEN`] starts a match and
	/// [`CLOSE`] ends it.
	fn parse(marked: &str) -> Self {
		let mut highlighted = Self::default();
		let mut matched = false;
		let mut text = String::new();

		for character in marked.chars() {
			let starts = match character {
				OPEN => true,
				CLOSE => false,
				character => {
					text.push(character);
					continue;
				}
			};

			highlighted.push(std::mem::take(&mut text), matched);
			matched = starts;
		}

		highlighted.push(text, matched);
		highlighted
	}

	fn push(&mut self, text: String, matched: bool) {
		if text.is_empty() {
			return;
		}

		match self.segments.last_mut() {
			Some(last) if last.matched == matched => last.text.push_str(&text),
			_ => self.segments.push(Segment { text, matched }),
		}
	}
}

/// A parsed search: groups any of which may match, each a set of terms that
/// must all hold. Every group has a term that must match, so a search never
/// finds rows only by what they lack.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Terms {
	groups: Vec<Vec<Term>>,
}

/// A word or phrase a row must, or must not, contain.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Term {
	text: String,
	negated: bool,
	/// It matches any word it begins: one word of letters, digits, and `_`.
	prefix: bool,
}

impl Terms {
	/// Reads a web-search-style query: words, `"phrases"`, `-excluded`,
	/// `OR`, and `prefix*`. Terms without a letter or digit are dropped, and
	/// so are groups with nothing that must match.
	fn parse(text: &str) -> std::result::Result<Self, String> {
		let mut groups: Vec<Vec<Term>> = vec![Vec::new()];
		let mut chars = text.chars().peekable();

		while let Some(&next) = chars.peek() {
			if next.is_whitespace() {
				chars.next();
				continue;
			}

			let negated = chars.next_if_eq(&'-').is_some();
			let (text, quoted) = if chars.next_if_eq(&'"').is_some() {
				let phrase: String = chars.by_ref().take_while(|&inner| inner != '"').collect();
				(phrase, true)
			} else {
				let mut word = String::new();

				while let Some(inner) =
					chars.next_if(|&inner| !inner.is_whitespace() && inner != '"')
				{
					word.push(inner);
				}

				(word, false)
			};

			if !quoted && !negated && text == "OR" {
				groups.push(Vec::new());
				continue;
			}

			let starred = if quoted {
				chars.next_if_eq(&'*').is_some()
			} else {
				text.ends_with('*')
			};
			let text = if quoted {
				text
			} else {
				text.trim_end_matches('*').to_string()
			};

			if !text.chars().any(char::is_alphanumeric) {
				continue;
			}

			let prefix = starred
				&& text
					.chars()
					.all(|character| character.is_alphanumeric() || character == '_');
			groups
				.last_mut()
				.expect("there is always a group")
				.push(Term {
					text,
					negated,
					prefix,
				});
		}

		groups.retain(|group| group.iter().any(|term| !term.negated));
		let terms: usize = groups.iter().map(Vec::len).sum();

		if terms > MAX_TERMS {
			return Err(format!(
				"a search has at most {MAX_TERMS} words and phrases, and this one has {terms}"
			));
		}

		Ok(Self { groups })
	}

	/// The query as an FTS5 expression, every term a quoted string so that
	/// nothing in it is read as syntax: `("a" AND "b"*) NOT "c" OR "d"`.
	fn fts5(&self) -> String {
		let term = |term: &Term| {
			let mut quoted = format!("\"{}\"", term.text.replace('"', "\"\""));

			if term.prefix {
				quoted.push('*');
			}

			quoted
		};

		self.groups
			.iter()
			.map(|group| {
				let required: Vec<String> = group
					.iter()
					.filter(|term| !term.negated)
					.map(term)
					.collect();
				let mut expression = format!("({})", required.join(" AND "));

				for excluded in group.iter().filter(|term| term.negated) {
					expression.push_str(" NOT ");
					expression.push_str(&term(excluded));
				}

				format!("({expression})")
			})
			.collect::<Vec<_>>()
			.join(" OR ")
	}

	/// The query as a Postgres `tsquery`, every term a bound value:
	/// `(phraseto_tsquery(cfg, $1) && !!phraseto_tsquery(cfg, $2)) || …`.
	/// A prefix is `to_tsquery(cfg, 'word:*')`, safe because the word is
	/// only letters, digits, and `_`.
	fn write_tsquery(&self, sql: &mut Sql, language: &str) {
		sql.push("(");

		for (index, group) in self.groups.iter().enumerate() {
			if index > 0 {
				sql.push(" || ");
			}

			sql.push("(");

			for (position, term) in group.iter().enumerate() {
				if position > 0 {
					sql.push(" && ");
				}

				if term.negated {
					sql.push("!!");
				}

				if term.prefix {
					sql.push(&format!("to_tsquery({language}, "))
						.value(Value::Text(format!("{}:*", term.text)))
						.push(")");
				} else {
					sql.push(&format!("phraseto_tsquery({language}, "))
						.value(Value::Text(term.text.clone()))
						.push(")");
				}
			}

			sql.push(")");
		}

		sql.push(")");
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn term(text: &str, negated: bool, prefix: bool) -> Term {
		Term {
			text: text.to_string(),
			negated,
			prefix,
		}
	}

	#[test]
	fn queries_read_like_a_search_box() {
		let terms = Terms::parse(r#"rust "pass keys" -java OR go* -"c++" lang*uage"#).unwrap();
		assert_eq!(
			terms.groups,
			vec![
				vec![
					term("rust", false, false),
					term("pass keys", false, false),
					term("java", true, false),
				],
				vec![
					term("go", false, true),
					term("c++", true, false),
					term("lang*uage", false, false),
				],
			]
		);
		assert_eq!(
			terms.fts5(),
			r#"(("rust" AND "pass keys") NOT "java") OR (("go"* AND "lang*uage") NOT "c++")"#
		);
	}

	#[test]
	fn punctuation_and_exclusions_alone_find_nothing() {
		for text in [
			"",
			"   ",
			"-rust",
			"!!! ???",
			"OR",
			"- -",
			"\"\"",
			"-\"a b\" OR -c",
		] {
			assert_eq!(
				Terms::parse(text).unwrap().groups,
				Vec::<Vec<Term>>::new(),
				"{text:?}"
			);
		}

		// A group of exclusions is dropped; the other group stays.
		let terms = Terms::parse("-a OR b").unwrap();
		assert_eq!(terms.groups, vec![vec![term("b", false, false)]]);
	}

	#[test]
	fn quotes_and_stars_are_never_syntax() {
		// A quote ends a word and starts a phrase, which may run to the end.
		let terms = Terms::parse(r#"x"y z" "unterminated phrase"#).unwrap();
		assert_eq!(
			terms.groups[0],
			vec![
				term("x", false, false),
				term("y z", false, false),
				term("unterminated phrase", false, false),
			]
		);
		assert_eq!(
			terms.fts5(),
			r#"(("x" AND "y z" AND "unterminated phrase"))"#
		);

		// A phrase or punctuated word can't be a prefix.
		let terms = Terms::parse(r#""two words"* c-d*"#).unwrap();
		assert!(!terms.groups[0][0].prefix && !terms.groups[0][1].prefix);
		assert_eq!(terms.fts5(), r#"(("two words" AND "c-d"))"#);
	}

	#[test]
	fn searches_have_a_term_limit() {
		let words = vec!["word"; MAX_TERMS].join(" ");
		assert!(Terms::parse(&words).is_ok());
		let error = Terms::parse(&format!("{words} one")).unwrap_err();
		assert!(error.contains("at most 32"), "{error}");
	}

	#[test]
	fn highlights_split_into_runs() {
		let marked = format!("a {OPEN}rust{CLOSE} and {OPEN}go{CLOSE}{OPEN}lang{CLOSE}!");
		let highlighted = Highlighted::parse(&marked);
		assert_eq!(
			highlighted.segments(),
			&[
				Segment {
					text: "a ".into(),
					matched: false
				},
				Segment {
					text: "rust".into(),
					matched: true
				},
				Segment {
					text: " and ".into(),
					matched: false
				},
				Segment {
					text: "golang".into(),
					matched: true
				},
				Segment {
					text: "!".into(),
					matched: false
				},
			]
		);
		assert_eq!(highlighted.text(), "a rust and golang!");
		assert_eq!(
			highlighted.matches().collect::<Vec<_>>(),
			["rust", "golang"]
		);
		assert_eq!(Highlighted::parse(""), Highlighted::default());
		// An unclosed match runs to the end.
		assert_eq!(
			Highlighted::parse(&format!("x{OPEN}y"))
				.matches()
				.collect::<Vec<_>>(),
			["y"]
		);
	}
}
