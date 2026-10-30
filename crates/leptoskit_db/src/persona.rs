//! Who the studio can view the data as: each persona turns what you type,
//! such as a user's ID, into the context a request from them would have.

use crate::Context;

/// A kind of request the studio can view the data as, registered with
/// [`register_persona!`](crate::register_persona). `leptoskit_auth`
/// registers `account`, a signed-in account by its ID; register your own
/// for the globals your policies read:
///
/// ```ignore
/// leptoskit::db::register_persona!(leptoskit::db::Persona {
/// 	name: "viewer",
/// 	describe: "a reader, by their user ID",
/// 	context: |id| {
/// 		let id = id.parse().map_err(|error| format!("not a user ID: {error}"))?;
/// 		Ok(leptoskit::db::Context::new().with(Viewer(id)))
/// 	},
/// });
/// ```
///
/// Registering one adds a function pointer and two strings to the program;
/// only the studio, which release builds leave out, calls it.
#[derive(Clone, Copy, Debug)]
pub struct Persona {
	/// What the studio calls it, such as `account`.
	pub name: &'static str,
	/// What to type, such as "a signed-in account, by its ID".
	pub describe: &'static str,
	/// The context a request from the persona with this identity has, or why
	/// the text names nobody.
	pub context: fn(&str) -> Result<Context, String>,
}

inventory::collect!(Persona);

/// Every registered persona, by name.
#[cfg(all(
	feature = "studio",
	debug_assertions,
	any(feature = "postgres", feature = "sqlite")
))]
pub(crate) fn personas() -> Vec<&'static Persona> {
	let mut personas: Vec<&'static Persona> = inventory::iter::<Persona>().collect();
	personas.sort_by_key(|persona| persona.name);
	personas
}

/// Registers a [`Persona`] for the studio. It compiles to nothing without the
/// `server` feature.
#[macro_export]
macro_rules! register_persona {
	($persona:expr $(,)?) => {
		$crate::__server! {
			$crate::__private::inventory::submit! { $persona }
		}
	};
}
