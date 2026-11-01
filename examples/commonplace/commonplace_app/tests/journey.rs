//! A research session the way a person has one, over the real screens on the
//! headless backend, with the real server in-process behind a bearer
//! transport, as the native apps use it.

use std::time::Duration;

use commonplace_app::api::Api;
use commonplace_app::routes::Navigation;
use commonplace_app::routes::Route;
use commonplace_app::store;
use commonplace_app::store::Store;
use commonplace_app::transport::Request;
use commonplace_app::transport::Response;
use commonplace_app::transport::Sending;
use commonplace_app::transport::Transport;
use http_body_util::BodyExt;
use leptoskit::auth::Auth;
use leptoskit::auth::Keys;
use leptoskit::auth::Mailbox;
use leptoskit::server::axum::Router;
use leptoskit::server::axum::body::Body;
use leptoskit_testing::Finder;
use leptoskit_testing::WidgetTester;
use tower::ServiceExt;

/// The server, called in-process.
struct InProcess(Router);

impl Transport for InProcess {
	fn send(&self, request: Request) -> Sending {
		let router = self.0.clone();
		Box::pin(async move {
			let mut builder = leptoskit::server::axum::http::Request::builder()
				.method(request.method)
				.uri(&request.path);
			if let Some(token) = &request.bearer {
				builder = builder.header("authorization", format!("Bearer {token}"));
			}
			if request.body.is_some() {
				builder = builder.header("content-type", "application/json");
			}
			let body = request.body.map_or_else(Body::empty, Body::from);
			let response = router
				.oneshot(builder.body(body).map_err(|error| error.to_string())?)
				.await
				.unwrap_or_else(|never| match never {});
			let status = response.status().as_u16();
			let body = response
				.into_body()
				.collect()
				.await
				.map_err(|error| error.to_string())?
				.to_bytes()
				.to_vec();
			Ok(Response { status, body })
		})
	}

	fn uses_bearer(&self) -> bool {
		true
	}
}

async fn server() -> (Router, Mailbox) {
	let db = leptoskit::db::testing::sqlite_migrated(commonplace_db::MIGRATIONS).await;
	let mailbox = Mailbox::new();
	let auth = Auth::builder(&db, Keys::single("test", [5; 32]))
		.mailer(mailbox.clone())
		.build()
		.unwrap();
	let router = commonplace_backend::router()
		.merge(auth.router())
		.layer(auth.layer());
	(router, mailbox)
}

/// Lets the app's tasks run, renders, and waits until `finder` is on
/// screen.
async fn until(tester: &mut WidgetTester, finder: &Finder) {
	for _ in 0..400 {
		tokio::time::sleep(Duration::from_millis(5)).await;
		tester.pump();
		if tester.find(finder).is_ok() {
			return;
		}
	}
	panic!("{finder:?} never appeared:\n{}", tester.tree());
}

#[tokio::test(flavor = "current_thread")]
async fn sign_in_start_a_project_clip_and_cite() {
	tokio::task::LocalSet::new()
		.run_until(async {
			let (router, mailbox) = server().await;
			store::install(Api::new(InProcess(router)));
			let navigation = Navigation::new(Route::Library);
			let render = navigation.clone();
			let mut tester = WidgetTester::new(move || render.render(), 800.0, 3000.0).unwrap();

			// A native app with no kept token starts signed out.
			Store::current().start(None);
			until(&mut tester, &Finder::text("Sign in to Commonplace")).await;

			tester
				.enter_text(&Finder::label("Email"), "ada@example.com")
				.unwrap();
			tester.tap(&Finder::text("Email me a code")).unwrap();
			until(&mut tester, &Finder::label("Code")).await;
			let code = mailbox.latest_to("ada@example.com").unwrap().subject;
			let code = code.rsplit(' ').next().unwrap();
			tester.enter_text(&Finder::label("Code"), code).unwrap();
			tester.tap(&Finder::text("Sign in")).unwrap();
			until(&mut tester, &Finder::text("Your projects")).await;
			assert!(
				store::api().session().is_some(),
				"the app keeps its bearer token"
			);
			// An access token that stopped working is renewed with the
			// refresh token, and the request goes through.
			let kept = store::api().session().unwrap();
			let mut tokens: serde_json::Value = serde_json::from_str(&kept).unwrap();
			tokens["access_token"] = "expired".into();
			store::api().set_session(Some(tokens.to_string()));
			store::api().projects().await.unwrap();
			let renewed = store::api().session().unwrap();
			assert_ne!(renewed, kept, "the refresh replaced both tokens");
			assert!(!renewed.contains("expired"));
			until(&mut tester, &Finder::text("Start a project")).await;

			tester
				.enter_text(&Finder::label("Title"), "Urban heat islands")
				.unwrap();
			tester
				.enter_text(&Finder::label("Question"), "Why are cities hotter?")
				.unwrap();
			tester.tap(&Finder::text("Create project")).unwrap();
			until(&mut tester, &Finder::text("Clip a quote by hand")).await;
			assert!(matches!(navigation.current(), Route::Project(_)));

			tester
				.enter_text(&Finder::label("Quote"), "Asphalt stores the day's heat")
				.unwrap();
			tester
				.enter_text(&Finder::label("Page address"), "https://example.org/heat")
				.unwrap();
			tester
				.enter_text(&Finder::label("Page title"), "Why cities run hot")
				.unwrap();
			tester
				.enter_text(&Finder::label("Author"), "Ada Lovelace")
				.unwrap();
			tester
				.enter_text(&Finder::label("Published"), "2024-05-01")
				.unwrap();
			tester
				.enter_text(&Finder::label("Tags"), "Heat, method")
				.unwrap();
			tester.tap(&Finder::text("Save clip")).unwrap();
			until(
				&mut tester,
				&Finder::text("\u{201c}Asphalt stores the day's heat\u{201d}"),
			)
			.await;
			until(&mut tester, &Finder::text("#heat #method")).await;

			tester
				.enter_text(&Finder::label("New note"), "Compare rural stations.")
				.unwrap();
			tester.tap(&Finder::text("Add note")).unwrap();
			until(&mut tester, &Finder::text("Compare rural stations.")).await;

			tester.tap(&Finder::text("Export bibliography")).unwrap();
			until(&mut tester, &Finder::label("Bibliography")).await;
			until(
				&mut tester,
				&Finder::text(
					"Lovelace, A. (2024, May 1). Why cities run hot. https://example.org/heat",
				),
			)
			.await;
			tester.tap(&Finder::text("BibTeX")).unwrap();
			until(&mut tester, &Finder::text("BibTeX (shown)")).await;

			tester.tap(&Finder::text("Back to the project")).unwrap();
			until(&mut tester, &Finder::text("Clip a quote by hand")).await;
			tester.tap(&Finder::text("Sign out")).unwrap();
			until(&mut tester, &Finder::text("Sign in to Commonplace")).await;
			assert_eq!(store::api().session(), None);
		})
		.await;
}
