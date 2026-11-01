//! The studio: a browser UI for the development database, which
//! `leptoskit db studio` serves from inside the app's db crate, so that it
//! reads every table through the app's own policies.
//!
//! It exists only in builds with debug assertions and the `studio` feature:
//! a release build compiles none of it. It listens on a loopback address
//! only, every request must carry the token generated for the run, and the
//! `Host` must be that address, so other machines, other sites in the
//! browser, and DNS rebinding can't reach it. Pages are leptos views
//! rendered on the server, with links and forms and no script.
//!
//! - **Viewing as someone.** Every read goes through the policy layer as the
//!   chosen viewer: nobody (an anonymous request), a registered
//!   [`Persona`](crate::Persona) with the identity you type, or `system`, which
//!   bypasses policies and needs a reason, as the audit log records.
//! - **Writing.** Edits and deletes only appear in edit mode, each shows what
//!   it will do and waits for a second, confirming submit, and each runs
//!   through the table's own `save` or `delete`: its hooks, rules, policies,
//!   and audit log.

mod ops;
mod pages;

use std::collections::HashMap;
use std::collections::VecDeque;
use std::fmt::Write as _;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::OnceLock;

use axum::Router;
use axum::extract::Form;
use axum::extract::Path;
use axum::extract::Query;
use axum::extract::Request;
use axum::extract::State;
use axum::http::HeaderValue;
use axum::http::StatusCode;
use axum::http::header;
use axum::middleware::Next;
use axum::response::Html;
use axum::response::IntoResponse;
use axum::response::Response;
use axum::routing::get;
use leptoskit_db_schema::TableName;
pub(crate) use ops::TableOps;

use crate::Context;
use crate::Db;
use crate::Read;
use crate::live::Invalidation;

/// What the page's title starts with, which a test looks for in binaries to
/// prove release builds leave the studio out.
pub(crate) const TITLE: &str = "leptoskit db studio";

/// How many rows a table page shows.
const PAGE: u64 = 50;

/// How many viewers' live changes the studio follows at once, and how many
/// changes it keeps for each.
const FEEDS: usize = 16;
const KEPT: usize = 200;

/// Where and what the studio serves.
#[derive(Clone, Debug)]
pub struct Options {
	/// The loopback address to listen on; port 0 picks a free one.
	pub address: SocketAddr,
	/// The app's migrations directory, for the migrations page.
	pub migrations: Option<PathBuf>,
}

/// A studio that is serving.
#[derive(Debug)]
pub struct Running {
	/// The address to open, with this run's token.
	pub url: String,
	task: tokio::task::JoinHandle<std::io::Result<()>>,
}

impl Running {
	/// Serves until the server fails.
	///
	/// # Errors
	///
	/// When the server stops on an I/O error.
	pub async fn wait(self) -> std::io::Result<()> {
		self.task.await.map_err(std::io::Error::other)?
	}

	/// Stops serving.
	pub fn stop(self) {
		self.task.abort();
	}
}

/// The studio's state, shared by every request.
pub(crate) struct Studio {
	db: Db,
	token: String,
	hosts: Vec<String>,
	migrations: Option<PathBuf>,
	feeds: Mutex<HashMap<String, Arc<Mutex<VecDeque<Change>>>>>,
}

/// One live change a viewer heard of.
#[derive(Clone, Debug)]
pub(crate) struct Change {
	pub(crate) at: crate::Timestamp,
	pub(crate) what: String,
}

/// Starts the studio on `options.address`, which must be a loopback
/// address, with a token made for this run.
///
/// # Errors
///
/// When the address isn't a loopback one, no random token can be made, or
/// the address can't be bound.
pub async fn start(db: Db, options: Options) -> std::io::Result<Running> {
	if !options.address.ip().is_loopback() {
		return Err(std::io::Error::new(
			std::io::ErrorKind::InvalidInput,
			format!(
				"the studio only listens on this machine; {} isn't a loopback address",
				options.address
			),
		));
	}

	let token = token()?;
	let listener = tokio::net::TcpListener::bind(options.address).await?;
	let address = listener.local_addr()?;
	let port = address.port();
	let hosts = vec![
		format!("127.0.0.1:{port}"),
		format!("localhost:{port}"),
		format!("[::1]:{port}"),
	];
	let url = format!("http://{address}/?token={token}");
	let studio = Arc::new(Studio {
		db,
		token,
		hosts,
		migrations: options.migrations,
		feeds: Mutex::new(HashMap::new()),
	});
	let router = router(studio);
	let task = tokio::spawn(async move { axum::serve(listener, router).await });
	Ok(Running { url, task })
}

/// Serves the studio as `leptoskit db studio` asks, through the
/// environment, until the process ends.
///
/// # Panics
///
/// When the environment is incomplete or the studio can't start: the panic
/// is the message `leptoskit db studio` shows.
pub(crate) fn serve_from_env() {
	let address = std::env::var(crate::STUDIO_ENV)
		.expect("set by `leptoskit db studio`")
		.parse::<SocketAddr>()
		.unwrap_or_else(|error| panic!("{} isn't an address: {error}", crate::STUDIO_ENV));
	let url = std::env::var(crate::STUDIO_DATABASE_ENV)
		.unwrap_or_else(|_| panic!("{} names no database", crate::STUDIO_DATABASE_ENV));
	let migrations = std::env::var_os(crate::STUDIO_MIGRATIONS_ENV).map(PathBuf::from);
	let row_security = std::env::var(crate::POLICIES_ENV).as_deref() == Ok("rls");
	let runtime = tokio::runtime::Builder::new_multi_thread()
		.enable_all()
		.build()
		.unwrap_or_else(|error| panic!("couldn't start the studio's runtime: {error}"));

	let served = runtime.block_on(async {
		let mut db = Db::connect(&url)
			.await
			.map_err(|error| format!("couldn't connect to the database: {}", chain(&error)))?;

		if row_security {
			db = db.with_row_security().map_err(|error| chain(&error))?;
		}

		let running = start(
			db,
			Options {
				address,
				migrations,
			},
		)
		.await
		.map_err(|error| format!("couldn't start the studio: {error}"))?;
		println!("{TITLE}: {}", running.url);
		running.wait().await.map_err(|error| chain(&error))
	});

	if let Err(error) = served {
		panic!("{error}");
	}
}

/// An error and each error that caused it, joined, since a database
/// error's own message is often general and its source says what happened.
fn chain(error: &(dyn std::error::Error + 'static)) -> String {
	let mut text = error.to_string();
	let mut source = error.source();

	while let Some(cause) = source {
		text.push_str(": ");
		text.push_str(&cause.to_string());
		source = cause.source();
	}

	text
}

/// 32 random bytes, in hex.
fn token() -> std::io::Result<String> {
	let mut bytes = [0u8; 32];
	getrandom::fill(&mut bytes).map_err(|error| std::io::Error::other(error.to_string()))?;
	Ok(bytes.iter().fold(String::new(), |mut hex, byte| {
		let _ = write!(hex, "{byte:02x}");
		hex
	}))
}

/// Whether two texts are equal, taking as long whatever they hold.
fn same(left: &str, right: &str) -> bool {
	left.len() == right.len()
		&& left
			.bytes()
			.zip(right.bytes())
			.fold(0u8, |difference, (left, right)| difference | (left ^ right))
			== 0
}

pub(crate) fn router(studio: Arc<Studio>) -> Router {
	Router::new()
		.route("/", get(tables))
		.route("/table/{name}", get(table))
		.route("/table/{name}/edit", get(edit_form).post(edit))
		.route("/table/{name}/delete", get(delete_form).post(delete))
		.route("/schema", get(schema))
		.route("/migrations", get(migrations))
		.route("/policies", get(policies))
		.route("/audit", get(audit))
		.route("/live", get(live))
		.layer(axum::middleware::from_fn_with_state(studio.clone(), guard))
		.with_state(studio)
}

/// Lets a request through only with this run's token, sent to this
/// machine's address, from no other site; and keeps the response out of
/// caches, frames, and referrers.
async fn guard(State(studio): State<Arc<Studio>>, request: Request, next: Next) -> Response {
	let headers = request.headers();
	let host = headers
		.get(header::HOST)
		.and_then(|host| host.to_str().ok())
		.unwrap_or_default();
	let origin_allowed = headers
		.get(header::ORIGIN)
		.and_then(|origin| origin.to_str().ok())
		.is_none_or(|origin| {
			studio
				.hosts
				.iter()
				.any(|host| origin == format!("http://{host}"))
		});
	let token = request
		.uri()
		.query()
		.and_then(|query| {
			query
				.split('&')
				.find_map(|pair| pair.strip_prefix("token="))
		})
		.unwrap_or_default();

	let mut response = if !studio.hosts.iter().any(|allowed| allowed == host) {
		(
			StatusCode::FORBIDDEN,
			"the studio only answers requests to its own address",
		)
			.into_response()
	} else if !origin_allowed {
		(
			StatusCode::FORBIDDEN,
			"the studio only answers its own pages",
		)
			.into_response()
	} else if !same(token, &studio.token) {
		(
			StatusCode::FORBIDDEN,
			"open the studio with the address it printed, which holds this run's token",
		)
			.into_response()
	} else {
		next.run(request).await
	};

	let headers = response.headers_mut();
	for (name, value) in [
		(header::CACHE_CONTROL, "no-store"),
		(header::REFERRER_POLICY, "no-referrer"),
		(header::X_FRAME_OPTIONS, "DENY"),
		(header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
		(
			header::CONTENT_SECURITY_POLICY,
			"default-src 'none'; style-src 'unsafe-inline'; form-action 'self'; frame-ancestors \
			 'none'; base-uri 'none'",
		),
	] {
		headers.insert(name, HeaderValue::from_static(value));
	}

	response
}

/// Who the pages view the data as, and whether they offer edits: carried
/// from page to page in each link and form.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Viewer {
	/// `anonymous`, `system`, or a persona's name.
	pub(crate) kind: String,
	/// The persona's identity, such as an account ID.
	pub(crate) who: String,
	/// Why `system` bypasses policies.
	pub(crate) reason: String,
	pub(crate) edit: bool,
}

impl Viewer {
	fn from_query(query: &HashMap<String, String>) -> Self {
		let text = |name: &str| query.get(name).cloned().unwrap_or_default();
		let kind = text("as");

		Self {
			kind: if kind.is_empty() {
				"anonymous".into()
			} else {
				kind
			},
			who: text("who"),
			reason: text("reason"),
			edit: query.get("edit").is_some_and(|edit| edit == "1"),
		}
	}

	/// The query parameters that carry it.
	pub(crate) fn params(&self) -> Vec<(&'static str, String)> {
		let mut params = vec![("as", self.kind.clone())];

		if !self.who.is_empty() {
			params.push(("who", self.who.clone()));
		}

		if !self.reason.is_empty() {
			params.push(("reason", self.reason.clone()));
		}

		if self.edit {
			params.push(("edit", "1".into()));
		}

		params
	}

	/// A short description, for headings and for telling feeds apart.
	pub(crate) fn describe(&self) -> String {
		match self.kind.as_str() {
			"anonymous" => "nobody: an anonymous request".into(),
			"system" => format!("system, because {}", self.reason),
			persona => format!("{persona} {}", self.who),
		}
	}

	/// The database handle that reads as this viewer, or why there is none.
	fn db(&self, base: &Db) -> Result<Db, String> {
		match self.kind.as_str() {
			"anonymous" => Ok(base.with_context(Context::new())),
			"system" => {
				let reason = self.reason.trim();

				if reason.is_empty() {
					return Err(
						"viewing as system bypasses every policy, so say why: type a reason".into(),
					);
				}

				Ok(base.system(intern(&format!("studio: {reason}"))))
			}
			name => {
				let persona = crate::persona::personas()
					.into_iter()
					.find(|persona| persona.name == name)
					.ok_or_else(|| format!("no persona is named `{name}`"))?;
				let context = (persona.context)(self.who.trim())?;
				Ok(base.with_context(context))
			}
		}
	}
}

/// A reason as the `'static` text [`Db::system`] records: each distinct one
/// is kept once for the life of the process.
fn intern(reason: &str) -> &'static str {
	static REASONS: OnceLock<Mutex<HashMap<String, &'static str>>> = OnceLock::new();
	let mut reasons = REASONS
		.get_or_init(Mutex::default)
		.lock()
		.unwrap_or_else(std::sync::PoisonError::into_inner);

	if let Some(reason) = reasons.get(reason) {
		return reason;
	}

	let leaked: &'static str = Box::leak(reason.to_string().into_boxed_str());
	reasons.insert(reason.to_string(), leaked);
	leaked
}

/// The registered table with this flat name, and its operations.
fn table_named(name: &str) -> Option<(TableName, TableOps)> {
	crate::table::entries()
		.iter()
		.find(|(table, _)| table.flat() == name)
		.map(|(table, entry)| (table.clone(), entry.studio))
}

/// Every registered table, by flat name.
fn table_names() -> Vec<TableName> {
	let mut names: Vec<TableName> = crate::table::entries().keys().cloned().collect();
	names.sort_by_key(TableName::flat);
	names
}

type Params = Query<HashMap<String, String>>;

/// A page, or the viewer's problem as one.
fn page(
	studio: &Studio,
	viewer: &Viewer,
	title: &str,
	body: Result<pages::Body, String>,
) -> Response {
	let html = pages::render(&pages::Frame {
		token: &studio.token,
		viewer,
		title,
		personas: crate::persona::personas(),
		body: body.unwrap_or_else(pages::Body::Problem),
	});
	Html(html).into_response()
}

async fn tables(State(studio): State<Arc<Studio>>, Query(query): Params) -> Response {
	let viewer = Viewer::from_query(&query);
	let body = async {
		let db = viewer.db(&studio.db)?;
		let mut counts = Vec::new();

		for name in table_names() {
			let (_, ops) = table_named(&name.flat()).expect("a registered table");
			let browse = ops::Browse {
				limit: 0,
				..ops::Browse::default()
			};
			let count = (ops.read)(&db, &browse)
				.await
				.map(|rows| rows.total)
				.map_err(|error| chain(&error));
			counts.push((name.flat(), count));
		}

		Ok(pages::Body::Tables(counts))
	}
	.await;
	page(&studio, &viewer, "Tables", body)
}

/// What a table page's query asks for.
fn browse_of(query: &HashMap<String, String>) -> ops::Browse {
	let text = |name: &str| query.get(name).map(String::as_str).unwrap_or_default();
	let filter = (!text("column").is_empty())
		.then(|| {
			ops::Op::parse(text("op")).map(|op| {
				ops::Filter {
					column: text("column").into(),
					op,
					value: text("value").into(),
				}
			})
		})
		.flatten();
	let page: u64 = text("page").parse().unwrap_or(0);

	ops::Browse {
		filter,
		order: (!text("order").is_empty()).then(|| (text("order").into(), text("desc") == "1")),
		limit: PAGE,
		offset: page.saturating_mul(PAGE),
		with_deleted: text("deleted") == "1",
	}
}

async fn table(
	State(studio): State<Arc<Studio>>,
	Path(name): Path<String>,
	Query(query): Params,
) -> Response {
	let viewer = Viewer::from_query(&query);
	let browse = browse_of(&query);
	let body = async {
		let db = viewer.db(&studio.db)?;
		let (_, ops) = table_named(&name).ok_or_else(|| format!("there's no table `{name}`"))?;
		let rows = (ops.read)(&db, &browse)
			.await
			.map_err(|error| chain(&error))?;
		Ok(pages::Body::Rows {
			table: name.clone(),
			browse,
			rows,
		})
	}
	.await;
	page(&studio, &viewer, &name, body)
}

/// The key a link or form names, as a JSON array of texts.
fn key_of(query: &HashMap<String, String>) -> Result<Vec<String>, String> {
	serde_json::from_str(query.get("key").map(String::as_str).unwrap_or_default())
		.map_err(|error| format!("the row's key is missing or malformed: {error}"))
}

/// Edits are offered only in edit mode, and the form must say so too.
fn editing(viewer: &Viewer) -> Result<(), String> {
	if viewer.edit {
		Ok(())
	} else {
		Err("turn on edit mode to change rows".into())
	}
}

async fn edit_form(
	State(studio): State<Arc<Studio>>,
	Path(name): Path<String>,
	Query(query): Params,
) -> Response {
	let viewer = Viewer::from_query(&query);
	let body = (|| {
		editing(&viewer)?;
		table_named(&name).ok_or_else(|| format!("there's no table `{name}`"))?;
		Ok(pages::Body::EditForm {
			table: name.clone(),
			key: key_of(&query)?,
			column: query.get("column").cloned().unwrap_or_default(),
			current: query.get("current").cloned().unwrap_or_default(),
		})
	})();
	page(&studio, &viewer, &name, body)
}

/// A submitted edit: the first submit asks for confirmation, and only a
/// second, confirming one writes.
async fn edit(
	State(studio): State<Arc<Studio>>,
	Path(name): Path<String>,
	Query(query): Params,
	Form(form): Form<HashMap<String, String>>,
) -> Response {
	let viewer = Viewer::from_query(&form);
	let body = async {
		same_viewer(&query, &viewer)?;
		editing(&viewer)?;
		let (_, ops) = table_named(&name).ok_or_else(|| format!("there's no table `{name}`"))?;
		let change = ops::Edit {
			key: key_of(&form)?,
			column: form.get("column").cloned().unwrap_or_default(),
			value: (form.get("null").map(String::as_str) != Some("1"))
				.then(|| form.get("value").cloned().unwrap_or_default()),
		};

		if form.get("confirmed").map(String::as_str) != Some("yes") {
			return Ok(pages::Body::ConfirmEdit {
				table: name.clone(),
				change,
			});
		}

		let db = viewer.db(&studio.db)?;
		let edit = ops.edit;
		let applied = change.clone();
		on_own_thread(move || edit(db, applied))
			.await
			.map_err(|error| format!("the change was refused: {}", chain(&error)))?;
		Ok(pages::Body::Done {
			table: name.clone(),
			message: format!(
				"Saved `{}` of the row keyed {}.",
				change.column,
				change.key.join(", ")
			),
		})
	}
	.await;
	page(&studio, &viewer, &name, body)
}

async fn delete_form(
	State(studio): State<Arc<Studio>>,
	Path(name): Path<String>,
	Query(query): Params,
) -> Response {
	let viewer = Viewer::from_query(&query);
	let body = (|| {
		editing(&viewer)?;
		table_named(&name).ok_or_else(|| format!("there's no table `{name}`"))?;
		Ok(pages::Body::ConfirmDelete {
			table: name.clone(),
			key: key_of(&query)?,
		})
	})();
	page(&studio, &viewer, &name, body)
}

async fn delete(
	State(studio): State<Arc<Studio>>,
	Path(name): Path<String>,
	Query(query): Params,
	Form(form): Form<HashMap<String, String>>,
) -> Response {
	let viewer = Viewer::from_query(&form);
	let body = async {
		same_viewer(&query, &viewer)?;
		editing(&viewer)?;
		let (_, ops) = table_named(&name).ok_or_else(|| format!("there's no table `{name}`"))?;
		let key = key_of(&form)?;

		if form.get("confirmed").map(String::as_str) != Some("yes") {
			return Ok(pages::Body::ConfirmDelete {
				table: name.clone(),
				key,
			});
		}

		let db = viewer.db(&studio.db)?;
		let remove = ops.remove;
		let removed = key.clone();
		on_own_thread(move || remove(db, removed))
			.await
			.map_err(|error| format!("the delete was refused: {}", chain(&error)))?;
		Ok(pages::Body::Done {
			table: name.clone(),
			message: format!("Deleted the row keyed {}.", key.join(", ")),
		})
	}
	.await;
	page(&studio, &viewer, &name, body)
}

/// A form's viewer must be the one its address names, so a page can't be
/// made to write as someone else than it shows.
fn same_viewer(query: &HashMap<String, String>, viewer: &Viewer) -> Result<(), String> {
	if Viewer::from_query(query) == *viewer {
		Ok(())
	} else {
		Err("the form's viewer doesn't match the page's; reload it".into())
	}
}

async fn schema(State(studio): State<Arc<Studio>>, Query(query): Params) -> Response {
	let viewer = Viewer::from_query(&query);
	let body = crate::schema(studio.db.dialect())
		.map(pages::Body::Schema)
		.map_err(|error| chain(&error));
	page(&studio, &viewer, "Schema", body)
}

async fn migrations(State(studio): State<Arc<Studio>>, Query(query): Params) -> Response {
	let viewer = Viewer::from_query(&query);
	let body = async {
		let Some(directory) = &studio.migrations else {
			return Err("the studio wasn't told where the migrations are".to_string());
		};
		let migrations = crate::load_migrations(directory).map_err(|error| chain(&error))?;
		let applied = studio
			.db
			.applied_migrations()
			.await
			.map_err(|error| chain(&error))?;
		let shown = query.get("show").cloned();

		Ok(pages::Body::Migrations(
			migrations
				.into_iter()
				.map(|migration| {
					let open = shown.as_deref() == Some(&*migration.name);
					pages::MigrationRow {
						applied: applied.iter().any(|name| *name == migration.name),
						sql: open.then(|| migration.sql.to_string()),
						name: migration.name.to_string(),
					}
				})
				.collect(),
		))
	}
	.await;
	page(&studio, &viewer, "Migrations", body)
}

async fn policies(State(studio): State<Arc<Studio>>, Query(query): Params) -> Response {
	let viewer = Viewer::from_query(&query);
	let body = (|| {
		let db = viewer.db(&studio.db)?;
		let access = db.access().clone();
		let mut tables = Vec::new();

		for name in table_names() {
			let (_, ops) = table_named(&name.flat()).expect("a registered table");
			let rules = (ops.rules)(&access, studio.db.dialect()).map_err(|error| chain(&error));
			tables.push((name.flat(), rules));
		}

		Ok(pages::Body::Policies(tables))
	})();
	page(&studio, &viewer, "Policies", body)
}

async fn audit(State(studio): State<Arc<Studio>>, Query(query): Params) -> Response {
	let viewer = Viewer::from_query(&query);
	let page_number: u64 = query
		.get("page")
		.and_then(|page| page.parse().ok())
		.unwrap_or(0);
	let body = async {
		let db = viewer.db(&studio.db)?;
		let entries = crate::AuditEntry::query()
			.order_by(|entry| entry.id.desc())
			.limit(PAGE)
			.offset(page_number.saturating_mul(PAGE))
			.all(&db)
			.await
			.map_err(|error| format!("couldn't read the audit log: {}", chain(&error)))?;
		Ok(pages::Body::Audit {
			entries,
			page: page_number,
		})
	}
	.await;
	page(&studio, &viewer, "Audit log", body)
}

/// The changes this viewer has heard of since the studio started following
/// them, newest first; following them starts with the first visit.
async fn live(State(studio): State<Arc<Studio>>, Query(query): Params) -> Response {
	let viewer = Viewer::from_query(&query);
	let body = async {
		let db = viewer.db(&studio.db)?;
		let changes = follow(&studio, &viewer, db).await?;
		let changes = changes
			.lock()
			.unwrap_or_else(std::sync::PoisonError::into_inner)
			.iter()
			.rev()
			.cloned()
			.collect();
		Ok(pages::Body::Live(changes))
	}
	.await;
	page(&studio, &viewer, "Live changes", body)
}

/// The changes `viewer` hears of, following them from now if nothing does
/// yet.
async fn follow(
	studio: &Studio,
	viewer: &Viewer,
	db: Db,
) -> Result<Arc<Mutex<VecDeque<Change>>>, String> {
	let key = viewer.describe();

	if let Some(changes) = studio
		.feeds
		.lock()
		.unwrap_or_else(std::sync::PoisonError::into_inner)
		.get(&key)
	{
		return Ok(changes.clone());
	}

	let mut feed = db
		.changes()
		.await
		.map_err(|error| format!("couldn't follow live changes: {}", chain(&error)))?;
	let changes = Arc::new(Mutex::new(VecDeque::new()));
	let mut feeds = studio
		.feeds
		.lock()
		.unwrap_or_else(std::sync::PoisonError::into_inner);

	if feeds.len() >= FEEDS {
		return Err(format!(
			"the studio follows live changes for {FEEDS} viewers at most; restart it to follow \
			 another"
		));
	}

	feeds.insert(key, changes.clone());
	let kept = changes.clone();
	tokio::spawn(async move {
		loop {
			let what = match feed.next().await {
				Ok(invalidation) => summary(&invalidation),
				Err(error) => format!("the feed stopped: {}", chain(&error)),
			};
			let stopped = what.starts_with("the feed stopped");
			let mut kept = kept
				.lock()
				.unwrap_or_else(std::sync::PoisonError::into_inner);

			if kept.len() == KEPT {
				kept.pop_front();
			}

			kept.push_back(Change {
				at: crate::now(),
				what,
			});

			if stopped {
				break;
			}
		}
	});
	Ok(changes)
}

/// What a live change says, in a line.
fn summary(invalidation: &Invalidation) -> String {
	match invalidation {
		Invalidation::Everything => "everything: changes may have been missed".into(),
		Invalidation::Rows(tables) => {
			tables
				.iter()
				.map(|rows| {
					let keys: Vec<String> = rows.keys.iter().map(|key| key.join(", ")).collect();
					format!("{}: {}", rows.table, keys.join("; "))
				})
				.collect::<Vec<_>>()
				.join(" · ")
		}
	}
}

/// Runs a write that holds a row across awaits on a blocking thread, which
/// waits for it: a row needn't be `Send`, but a handler's future must be.
async fn on_own_thread(
	write: impl FnOnce() -> ops::Local<()> + Send + 'static,
) -> crate::Result<()> {
	let handle = tokio::runtime::Handle::current();
	tokio::task::spawn_blocking(move || handle.block_on(write()))
		.await
		.map_err(|error| crate::expr::invalid(&format!("the write didn't finish: {error}")))?
}

#[cfg(test)]
mod tests;
