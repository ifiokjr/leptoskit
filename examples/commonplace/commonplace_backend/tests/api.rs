//! The API as the native apps and the extension call it: signed in with a
//! bearer token from an emailed code, over the real auth layer and an
//! in-memory SQLite database with Commonplace's migrations applied.

use commonplace_db::PublicClip;
use commonplace_db::PublicNote;
use commonplace_db::PublicProject;
use commonplace_shared::ClipChanges;
use commonplace_shared::ClipOnPage;
use commonplace_shared::NewClipInput;
use commonplace_shared::NewProjectInput;
use commonplace_shared::NoteInput;
use commonplace_shared::PageInput;
use commonplace_shared::ProjectDetail;
use commonplace_shared::ProjectSummary;
use commonplace_shared::TextQuote;
use http_body_util::BodyExt;
use leptoskit::auth::Auth;
use leptoskit::auth::Keys;
use leptoskit::auth::Mailbox;
use leptoskit::server::axum::Router;
use leptoskit::server::axum::body::Body;
use leptoskit::server::axum::http::Request;
use leptoskit::server::axum::http::StatusCode;
use leptoskit::server::axum::http::header;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use serde_json::json;
use tower::ServiceExt;

struct Server {
	router: Router,
	mailbox: Mailbox,
}

async fn server() -> Server {
	let db = leptoskit::db::testing::sqlite_migrated(commonplace_db::MIGRATIONS).await;
	let mailbox = Mailbox::new();
	let auth = Auth::builder(&db, Keys::single("test", [9; 32]))
		.mailer(mailbox.clone())
		.origin("https://commonplace.example")
		.build()
		.unwrap();
	let router = commonplace_backend::router()
		.merge(auth.router())
		.layer(auth.layer());
	Server { router, mailbox }
}

impl Server {
	async fn send(
		&self,
		method: &str,
		path: &str,
		token: Option<&str>,
		body: Option<Value>,
	) -> (StatusCode, Vec<u8>) {
		let mut request = Request::builder().method(method).uri(path);
		if let Some(token) = token {
			request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
		}
		if body.is_some() {
			request = request.header(header::CONTENT_TYPE, "application/json");
		}
		let request = request
			.body(body.map_or_else(Body::empty, |body| Body::from(body.to_string())))
			.unwrap();
		let response = self.router.clone().oneshot(request).await.unwrap();
		let status = response.status();
		let bytes = response.into_body().collect().await.unwrap().to_bytes();
		(status, bytes.to_vec())
	}

	async fn json<T: DeserializeOwned>(
		&self,
		method: &str,
		path: &str,
		token: &str,
		body: Option<impl Serialize>,
	) -> T {
		let body = body.map(|body| serde_json::to_value(body).unwrap());
		let (status, bytes) = self.send(method, path, Some(token), body).await;
		assert!(
			status.is_success(),
			"{method} {path}: {status} {}",
			String::from_utf8_lossy(&bytes)
		);
		serde_json::from_slice(&bytes).unwrap()
	}

	/// Signs in the way the apps do, returning the bearer token.
	async fn sign_in(&self, email: &str) -> String {
		let (status, _) = self
			.send(
				"POST",
				"/auth/email/send",
				None,
				Some(json!({ "email": email })),
			)
			.await;
		assert!(status.is_success());
		let mail = self.mailbox.latest_to(email).unwrap();
		let code = mail.subject.rsplit(' ').next().unwrap().to_owned();
		let (status, bytes) = self
			.send(
				"POST",
				"/auth/email/verify",
				None,
				Some(json!({ "email": email, "code": code, "transport": "bearer" })),
			)
			.await;
		assert_eq!(status, StatusCode::OK);
		let answer: Value = serde_json::from_slice(&bytes).unwrap();
		answer["token"].as_str().unwrap().to_owned()
	}
}

fn clip(project: &PublicProject, url: &str, quote: &str) -> NewClipInput {
	NewClipInput {
		project: Some(project.id),
		page: PageInput {
			url: url.into(),
			title: "Why cities run hot".into(),
			author: Some("Ada Lovelace".into()),
			site: Some("The Climate Desk".into()),
			published: Some("2024-05-01".into()),
		},
		quote: TextQuote {
			exact: quote.into(),
			prefix: Some("In cities, ".into()),
			suffix: None,
		},
		note: Some("  key claim ".into()),
		tags: "Heat, method".into(),
	}
}

#[tokio::test]
async fn signed_out_requests_are_refused() {
	let server = server().await;
	let (status, _) = server.send("GET", "/api/projects", None, None).await;
	assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_research_session() {
	let server = server().await;
	let ada = server.sign_in("ada@example.com").await;

	let project: PublicProject = server
		.json(
			"POST",
			"/api/projects",
			&ada,
			Some(NewProjectInput {
				title: " Urban heat islands ".into(),
				question: Some("Why are cities hotter?".into()),
			}),
		)
		.await;
	assert_eq!(project.title, "Urban heat islands");

	// Two quotes from one page make one source.
	let first: PublicClip = server
		.json(
			"POST",
			"/api/clips",
			&ada,
			Some(clip(&project, "https://example.org/heat", "heat rises")),
		)
		.await;
	assert_eq!(first.note.as_deref(), Some("key claim"));
	assert_eq!(first.tags, "heat,method");
	let _second: PublicClip = server
		.json(
			"POST",
			"/api/clips",
			&ada,
			Some(clip(
				&project,
				"https://example.org/heat",
				"asphalt stores heat",
			)),
		)
		.await;

	let note: PublicNote = server
		.json(
			"POST",
			&format!("/api/projects/{}/notes", project.id),
			&ada,
			Some(NoteInput {
				body: "Compare with rural stations.".into(),
			}),
		)
		.await;

	let detail: ProjectDetail = server
		.json(
			"GET",
			&format!("/api/projects/{}", project.id),
			&ada,
			None::<()>,
		)
		.await;
	assert_eq!(detail.sources.len(), 1);
	assert_eq!(detail.clips.len(), 2);
	assert_eq!(detail.notes, vec![note.clone()]);
	assert_eq!(detail.clips_from(&detail.sources[0]).count(), 2);

	let library: Vec<ProjectSummary> = server.json("GET", "/api/projects", &ada, None::<()>).await;
	assert_eq!((library[0].sources, library[0].clips), (1, 2));

	let edited: PublicClip = server
		.json(
			"PATCH",
			&format!("/api/clips/{}", first.id),
			&ada,
			Some(ClipChanges {
				note: Some(None),
				tags: Some("Cities".into()),
			}),
		)
		.await;
	assert_eq!((edited.note, edited.tags.as_str()), (None, "cities"));

	let on_page: Vec<ClipOnPage> = server
		.json(
			"GET",
			"/api/clips?url=https%3A%2F%2Fexample.org%2Fheat",
			&ada,
			None::<()>,
		)
		.await;
	assert_eq!(on_page.len(), 2);
	assert_eq!(on_page[0].project.id, project.id);

	let (status, _) = server
		.send(
			"DELETE",
			&format!("/api/clips/{}", first.id),
			Some(&ada),
			None,
		)
		.await;
	assert_eq!(status, StatusCode::NO_CONTENT);
	let (status, _) = server
		.send(
			"DELETE",
			&format!("/api/projects/{}", project.id),
			Some(&ada),
			None,
		)
		.await;
	assert_eq!(status, StatusCode::NO_CONTENT);
	let library: Vec<ProjectSummary> = server.json("GET", "/api/projects", &ada, None::<()>).await;
	assert!(library.is_empty());
}

#[tokio::test]
async fn people_see_only_their_own_research() {
	let server = server().await;
	let ada = server.sign_in("ada@example.com").await;
	let grace = server.sign_in("grace@example.com").await;
	let project: PublicProject = server
		.json(
			"POST",
			"/api/projects",
			&ada,
			Some(NewProjectInput {
				title: "Private".into(),
				question: None,
			}),
		)
		.await;
	let _: PublicClip = server
		.json(
			"POST",
			"/api/clips",
			&ada,
			Some(clip(&project, "https://example.org/a", "a quote")),
		)
		.await;

	let library: Vec<ProjectSummary> = server
		.json("GET", "/api/projects", &grace, None::<()>)
		.await;
	assert!(library.is_empty());
	let path = format!("/api/projects/{}", project.id);
	assert_eq!(
		server.send("GET", &path, Some(&grace), None).await.0,
		StatusCode::NOT_FOUND
	);
	assert_eq!(
		server.send("DELETE", &path, Some(&grace), None).await.0,
		StatusCode::NOT_FOUND
	);
	let (status, _) = server
		.send(
			"POST",
			"/api/clips",
			Some(&grace),
			Some(serde_json::to_value(clip(&project, "https://example.org/b", "theirs")).unwrap()),
		)
		.await;
	assert_eq!(status, StatusCode::NOT_FOUND);
	let on_page: Vec<ClipOnPage> = server
		.json(
			"GET",
			"/api/clips?url=https%3A%2F%2Fexample.org%2Fa",
			&grace,
			None::<()>,
		)
		.await;
	assert!(on_page.is_empty());
}

#[tokio::test]
async fn bad_input_names_the_problem() {
	let server = server().await;
	let ada = server.sign_in("ada@example.com").await;
	let (status, bytes) = server
		.send(
			"POST",
			"/api/projects",
			Some(&ada),
			Some(json!({ "title": "  " })),
		)
		.await;
	assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
	let error: commonplace_shared::ApiError = serde_json::from_slice(&bytes).unwrap();
	assert_eq!(error.error, "invalid");

	let project: PublicProject = server
		.json(
			"POST",
			"/api/projects",
			&ada,
			Some(NewProjectInput {
				title: "P".into(),
				question: None,
			}),
		)
		.await;
	let mut not_a_page = clip(&project, "file:///etc/passwd", "q");
	let (status, _) = server
		.send(
			"POST",
			"/api/clips",
			Some(&ada),
			Some(serde_json::to_value(&not_a_page).unwrap()),
		)
		.await;
	assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
	not_a_page.page.url = "https://example.org".into();
	not_a_page.page.published = Some("last week".into());
	let (status, _) = server
		.send(
			"POST",
			"/api/clips",
			Some(&ada),
			Some(serde_json::to_value(&not_a_page).unwrap()),
		)
		.await;
	assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}
