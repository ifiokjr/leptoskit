//! Which parts of the app one server process runs: one binary, started
//! with `--role`, can be the whole app or one part of a larger deployment.
//!
//! ```ignore
//! let roles = rpc::server::Roles::from_env_and_args()?; // all four unless asked
//!
//! if roles.web() { /* HTTP, server rendering, the RPC routes */ }
//! if roles.live() { /* the /live sockets */ }
//! if roles.worker() { /* jobs, cron, change log upkeep */ }
//! if roles.capture() { /* replication capture, the relay publisher */ }
//! ```
//!
//! | Role      | What it starts                                                            |
//! | --------- | ------------------------------------------------------------------------- |
//! | `web`     | HTTP: pages, server rendering, `POST /api/<name>`, sign-in                |
//! | `live`    | The live sockets: `GET /live` and its server-sent events fallback         |
//! | `worker`  | Background jobs, cron, and pruning the change log                         |
//! | `capture` | Replication capture (`[db] live_capture = "replication"`) and the relay's publisher |
//!
//! Every process serves `/healthz` and `/readyz` whatever its roles, and
//! `/readyz` checks only what its roles started.

use std::fmt;
use std::str::FromStr;

/// The environment variable [`Roles::from_env`] reads, such as
/// `LEPTOSKIT_ROLE=web,live`.
pub const ROLE_ENV: &str = "LEPTOSKIT_ROLE";

/// One part of the app a process can run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Role {
	/// HTTP: pages, server rendering, the RPC routes, and sign-in.
	Web,
	/// The live sockets.
	Live,
	/// Background jobs, cron, and the change log's upkeep.
	Worker,
	/// Replication capture and the live relay's publisher.
	Capture,
}

impl Role {
	/// Every role, in order.
	pub const ALL: [Self; 4] = [Self::Web, Self::Live, Self::Worker, Self::Capture];

	/// Its name, as `--role` takes it.
	pub fn as_str(self) -> &'static str {
		match self {
			Self::Web => "web",
			Self::Live => "live",
			Self::Worker => "worker",
			Self::Capture => "capture",
		}
	}

	fn bit(self) -> u8 {
		match self {
			Self::Web => 1,
			Self::Live => 2,
			Self::Worker => 4,
			Self::Capture => 8,
		}
	}
}

impl fmt::Display for Role {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str(self.as_str())
	}
}

impl FromStr for Role {
	type Err = RolesError;

	fn from_str(name: &str) -> Result<Self, Self::Err> {
		Self::ALL
			.into_iter()
			.find(|role| role.as_str().eq_ignore_ascii_case(name.trim()))
			.ok_or_else(|| RolesError::Unknown(name.trim().to_owned()))
	}
}

/// The roles one process runs: any combination, all of them by default.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Roles(u8);

impl Roles {
	/// Every role: the whole app in one process.
	pub const ALL: Self = Self(15);

	/// Just `roles`.
	pub fn only(roles: impl IntoIterator<Item = Role>) -> Self {
		Self(roles.into_iter().fold(0, |bits, role| bits | role.bit()))
	}

	/// Whether it runs `role`.
	pub fn has(self, role: Role) -> bool {
		self.0 & role.bit() != 0
	}

	/// Whether it serves HTTP: pages, server rendering, the RPC routes.
	pub fn web(self) -> bool {
		self.has(Role::Web)
	}

	/// Whether it holds live sockets.
	pub fn live(self) -> bool {
		self.has(Role::Live)
	}

	/// Whether it runs jobs, cron and the change log's upkeep.
	pub fn worker(self) -> bool {
		self.has(Role::Worker)
	}

	/// Whether it runs replication capture and the relay's publisher.
	pub fn capture(self) -> bool {
		self.has(Role::Capture)
	}

	/// Whether it runs every role.
	pub fn is_all(self) -> bool {
		self == Self::ALL
	}

	/// The roles it runs, in order.
	pub fn iter(self) -> impl Iterator<Item = Role> {
		Role::ALL.into_iter().filter(move |role| self.has(*role))
	}

	/// Roles from a list such as `web,live`; `all` means every role.
	///
	/// # Errors
	///
	/// When a name isn't a role, or the list names none.
	pub fn parse(list: &str) -> Result<Self, RolesError> {
		let mut roles = Self(0);

		for name in list
			.split(',')
			.map(str::trim)
			.filter(|name| !name.is_empty())
		{
			if name.eq_ignore_ascii_case("all") {
				roles = Self::ALL;
			} else {
				roles.0 |= name.parse::<Role>()?.bit();
			}
		}

		if roles.0 == 0 {
			return Err(RolesError::Empty);
		}

		Ok(roles)
	}

	/// The roles [`ROLE_ENV`] names, or every role when it isn't set.
	///
	/// # Errors
	///
	/// As [`Roles::parse`], when it's set.
	pub fn from_env() -> Result<Self, RolesError> {
		match std::env::var(ROLE_ENV) {
			Ok(list) if !list.trim().is_empty() => Self::parse(&list),
			_ => Ok(Self::ALL),
		}
	}

	/// The roles `--role` names in `args`, as `--role web,live`,
	/// `--role=web` or `--role web --role live`; without one, the
	/// environment's ([`Roles::from_env`]). Other arguments are left for the
	/// app.
	///
	/// # Errors
	///
	/// As [`Roles::parse`], or when `--role` has no value.
	pub fn from_args<I, S>(args: I) -> Result<Self, RolesError>
	where
		I: IntoIterator<Item = S>,
		S: AsRef<str>,
	{
		let mut named: Option<Self> = None;
		let mut args = args.into_iter();

		while let Some(arg) = args.next() {
			let arg = arg.as_ref();
			let list = if arg == "--role" {
				args.next()
					.map(|value| value.as_ref().to_owned())
					.ok_or(RolesError::Missing)?
			} else if let Some(list) = arg.strip_prefix("--role=") {
				list.to_owned()
			} else {
				continue;
			};
			let roles = Self::parse(&list)?;
			named = Some(named.map_or(roles, |named| Self(named.0 | roles.0)));
		}

		match named {
			Some(roles) => Ok(roles),
			None => Self::from_env(),
		}
	}

	/// [`Roles::from_args`] with this process's arguments.
	///
	/// # Errors
	///
	/// As [`Roles::from_args`].
	pub fn from_env_and_args() -> Result<Self, RolesError> {
		Self::from_args(std::env::args().skip(1))
	}
}

impl Default for Roles {
	fn default() -> Self {
		Self::ALL
	}
}

impl fmt::Display for Roles {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		let names: Vec<&str> = self.iter().map(Role::as_str).collect();
		f.write_str(&names.join(","))
	}
}

impl fmt::Debug for Roles {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "Roles({self})")
	}
}

impl FromStr for Roles {
	type Err = RolesError;

	fn from_str(list: &str) -> Result<Self, Self::Err> {
		Self::parse(list)
	}
}

impl FromIterator<Role> for Roles {
	fn from_iter<I: IntoIterator<Item = Role>>(roles: I) -> Self {
		Self::only(roles)
	}
}

/// Roles that don't parse.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RolesError {
	/// A name that isn't a role.
	#[error("`{0}` isn't a role: use web, live, worker, capture, or all")]
	Unknown(String),
	/// A list with no roles in it.
	#[error("no roles given: use web, live, worker, capture, or all")]
	Empty,
	/// `--role` at the end, with nothing after it.
	#[error("`--role` needs a value, such as `--role web,live`")]
	Missing,
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn every_role_by_default() {
		assert_eq!(Roles::default(), Roles::ALL);
		assert!(
			Roles::ALL.web() && Roles::ALL.live() && Roles::ALL.worker() && Roles::ALL.capture()
		);
		assert_eq!(Roles::ALL.to_string(), "web,live,worker,capture");
	}

	#[test]
	fn lists_parse_in_any_order_and_case() {
		let roles = Roles::parse(" Live, web ").unwrap();
		assert!(roles.web() && roles.live() && !roles.worker() && !roles.capture());
		assert_eq!(roles.to_string(), "web,live");
		assert_eq!(Roles::parse("all").unwrap(), Roles::ALL);
		assert_eq!(
			Roles::parse("bogus"),
			Err(RolesError::Unknown("bogus".into()))
		);
		assert_eq!(Roles::parse(" , "), Err(RolesError::Empty));
	}

	#[test]
	fn the_flag_takes_every_form_and_adds_up() {
		let roles =
			Roles::from_args(["--port", "3000", "--role", "worker", "--role=capture"]).unwrap();
		assert_eq!(roles, Roles::only([Role::Worker, Role::Capture]));
		assert_eq!(Roles::from_args(["--role"]), Err(RolesError::Missing));
		assert_eq!(
			Roles::from_args(["--role", "web,live"]).unwrap(),
			[Role::Web, Role::Live].into_iter().collect()
		);
	}
}
