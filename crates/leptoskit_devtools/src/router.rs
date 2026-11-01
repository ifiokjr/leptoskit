//! The route panel: the router's current route, and navigation from the
//! `DevTools`.

use std::rc::Rc;

use leptoskit_devtools_protocol::Command;
use leptoskit_router::Routable;
use leptoskit_router::Router;
use reactive_graph::effect::ImmediateEffect;
use reactive_graph::graph::untrack;
use reactive_graph::owner::on_cleanup;

use crate::registry;
use crate::registry::Report;
use crate::registry::Slot;
use crate::registry::Source;

/// Report `router`'s current route to the `DevTools`, and let them navigate
/// it.
///
/// Call it once with the router the app renders. The route panel then
/// shows each route as its href the moment the app navigates, and its
/// "Go to" field parses an href into a route and navigates there, as a link
/// would. Tracking a second router replaces the first, and a router tracked
/// inside a component stops being reported when the component unmounts.
///
/// ```ignore
/// let router = Router::<Route>::from_browser()?;
/// #[cfg(feature = "devtools")]
/// leptoskit::devtools::track_router(&router);
/// ```
///
/// Outside a `leptoskit dev` build this does nothing.
pub fn track_router<R>(router: &Router<R>)
where
	R: Routable + Clone + PartialEq + Send + Sync + 'static,
{
	if !registry::active() {
		return;
	}

	let id = registry::register(
		Some(Slot::Route),
		Rc::new(RouteSource {
			router: router.clone(),
		}),
	);
	let watched = router.clone();
	// Reading the route subscribes to navigation; serializing it waits for
	// the agent's flush.
	let effect = ImmediateEffect::new_isomorphic(move || {
		watched.current();
		registry::mark_dirty(id);
	});
	registry::keep(id, effect);
	on_cleanup(move || registry::unregister(id));
}

struct RouteSource<R: 'static> {
	router: Router<R>,
}

impl<R> Source for RouteSource<R>
where
	R: Routable + Clone + PartialEq + Send + Sync + 'static,
{
	fn collect(&self, report: &mut Report) {
		let route = untrack(|| self.router.current());
		report.route(route.href().as_str().to_owned());
	}

	fn apply(&self, command: &Command) -> Option<Result<(), String>> {
		let Command::Navigate { href } = command else {
			return None;
		};
		Some(match R::parse(href) {
			Ok(route) => {
				self.router.navigate(route);
				Ok(())
			}
			Err(_) => Err(format!("no route matches {href:?}")),
		})
	}
}
