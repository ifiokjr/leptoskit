//! Commonplace's tables. Every row belongs to the account that made it, and
//! each table's policy lets only that account see or change it.
//!
//! Clients compile only the `Public…` types: the browser, the native apps,
//! and the extension receive rows in that form and nothing else.
//!
//! After changing a table, run `leptoskit db migration create <name>` and
//! commit what it writes under `migrations/`.

use leptoskit_auth::Account;
#[cfg(feature = "server")]
use leptoskit_auth::AuthContext;
use leptoskit_db::Id;
use leptoskit_db::Timestamp;
use leptoskit_db::policy;
use leptoskit_db::table;

leptoskit_db::packs!(leptoskit_auth);

/// A research question, and everything collected for it.
#[table(name = "projects", public, index(owner_id))]
#[policy(all, |project, cx| cx.signed_in(|me| project.owner_id.eq(me.account)))]
pub struct Project {
	pub id: Id<Project>,
	#[db(on_delete = cascade)]
	pub owner_id: Id<Account>,
	/// The project's name, such as "Urban heat islands".
	#[db(min_len = 1, max_len = 120)]
	pub title: String,
	/// The question the project tries to answer.
	#[db(max_len = 500)]
	pub question: Option<String>,
	#[db(created_at)]
	pub created_at: Timestamp,
	#[db(updated_at)]
	pub updated_at: Timestamp,
}

/// A page someone read: everything a citation needs.
#[table(name = "sources", public, index(project_id), unique(project_id, url))]
#[policy(all, |source, cx| cx.signed_in(|me| source.owner_id.eq(me.account)))]
pub struct Source {
	pub id: Id<Source>,
	#[db(on_delete = cascade)]
	pub owner_id: Id<Account>,
	#[db(on_delete = cascade)]
	pub project_id: Id<Project>,
	#[db(min_len = 1, max_len = 2048)]
	pub url: String,
	#[db(min_len = 1, max_len = 300)]
	pub title: String,
	#[db(max_len = 200)]
	pub author: Option<String>,
	/// The site's name, such as "The Guardian".
	#[db(max_len = 200)]
	pub site: Option<String>,
	/// When the page was published, as `YYYY`, `YYYY-MM`, or `YYYY-MM-DD`.
	#[db(max_len = 10, regex = "^[0-9]{4}(-[0-9]{2}(-[0-9]{2})?)?$")]
	pub published: Option<String>,
	/// When it was first clipped, which citations call the access date.
	#[db(created_at)]
	pub accessed_at: Timestamp,
}

/// A quote from a source, with the reader's note and tags.
///
/// `prefix` and `suffix` are a little of the text on each side of the quote,
/// a W3C Web Annotation text-quote selector, so the quote can be found on the
/// page again even when it appears there more than once.
#[table(name = "clips", public, index(project_id), index(source_id))]
#[policy(all, |clip, cx| cx.signed_in(|me| clip.owner_id.eq(me.account)))]
pub struct Clip {
	pub id: Id<Clip>,
	#[db(on_delete = cascade)]
	pub owner_id: Id<Account>,
	#[db(on_delete = cascade)]
	pub project_id: Id<Project>,
	#[db(on_delete = cascade)]
	pub source_id: Id<Source>,
	#[db(min_len = 1, max_len = 5000)]
	pub quote: String,
	#[db(max_len = 64)]
	pub prefix: Option<String>,
	#[db(max_len = 64)]
	pub suffix: Option<String>,
	#[db(max_len = 5000)]
	pub note: Option<String>,
	/// Comma-separated, lowercase tags, such as `method,cities`.
	#[db(max_len = 500, default = "")]
	pub tags: String,
	#[db(created_at)]
	pub created_at: Timestamp,
	#[db(updated_at)]
	pub updated_at: Timestamp,
}

/// A note in a project, written in markdown.
#[table(name = "notes", public, index(project_id))]
#[policy(all, |note, cx| cx.signed_in(|me| note.owner_id.eq(me.account)))]
pub struct Note {
	pub id: Id<Note>,
	#[db(on_delete = cascade)]
	pub owner_id: Id<Account>,
	#[db(on_delete = cascade)]
	pub project_id: Id<Project>,
	#[db(min_len = 1, max_len = 20000)]
	pub body: String,
	#[db(created_at)]
	pub created_at: Timestamp,
	#[db(updated_at)]
	pub updated_at: Timestamp,
}

leptoskit_db::export_schema!();

/// Every migration, embedded for the server to apply at startup.
#[cfg(feature = "server")]
pub static MIGRATIONS: &[leptoskit_db::Migration] = leptoskit_db::migrations!("../migrations");
