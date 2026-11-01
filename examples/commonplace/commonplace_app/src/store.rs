//! The app's state, and the actions that change it.
//!
//! Screens read [`State`] through [`Store::current`]. Actions that need the
//! server run as tasks: they mark what they are loading, call the API, and
//! write the answer back, and the screen renders again.

use std::cell::RefCell;

use commonplace_shared::ClipChanges;
use commonplace_shared::ClipId;
use commonplace_shared::NewClipInput;
use commonplace_shared::NewProjectInput;
use commonplace_shared::NoteId;
use commonplace_shared::PageInput;
use commonplace_shared::ProjectDetail;
use commonplace_shared::ProjectId;
use commonplace_shared::ProjectSummary;
use commonplace_shared::TextQuote;
use commonplace_shared::cite::Style;
use reactive_graph::signal::ArcRwSignal;
use reactive_graph::traits::GetUntracked;
use reactive_graph::traits::Update;
use reactive_graph::traits::With;
use reactive_graph::traits::WithUntracked;

use crate::api::Api;
use crate::api::Problem;
use crate::tasks::spawn;

/// Something loaded from the server.
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "devtools", derive(serde::Serialize, serde::Deserialize))]
pub enum Load<T> {
	#[default]
	Idle,
	Loading,
	Ready(T),
	Failed(String),
}

impl<T> Load<T> {
	/// The loaded value, if there is one.
	pub fn ready(&self) -> Option<&T> {
		match self {
			Self::Ready(value) => Some(value),
			_ => None,
		}
	}
}

/// Whether someone is signed in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "devtools", derive(serde::Serialize, serde::Deserialize))]
pub enum Session {
	/// Still asking the server.
	#[default]
	Checking,
	SignedOut,
	SignedIn,
}

/// The sign-in form.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "devtools", derive(serde::Serialize, serde::Deserialize))]
pub struct SignInForm {
	pub email: String,
	pub code: String,
	/// The code has been sent, so the form asks for it.
	pub sent: bool,
	pub busy: bool,
	pub error: Option<String>,
}

/// A clip being written by hand.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "devtools", derive(serde::Serialize, serde::Deserialize))]
pub struct ClipDraft {
	pub url: String,
	pub title: String,
	pub author: String,
	pub site: String,
	pub published: String,
	pub quote: String,
	pub note: String,
	pub tags: String,
}

/// Everything the screens show.
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "devtools", derive(serde::Serialize, serde::Deserialize))]
pub struct State {
	pub session: Session,
	pub sign_in: SignInForm,
	pub library: Load<Vec<ProjectSummary>>,
	/// The open project.
	pub project: Option<(ProjectId, Load<ProjectDetail>)>,
	pub new_project: NewProjectInput,
	pub clip: ClipDraft,
	pub note: String,
	/// A clip's note being edited, with its text so far.
	pub editing: Option<(ClipId, String)>,
	pub style: Style,
	/// Something went wrong that isn't one form's error.
	pub notice: Option<String>,
	/// An action is waiting for the server.
	pub busy: bool,
	/// The extension's own state.
	pub extension: ExtensionState,
}

/// What the extension's surfaces add.
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "devtools", derive(serde::Serialize, serde::Deserialize))]
pub struct ExtensionState {
	/// The page the popup, or a content script's frame, clips from.
	pub page: Option<PageInput>,
	/// The selection a content script's frame clips.
	pub quote: Option<TextQuote>,
	/// The project clips go to.
	pub project: Option<ProjectId>,
	/// What happened to the last clip, in words.
	pub status: Option<String>,
	/// The sites that show highlights, as match patterns.
	pub sites: Vec<String>,
	/// The site being added in the settings.
	pub site: String,
	/// The clip whose highlight was pressed.
	pub pressed: Option<commonplace_shared::ClipOnPage>,
	/// The clips taken from the page, by their highlight's ID.
	pub on_page: Vec<commonplace_shared::ClipOnPage>,
}

thread_local! {
	static STORE: Store = Store::default();
	static API: RefCell<Option<Api>> = const { RefCell::new(None) };
}

/// The API client the actions use. Each host installs its own at start.
///
/// # Panics
///
/// When no client is installed.
#[must_use]
pub fn api() -> Api {
	API.with_borrow(|api| {
		api.clone()
			.expect("Commonplace installs an API client at start")
	})
}

/// Installs the API client the actions use.
pub fn install(client: Api) {
	API.with_borrow_mut(|api| *api = Some(client));
}

/// A handle on the state. It is cheap to clone, and every clone is the same
/// state.
#[derive(Clone, Default)]
pub struct Store {
	state: ArcRwSignal<State>,
}

impl Store {
	/// The app's store.
	#[must_use]
	pub fn current() -> Self {
		STORE.with(Clone::clone)
	}

	/// Reads the state, so the screen renders again when it changes.
	pub fn with<T>(&self, read: impl FnOnce(&State) -> T) -> T {
		self.state.with(read)
	}

	/// Reads the state without subscribing to it.
	pub fn peek<T>(&self, read: impl FnOnce(&State) -> T) -> T {
		self.state.with_untracked(read)
	}

	/// Changes the state.
	pub fn update(&self, change: impl FnOnce(&mut State)) {
		self.state.update(change);
	}

	/// A snapshot, for tests.
	#[must_use]
	pub fn snapshot(&self) -> State {
		self.state.get_untracked()
	}

	/// Shows the state in `DevTools` as `store`, where it can be read and
	/// edited.
	#[cfg(feature = "devtools")]
	pub fn expose(&self) {
		leptoskit::devtools::expose("store", self.state.clone());
	}

	/// Puts a failure on screen. A signed-out failure returns to sign-in.
	fn failed(&self, problem: &Problem) {
		self.update(|state| {
			state.busy = false;
			if problem.signed_out {
				state.session = Session::SignedOut;
			} else {
				state.notice = Some(problem.message.clone());
			}
		});
	}

	/// Finds out whether someone is signed in, then loads the library.
	pub fn start(&self, token: Option<String>) {
		let store = self.clone();
		spawn(async move {
			let api = api();
			if api.uses_bearer() && token.is_none() {
				store.update(|state| state.session = Session::SignedOut);
				return;
			}
			api.set_session(token);
			match api.signed_in().await {
				Ok(true) => {
					store.update(|state| state.session = Session::SignedIn);
					store.load_library();
				}
				Ok(false) => store.update(|state| state.session = Session::SignedOut),
				Err(problem) => {
					store.update(|state| {
						state.session = Session::SignedOut;
						state.sign_in.error = Some(problem.message);
					});
				}
			}
		});
	}

	/// Sends a sign-in code to the address in the form.
	pub fn send_code(&self) {
		let email = self.peek(|state| state.sign_in.email.trim().to_owned());
		if email.is_empty() {
			self.update(|state| state.sign_in.error = Some("Enter your email address.".into()));
			return;
		}
		self.update(|state| {
			state.sign_in.busy = true;
			state.sign_in.error = None;
		});
		let store = self.clone();
		spawn(async move {
			let result = api().send_code(&email).await;
			store.update(|state| {
				state.sign_in.busy = false;
				match result {
					Ok(()) => state.sign_in.sent = true,
					Err(problem) => state.sign_in.error = Some(problem.message),
				}
			});
		});
	}

	/// Signs in with the code in the form.
	pub fn verify_code(&self) {
		let (email, code) = self.peek(|state| {
			(
				state.sign_in.email.trim().to_owned(),
				state.sign_in.code.trim().to_owned(),
			)
		});
		self.update(|state| {
			state.sign_in.busy = true;
			state.sign_in.error = None;
		});
		let store = self.clone();
		spawn(async move {
			match api().verify_code(&email, &code).await {
				Ok(token) => {
					crate::session::keep(token).await;
					store.update(|state| {
						state.sign_in = SignInForm::default();
						state.session = Session::SignedIn;
					});
					store.load_library();
				}
				Err(problem) => {
					store.update(|state| {
						state.sign_in.busy = false;
						state.sign_in.error = Some(problem.message);
					})
				}
			}
		});
	}

	/// Signs out, here and on the server.
	pub fn sign_out(&self) {
		let store = self.clone();
		spawn(async move {
			let _ = api().sign_out().await;
			crate::session::keep(None).await;
			store.update(|state| {
				*state = State {
					session: Session::SignedOut,
					..State::default()
				}
			});
		});
	}

	/// Loads the library again.
	pub fn load_library(&self) {
		self.update(|state| {
			if state.library.ready().is_none() {
				state.library = Load::Loading;
			}
		});
		let store = self.clone();
		spawn(async move {
			match api().projects().await {
				Ok(projects) => store.update(|state| state.library = Load::Ready(projects)),
				Err(problem) if problem.signed_out => store.failed(&problem),
				Err(problem) => store.update(|state| state.library = Load::Failed(problem.message)),
			}
		});
	}

	/// Opens a project, loading it unless it is already open. Screens call
	/// this while rendering, so it only starts work that isn't under way.
	pub fn open_project(&self, id: ProjectId) {
		let open = self.peek(|state| matches!(&state.project, Some((open, _)) if *open == id));
		if open {
			return;
		}
		self.reload_project(id);
	}

	/// Loads the open project again.
	pub fn reload_project(&self, id: ProjectId) {
		let store = self.clone();
		// Writing state while a screen renders would render it again in the
		// middle; the task starts after this render.
		spawn(async move {
			store.update(|state| {
				let keep = matches!(&state.project, Some((open, Load::Ready(_))) if *open == id);
				if !keep {
					state.project = Some((id, Load::Loading));
				}
			});
			let result = api().project(id).await;
			store.update(|state| {
				state.project = Some((
					id,
					match result {
						Ok(detail) => Load::Ready(detail),
						Err(problem) => Load::Failed(problem.message),
					},
				));
			});
		});
	}

	/// Creates a project from the form, then opens it with `then`.
	pub fn create_project(&self, then: impl FnOnce(ProjectId) + 'static) {
		let input = self.peek(|state| state.new_project.clone());
		if input.title.trim().is_empty() {
			self.update(|state| state.notice = Some("Give the project a title.".into()));
			return;
		}
		self.update(|state| state.busy = true);
		let store = self.clone();
		spawn(async move {
			match api().create_project(&input).await {
				Ok(project) => {
					store.update(|state| {
						state.busy = false;
						state.notice = None;
						state.new_project = NewProjectInput::default();
					});
					store.load_library();
					then(project.id);
				}
				Err(problem) => store.failed(&problem),
			}
		});
	}

	/// Deletes a project and everything in it.
	pub fn delete_project(&self, id: ProjectId, then: impl FnOnce() + 'static) {
		let store = self.clone();
		spawn(async move {
			match api().delete_project(id).await {
				Ok(()) => {
					store.update(|state| state.project = None);
					store.load_library();
					then();
				}
				Err(problem) => store.failed(&problem),
			}
		});
	}

	/// Saves the clip written by hand into the open project.
	pub fn add_clip(&self, project: ProjectId) {
		let draft = self.peek(|state| state.clip.clone());
		let optional = |text: &str| {
			let text = text.trim();
			(!text.is_empty()).then(|| text.to_owned())
		};
		let input = NewClipInput {
			project: Some(project),
			page: PageInput {
				url: draft.url.trim().to_owned(),
				title: draft.title.trim().to_owned(),
				author: optional(&draft.author),
				site: optional(&draft.site),
				published: optional(&draft.published),
			},
			quote: TextQuote {
				exact: draft.quote.trim().to_owned(),
				prefix: None,
				suffix: None,
			},
			note: optional(&draft.note),
			tags: draft.tags.clone(),
		};
		self.update(|state| state.busy = true);
		let store = self.clone();
		spawn(async move {
			match api().create_clip(&input).await {
				Ok(_) => {
					store.update(|state| {
						state.busy = false;
						state.notice = None;
						state.clip = ClipDraft::default();
					});
					store.reload_project(project);
				}
				Err(problem) => store.failed(&problem),
			}
		});
	}

	/// Saves the note being edited for a clip.
	pub fn save_clip_note(&self, project: ProjectId) {
		let Some((clip, note)) = self.peek(|state| state.editing.clone()) else {
			return;
		};
		let store = self.clone();
		spawn(async move {
			let changes = ClipChanges {
				note: Some(Some(note)),
				tags: None,
			};
			match api().update_clip(clip, &changes).await {
				Ok(_) => {
					store.update(|state| state.editing = None);
					store.reload_project(project);
				}
				Err(problem) => store.failed(&problem),
			}
		});
	}

	/// Deletes a clip.
	pub fn delete_clip(&self, project: ProjectId, clip: ClipId, then: impl FnOnce() + 'static) {
		let store = self.clone();
		spawn(async move {
			match api().delete_clip(clip).await {
				Ok(()) => {
					store.reload_project(project);
					then();
				}
				Err(problem) => store.failed(&problem),
			}
		});
	}

	/// Adds the note in the form to a project.
	pub fn add_note(&self, project: ProjectId) {
		let body = self.peek(|state| state.note.trim().to_owned());
		if body.is_empty() {
			return;
		}
		let store = self.clone();
		spawn(async move {
			match api().create_note(project, &body).await {
				Ok(_) => {
					store.update(|state| state.note.clear());
					store.reload_project(project);
				}
				Err(problem) => store.failed(&problem),
			}
		});
	}

	/// Deletes a note.
	pub fn delete_note(&self, project: ProjectId, note: NoteId) {
		let store = self.clone();
		spawn(async move {
			match api().delete_note(note).await {
				Ok(()) => store.reload_project(project),
				Err(problem) => store.failed(&problem),
			}
		});
	}
}
