//! The studio over real HTTP on a loopback address: who may reach it, what
//! each viewer reads through the policies, and edits that need edit mode
//! and a confirmation.

#![cfg(all(
	feature = "studio",
	feature = "sqlite",
	feature = "postgres",
	debug_assertions
))]

mod support;

use std::net::SocketAddr;
use std::time::Duration;

use leptoskit_db::Context;
use leptoskit_db::Db;
use leptoskit_db::Id;
use leptoskit_db::Persona;
use leptoskit_db::Timestamp;
use leptoskit_db::policy;
use leptoskit_db::studio::Options;
use leptoskit_db::studio::Running;
use leptoskit_db::table;
use leptoskit_db_schema::RequireHints;
use leptoskit_db_schema::Schema;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;

/// The person a request reads as.
#[derive(Clone, Copy)]
struct Viewer(Id<Writer>);

leptoskit_db::register_persona!(Persona {
	name: "writer",
	describe: "a writer, by ID",
	context: |id| {
		let id = id
			.parse()
			.map_err(|error| format!("not a writer's ID: {error}"))?;
		Ok(Context::new().with(Viewer(id)))
	},
});

/// Someone who writes notes.
#[table(name = "writers")]
pub struct Writer {
	pub id: Id<Self>,
	pub name: String,
}

/// A note: anyone reads a public one, its writer reads and changes their
/// own.
#[table(name = "notes", audit, live)]
#[policy(select, |note, cx| note.public.eq(true) | cx.global(|viewer: &Viewer| note.writer_id.eq(viewer.0)))]
#[policy(insert, update, delete, |note, cx| cx.global(|viewer: &Viewer| note.writer_id.eq(viewer.0)),
	message = "Only its writer changes a note")]
pub struct Note {
	pub id: Id<Self>,
	pub writer_id: Id<Writer>,
	pub title: String,
	pub public: bool,
	#[db(secret)]
	pub token: String,
	#[db(deleted_at)]
	pub deleted_at: Option<Timestamp>,
}

struct Fixture {
	db: Db,
	running: Running,
	address: SocketAddr,
	token: String,
	ada: Id<Writer>,
	grace: Id<Writer>,
	_migrations: tempfile::TempDir,
}

/// Runs `check` against a studio on SQLite, then on Postgres when it's
/// installed.
async fn on_both<F, Fut>(check: F)
where
	F: Fn(Fixture) -> Fut,
	Fut: Future<Output = ()>,
{
	check(fixture(Db::connect("sqlite::memory:").await.unwrap()).await).await;

	if let Some(cluster) = support::postgres::PostgresCluster::start() {
		check(fixture(Db::connect(&cluster.database()).await.unwrap()).await).await;
	}
}

async fn fixture(db: Db) -> Fixture {
	let dialect = db.dialect();
	let schema = leptoskit_db::schema(dialect).unwrap();
	let empty = Schema::empty(dialect);
	let plan = leptoskit_db_schema::diff(&empty, &schema, &mut RequireHints).unwrap();
	let sql = leptoskit_db_schema::render(&plan, &empty, &schema).sql();
	db.run_script(&sql).await.unwrap();

	// One migration, written but never applied, for the migrations page.
	let migrations = tempfile::tempdir().unwrap();
	std::fs::write(migrations.path().join("registry.txt"), "0001_init\n").unwrap();
	std::fs::create_dir(migrations.path().join("0001_init")).unwrap();
	std::fs::write(
		migrations.path().join("0001_init").join("migration.sql"),
		"CREATE TABLE marker (id integer);",
	)
	.unwrap();

	let system = db.system("a studio test");
	let ada = NewWriter { name: "ada".into() }
		.insert(&system)
		.await
		.unwrap()
		.id;
	let grace = NewWriter {
		name: "grace".into(),
	}
	.insert(&system)
	.await
	.unwrap()
	.id;

	for (writer, title, public) in [
		(ada, "ada's public note", true),
		(ada, "ada's private note", false),
		(grace, "grace's private note", false),
	] {
		NewNote {
			writer_id: writer,
			title: title.into(),
			public,
			token: "hunter2".into(),
		}
		.insert(&db.with_context(Context::new().with(Viewer(writer))))
		.await
		.unwrap();
	}

	let running = leptoskit_db::studio::start(
		db.clone(),
		Options {
			address: "127.0.0.1:0".parse().unwrap(),
			migrations: Some(migrations.path().to_path_buf()),
		},
	)
	.await
	.unwrap();
	let (address, token) = running
		.url
		.strip_prefix("http://")
		.and_then(|rest| rest.split_once("/?token="))
		.unwrap();
	let address = address.parse().unwrap();
	let token = token.to_string();

	Fixture {
		db,
		running,
		address,
		token,
		ada,
		grace,
		_migrations: migrations,
	}
}

/// A response: its status, headers, and body.
struct Reply {
	status: u16,
	head: String,
	body: String,
}

/// Sends one HTTP/1.1 request and reads the whole reply.
async fn send(address: SocketAddr, request: &str) -> Reply {
	let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
	stream.write_all(request.as_bytes()).await.unwrap();
	let mut reply = String::new();
	tokio::time::timeout(Duration::from_secs(10), stream.read_to_string(&mut reply))
		.await
		.expect("the studio answers")
		.unwrap();
	let (head, body) = reply.split_once("\r\n\r\n").unwrap();
	let status = head
		.split(' ')
		.nth(1)
		.and_then(|status| status.parse().ok())
		.unwrap();
	Reply {
		status,
		head: head.to_lowercase(),
		body: body.to_string(),
	}
}

fn encode(text: &str) -> String {
	text.bytes()
		.map(|byte| {
			if byte.is_ascii_alphanumeric() {
				char::from(byte).to_string()
			} else {
				format!("%{byte:02X}")
			}
		})
		.collect()
}

fn query(pairs: &[(&str, &str)]) -> String {
	pairs
		.iter()
		.map(|(name, value)| format!("{name}={}", encode(value)))
		.collect::<Vec<_>>()
		.join("&")
}

impl Fixture {
	fn host(&self) -> String {
		format!("127.0.0.1:{}", self.address.port())
	}

	/// `GET path` with this run's token and `pairs`.
	async fn get(&self, path: &str, pairs: &[(&str, &str)]) -> Reply {
		let mut all = vec![("token", self.token.as_str())];
		all.extend_from_slice(pairs);
		send(
			self.address,
			&format!(
				"GET {path}?{} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
				query(&all),
				self.host()
			),
		)
		.await
	}

	/// `POST path`, the viewer in the address and the form both, as the
	/// studio's own forms send it.
	async fn post(&self, path: &str, viewer: &[(&str, &str)], form: &[(&str, &str)]) -> Reply {
		let mut address = vec![("token", self.token.as_str())];
		address.extend_from_slice(viewer);
		let mut fields = viewer.to_vec();
		fields.extend_from_slice(form);
		let body = query(&fields);
		send(
			self.address,
			&format!(
				"POST {path}?{} HTTP/1.1\r\nHost: {}\r\nOrigin: http://{}\r\nContent-Type: \
				 application/x-www-form-urlencoded\r\nContent-Length: {}\r\nConnection: \
				 close\r\n\r\n{body}",
				query(&address),
				self.host(),
				self.host(),
				body.len()
			),
		)
		.await
	}

	async fn note(&self, title: &str) -> Note {
		Note::query()
			.with_deleted()
			.filter(|note| note.title.eq(title.to_string()))
			.one(&self.db.system("a studio test"))
			.await
			.unwrap()
	}
}

#[tokio::test]
async fn the_studio_listens_on_this_machine_only() {
	let db = Db::connect("sqlite::memory:").await.unwrap();

	for address in ["0.0.0.0:0", "192.168.1.10:0"] {
		let error = leptoskit_db::studio::start(
			db.clone(),
			Options {
				address: address.parse().unwrap(),
				migrations: None,
			},
		)
		.await
		.unwrap_err();
		assert!(error.to_string().contains("loopback"), "{error}");
	}
}

#[tokio::test]
async fn every_request_needs_this_runs_token_and_the_studios_own_address() {
	on_both(async |studio| {
		assert!(studio.running.url.starts_with("http://127.0.0.1:"));
		assert_eq!(studio.token.len(), 64);

		let host = studio.host();
		let request = |path: &str, host: &str, extra: &str| {
			format!("GET {path} HTTP/1.1\r\nHost: {host}\r\n{extra}Connection: close\r\n\r\n")
		};
		let refused = [
			request("/", &host, ""),
			request("/?token=wrong", &host, ""),
			request(&format!("/?token={}0", studio.token), &host, ""),
			// DNS rebinding: a page on another name that resolves here.
			request(
				&format!("/?token={}", studio.token),
				"rebound.example:80",
				"",
			),
			request(
				&format!("/?token={}", studio.token),
				&host,
				"Origin: http://evil.example\r\n",
			),
		];

		for request in refused {
			let reply = send(studio.address, &request).await;
			assert_eq!(reply.status, 403, "{request}");
			assert!(!reply.body.contains("notes"), "{request}");
		}

		let reply = studio.get("/", &[]).await;
		assert_eq!(reply.status, 200);
		for header in [
			"cache-control: no-store",
			"referrer-policy: no-referrer",
			"x-frame-options: deny",
			"content-security-policy: default-src 'none'",
		] {
			assert!(reply.head.contains(header), "{header} in {}", reply.head);
		}
		assert!(reply.body.starts_with("<!DOCTYPE html><html lang=\"en\">"));
		assert!(
			reply
				.body
				.contains("<title>Tables · leptoskit db studio</title>")
		);
		studio.running.stop();
	})
	.await;
}

#[tokio::test]
async fn each_viewer_reads_what_its_policies_allow() {
	on_both(async |studio| {
		let ada = studio.ada.to_string();

		// Nobody sees the public note only, and counts agree.
		let anonymous = studio.get("/table/notes", &[]).await;
		assert!(
			anonymous
				.body
				.contains("<span class=\"\">ada's public note</span>")
		);
		assert!(!anonymous.body.contains("private note"));
		assert!(anonymous.body.contains("Rows 1–1 of 1"));
		let tables = studio.get("/", &[]).await;
		assert!(
			tables.body.contains("notes</a></td><td>1</td>"),
			"{}",
			tables.body
		);

		// Ada sees her own too.
		let as_ada = studio
			.get("/table/notes", &[("as", "writer"), ("who", &ada)])
			.await;
		assert!(as_ada.body.contains("private note"));
		assert!(as_ada.body.contains("Rows 1–2 of 2"));
		assert!(as_ada.body.contains(&format!("Reading as writer {ada}.")));

		// A persona needs an identity it understands.
		let nobody = studio
			.get("/table/notes", &[("as", "writer"), ("who", "ada")])
			.await;
		assert!(nobody.body.contains("not a writer's ID"));
		assert!(!nobody.body.contains("public note"));
		let unknown = studio.get("/", &[("as", "admin")]).await;
		assert!(unknown.body.contains("no persona is named `admin`"));

		// System sees everything, only with a reason, and never a secret.
		let no_reason = studio.get("/table/notes", &[("as", "system")]).await;
		assert!(no_reason.body.contains("say why"));
		assert!(!no_reason.body.contains("note</span>"));
		let system = studio
			.get(
				"/table/notes",
				&[("as", "system"), ("reason", "debugging a ticket")],
			)
			.await;
		assert!(system.body.contains("Rows 1–3 of 3"));
		assert!(system.body.contains("‹secret›"));
		assert!(!system.body.contains("hunter2"));

		// What each viewer may do, as SQL.
		let policies = studio
			.get("/policies", &[("as", "writer"), ("who", &ada)])
			.await;
		assert!(policies.body.contains("<h2>notes</h2>"));
		let (first, second) = match studio.db.dialect() {
			leptoskit_db::Dialect::Postgres => ("$1", "$2"),
			leptoskit_db::Dialect::Sqlite => ("?1", "?2"),
		};

		assert!(policies.body.contains(&format!(
			"SELECT * FROM \"notes\" WHERE (\"public\" = {first} OR \"writer_id\" = {second})\n-- \
			 with [Bool(true), Uuid("
		)));
		// Without a viewer, the rule about the writer matches nothing.
		let anonymous = studio.get("/policies", &[]).await;
		assert!(
			anonymous
				.body
				.contains(&format!("WHERE (\"public\" = {first} OR 1 = 0)"))
		);
		assert!(policies.body.contains("every row: no policy restricts it"));
		studio.running.stop();
	})
	.await;
}

#[tokio::test]
async fn tables_page_filter_and_sort() {
	on_both(async |studio| {
		let system = studio.db.system("a studio test");

		for index in 0..60 {
			NewNote {
				writer_id: studio.grace,
				title: format!("bulk {index:02}"),
				public: true,
				token: "t".into(),
			}
			.insert(&system)
			.await
			.unwrap();
		}

		let first = studio.get("/table/notes", &[]).await;
		assert!(first.body.contains("Rows 1–50 of 61"));
		assert!(first.body.contains("Next →"));
		let second = studio.get("/table/notes", &[("page", "1")]).await;
		assert!(second.body.contains("Rows 51–61 of 61"));
		assert!(second.body.contains("← Previous"));

		let filtered = studio
			.get(
				"/table/notes",
				&[("column", "title"), ("op", "contains"), ("value", "bulk 5")],
			)
			.await;
		assert!(filtered.body.contains("Rows 1–10 of 10"));
		let sorted = studio
			.get(
				"/table/notes",
				&[
					("column", "title"),
					("op", "contains"),
					("value", "bulk 5"),
					("order", "title"),
					("desc", "1"),
				],
			)
			.await;
		let first_title = sorted.body.find("bulk 59").unwrap();
		let last_title = sorted.body.find("bulk 50").unwrap();
		assert!(first_title < last_title);

		let wrong = studio
			.get(
				"/table/notes",
				&[("column", "public"), ("op", "eq"), ("value", "maybe")],
			)
			.await;
		assert!(wrong.body.contains("`maybe` isn't `true` or `false`"));
		let missing = studio.get("/table/nothing", &[]).await;
		assert!(missing.body.contains("there's no table `nothing`"));
		studio.running.stop();
	})
	.await;
}

#[tokio::test]
async fn writes_need_edit_mode_a_confirmation_and_the_policies_consent() {
	on_both(async |studio| {
		let ada = studio.ada.to_string();
		let note = studio.note("ada's private note").await;
		let key = serde_json::to_string(&[note.id.to_string()]).unwrap();
		let as_ada = [("as", "writer"), ("who", ada.as_str())];
		let editing_as_ada = [("as", "writer"), ("who", ada.as_str()), ("edit", "1")];

		// Without edit mode there are no edit links, and a write is refused.
		let page = studio.get("/table/notes", &as_ada).await;
		assert!(!page.body.contains(">edit</a>"));
		let refused = studio
			.post(
				"/table/notes/edit",
				&as_ada,
				&[
					("key", &key),
					("column", "title"),
					("value", "renamed"),
					("confirmed", "yes"),
				],
			)
			.await;
		assert!(refused.body.contains("turn on edit mode"));
		assert_eq!(
			studio.note("ada's private note").await.title,
			"ada's private note"
		);

		// In edit mode, the first submit only asks.
		let page = studio.get("/table/notes", &editing_as_ada).await;
		assert!(page.body.contains(">edit</a>"));
		let form = studio
			.get(
				"/table/notes/edit",
				&[
					("as", "writer"),
					("who", &ada),
					("edit", "1"),
					("key", &key),
					("column", "title"),
					("current", "ada's private note"),
				],
			)
			.await;
		assert!(form.body.contains("Review the change"));
		let asked = studio
			.post(
				"/table/notes/edit",
				&editing_as_ada,
				&[("key", &key), ("column", "title"), ("value", "renamed")],
			)
			.await;
		assert!(asked.body.contains("Confirm the change"));
		assert_eq!(
			studio.note("ada's private note").await.title,
			"ada's private note"
		);

		// A form can't write as someone other than its page shows.
		let mismatched = studio
			.post(
				"/table/notes/edit",
				&editing_as_ada,
				&[
					("as", "system"),
					("reason", "sneaky"),
					("key", &key),
					("column", "title"),
					("value", "x"),
					("confirmed", "yes"),
				],
			)
			.await;
		assert!(
			mismatched
				.body
				.contains("the form's viewer doesn't match the page's")
		);

		let confirmed = studio
			.post(
				"/table/notes/edit",
				&editing_as_ada,
				&[
					("key", &key),
					("column", "title"),
					("value", "renamed"),
					("confirmed", "yes"),
				],
			)
			.await;
		assert!(
			confirmed.body.contains("Saved `title`"),
			"{}",
			confirmed.body
		);
		assert_eq!(studio.note("renamed").await.id, note.id);

		// Grace's policies refuse her changing Ada's note, with the rule's
		// message: grace can't see it, so it isn't found.
		let grace = studio.grace.to_string();
		let as_grace = [("as", "writer"), ("who", grace.as_str()), ("edit", "1")];
		let public = studio.note("ada's public note").await;
		let public_key = serde_json::to_string(&[public.id.to_string()]).unwrap();
		let forbidden = studio
			.post(
				"/table/notes/edit",
				&as_grace,
				&[
					("key", &public_key),
					("column", "title"),
					("value", "defaced"),
					("confirmed", "yes"),
				],
			)
			.await;
		assert!(
			forbidden.body.contains("Only its writer changes a note"),
			"{}",
			forbidden.body
		);
		assert_eq!(
			studio.note("ada's public note").await.title,
			"ada's public note"
		);

		// Keys and computed columns don't change, and values must fit.
		for (column, value, expected) in [
			("id", "x", "part of the key"),
			("public", "maybe", "isn"),
			("nothing", "x", "has no column"),
		] {
			let reply = studio
				.post(
					"/table/notes/edit",
					&editing_as_ada,
					&[
						("key", &key),
						("column", column),
						("value", value),
						("confirmed", "yes"),
					],
				)
				.await;
			assert!(reply.body.contains(expected), "{column}: {}", reply.body);
		}

		studio.running.stop();
	})
	.await;
}

#[tokio::test]
async fn deletes_need_edit_mode_and_a_confirmation_too() {
	on_both(async |studio| {
		let ada = studio.ada.to_string();
		let note = studio.note("ada's private note").await;
		let key = serde_json::to_string(&[note.id.to_string()]).unwrap();
		let editing_as_ada = [("as", "writer"), ("who", ada.as_str()), ("edit", "1")];
		let refused = studio
			.post(
				"/table/notes/delete",
				&[("as", "writer"), ("who", ada.as_str())],
				&[("key", &key), ("confirmed", "yes")],
			)
			.await;
		assert!(refused.body.contains("turn on edit mode"));

		// Deleting asks too, then soft-deletes through the table's delete.
		let asked = studio
			.get(
				"/table/notes/delete",
				&[
					("as", "writer"),
					("who", &ada),
					("edit", "1"),
					("key", &key),
				],
			)
			.await;
		assert!(asked.body.contains("Confirm the delete"));
		assert!(studio.note("ada's private note").await.deleted_at.is_none());
		let unconfirmed = studio
			.post("/table/notes/delete", &editing_as_ada, &[("key", &key)])
			.await;
		assert!(unconfirmed.body.contains("Confirm the delete"));
		let deleted = studio
			.post(
				"/table/notes/delete",
				&editing_as_ada,
				&[("key", &key), ("confirmed", "yes")],
			)
			.await;
		assert!(deleted.body.contains("Deleted the row"), "{}", deleted.body);
		assert!(studio.note("ada's private note").await.deleted_at.is_some());

		studio.running.stop();
	})
	.await;
}

#[tokio::test]
async fn system_writes_record_why_in_the_audit_log() {
	on_both(async |studio| {
		let ada = studio.ada.to_string();
		let public = studio.note("ada's public note").await;
		let public_key = serde_json::to_string(&[public.id.to_string()]).unwrap();
		let as_ada = [("as", "writer"), ("who", ada.as_str())];
		// System writes record why.
		let system = [("as", "system"), ("reason", "fixing a typo"), ("edit", "1")];
		let public = studio
			.post(
				"/table/notes/edit",
				&system,
				&[
					("key", &public_key),
					("column", "title"),
					("value", "fixed"),
					("confirmed", "yes"),
				],
			)
			.await;
		assert!(public.body.contains("Saved"));
		let audit = studio
			.get("/audit", &[("as", "system"), ("reason", "reading the log")])
			.await;
		assert!(
			audit.body.contains("studio: fixing a typo"),
			"{}",
			audit.body
		);
		// Without an audit reader, a viewer reads none of the log.
		let hidden = studio.get("/audit", &as_ada).await;
		assert!(!hidden.body.contains("studio: fixing a typo"));
		studio.running.stop();
	})
	.await;
}

#[tokio::test]
async fn what_rows_hold_is_shown_as_text() {
	on_both(async |studio| {
		NewNote {
			writer_id: studio.ada,
			title: "<script>alert(1)</script>".into(),
			public: true,
			token: "t".into(),
		}
		.insert(&studio.db.system("a studio test"))
		.await
		.unwrap();

		let page = studio.get("/table/notes", &[]).await;
		assert!(page.body.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
		assert!(!page.body.contains("<script"));
		studio.running.stop();
	})
	.await;
}

#[tokio::test]
async fn schema_migrations_and_live_changes() {
	on_both(async |studio| {
		let schema = studio.get("/schema", &[]).await;
		assert!(schema.body.contains("<h2>notes</h2>"));
		assert!(schema.body.contains("writer_id → writers (id)"));

		let migrations = studio.get("/migrations", &[]).await;
		assert!(migrations.body.contains("0001_init"));
		assert!(migrations.body.contains("pending"));
		assert!(!migrations.body.contains("CREATE TABLE marker"));
		let shown = studio.get("/migrations", &[("show", "0001_init")]).await;
		assert!(shown.body.contains("CREATE TABLE marker"));

		// The first visit starts following; later writes appear.
		let live = studio.get("/live", &[]).await;
		assert!(live.body.contains("http-equiv=\"refresh\""));
		let note = NewNote {
			writer_id: studio.ada,
			title: "fresh".into(),
			public: true,
			token: "t".into(),
		}
		.insert(&studio.db.system("a studio test"))
		.await
		.unwrap();

		let mut heard = false;

		for _ in 0..50 {
			if studio
				.get("/live", &[])
				.await
				.body
				.contains(&format!("notes: {}", note.id))
			{
				heard = true;
				break;
			}

			tokio::time::sleep(Duration::from_millis(100)).await;
		}

		assert!(heard, "the live page shows the insert");
		studio.running.stop();
	})
	.await;
}
