//! Network state is a hint, never a guarantee that a particular server is
//! reachable. Android can expose OS-validated internet access; all other
//! backends leave it unknown. No probe, public-IP service, or network request
//! is made by this crate.
//!
//! The wire snapshot is eight little-endian u32s: connection kind, connected,
//! internet validated, expensive, data saver, estimated kbps, estimated RTT ms,
//! effective connection class. Boolean fields use unknown/false/true = 0/1/2;
//! numeric estimates use `u32::MAX` for unknown.

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

/// Registered plugin name.
pub const NAME: &str = "network";
/// Eight little-endian u32 fields.
const SNAPSHOT_BYTES: usize = 32;
/// Empty payload -> current [`Snapshot`].
pub const SNAPSHOT: MethodId = MethodId(0);
/// Previous encoded snapshot -> next differing snapshot. Mobile/Apple only.
/// Dropping the guest call unregisters its native observer. One outstanding
/// wait is supported per plugin; a second receives `Busy`.
pub const WAIT_FOR_CHANGE: MethodId = MethodId(1);

/// Transport reported by the OS, or Unknown when it cannot be determined.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Connection {
	/// No trustworthy transport information.
	Unknown = 0,
	/// Platform reports no active connection.
	None = 1,
	/// Wireless LAN.
	Wifi = 2,
	/// Cellular data.
	Cellular = 3,
	/// Wired LAN.
	Ethernet = 4,
	/// Bluetooth network.
	Bluetooth = 5,
	/// Virtual private network.
	Vpn = 6,
	/// A known connection with another transport.
	Other = 7,
}

impl Connection {
	/// Every transport the wire snapshot defines, in code order.
	const ALL: [Self; 8] = [
		Self::Unknown,
		Self::None,
		Self::Wifi,
		Self::Cellular,
		Self::Ethernet,
		Self::Bluetooth,
		Self::Vpn,
		Self::Other,
	];

	/// Resolves a wire code, refusing codes the snapshot does not define.
	fn from_code(code: u32) -> Option<Self> {
		Self::ALL
			.into_iter()
			.find(|connection| *connection as u32 == code)
	}
}

/// Browser's estimated performance class, not the physical cellular generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum EffectiveType {
	/// No estimate available.
	Unknown = 0,
	/// Very slow connection.
	Slow2g = 1,
	/// Estimated 2G performance.
	TwoG = 2,
	/// Estimated 3G performance.
	ThreeG = 3,
	/// Estimated 4G performance.
	FourG = 4,
}

/// A single system reading. Unknown capabilities remain `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Snapshot {
	/// Platform connection transport.
	pub connection: Connection,
	/// Local connection/online hint; does not establish internet connectivity.
	pub connected: Option<bool>,
	/// OS-validated internet capability (currently Android only).
	pub internet_validated: Option<bool>,
	/// Metered/expensive connection, if reported by the platform.
	pub expensive: Option<bool>,
	/// Low-data/save-data mode, if reported by the platform.
	pub data_saver: Option<bool>,
	/// Browser estimate in kilobits per second; not a measured throughput test.
	pub downlink_kbps: Option<u32>,
	/// Browser estimate in milliseconds.
	pub round_trip_ms: Option<u32>,
	/// Browser's estimated connection performance.
	pub effective_type: EffectiveType,
}

fn flag(value: Option<bool>) -> u32 {
	match value {
		None => 0,
		Some(false) => 1,
		Some(true) => 2,
	}
}
fn read_flag(value: u32) -> Result<Option<bool>, Error> {
	match value {
		0 => Ok(None),
		1 => Ok(Some(false)),
		2 => Ok(Some(true)),
		_ => Err(Error::InvalidArgument),
	}
}
fn optional(value: u32) -> Option<u32> {
	(value != u32::MAX).then_some(value)
}

impl Snapshot {
	/// Creates a reading with only a local connectivity hint.
	pub fn connection(connection: Connection, connected: Option<bool>) -> Self {
		Self {
			connection,
			connected,
			internet_validated: None,
			expensive: None,
			data_saver: None,
			downlink_kbps: None,
			round_trip_ms: None,
			effective_type: EffectiveType::Unknown,
		}
	}

	/// Encodes a snapshot. `u32::MAX` is reserved and cannot be an estimate.
	pub fn encode(self) -> Result<Bytes, Error> {
		if self.downlink_kbps == Some(u32::MAX) || self.round_trip_ms == Some(u32::MAX) {
			return Err(Error::InvalidArgument);
		}

		let mut output = Encoder::with_capacity(SNAPSHOT_BYTES);
		for field in [
			self.connection as u32,
			flag(self.connected),
			flag(self.internet_validated),
			flag(self.expensive),
			flag(self.data_saver),
			self.downlink_kbps.unwrap_or(u32::MAX),
			self.round_trip_ms.unwrap_or(u32::MAX),
			self.effective_type as u32,
		] {
			output.u32(field);
		}
		Ok(output.finish())
	}

	/// Rejects truncated/trailing data and invalid enum/boolean values.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);

		let connection = Connection::from_code(input.u32()?).ok_or(Error::InvalidArgument)?;

		let connected = read_flag(input.u32()?)?;
		let internet_validated = read_flag(input.u32()?)?;
		let expensive = read_flag(input.u32()?)?;
		let data_saver = read_flag(input.u32()?)?;
		let downlink_kbps = optional(input.u32()?);
		let round_trip_ms = optional(input.u32()?);

		let effective_type = match input.u32()? {
			0 => EffectiveType::Unknown,
			1 => EffectiveType::Slow2g,
			2 => EffectiveType::TwoG,
			3 => EffectiveType::ThreeG,
			4 => EffectiveType::FourG,
			_ => return Err(Error::InvalidArgument),
		};

		input.finish()?;
		Ok(Self {
			connection,
			connected,
			internet_validated,
			expensive,
			data_saver,
			downlink_kbps,
			round_trip_ms,
			effective_type,
		})
	}
}

/// Desktop interface enumeration. A live local interface does not establish a
/// default route or internet access. Virtual interfaces can also be up.
#[cfg(any(
	target_os = "macos",
	target_os = "windows",
	all(target_os = "linux", not(target_env = "ohos"))
))]
pub mod desktop {
	use super::*;
	/// Lists local interface names and IP addresses using system APIs.
	pub fn interfaces() -> Result<Vec<if_addrs::Interface>, Error> {
		if_addrs::get_if_addrs().map_err(|_| Error::Native)
	}
	/// Reads operational interface state without inferring transport from
	/// names.
	pub fn snapshot() -> Result<Snapshot, Error> {
		let interfaces = interfaces()?;
		Ok(from_interfaces(&interfaces))
	}
	/// Loopback and link-local addresses exist without any reachable network:
	/// offline Macs keep `awdl0`/`utun*` up with `fe80::` addresses, and failed
	/// DHCP leaves a 169.254.0.0/16 address. Neither indicates connectivity.
	fn routable(entry: &if_addrs::Interface) -> bool {
		!entry.is_loopback()
			&& match entry.ip() {
				std::net::IpAddr::V4(ip) => !ip.is_link_local(),
				std::net::IpAddr::V6(ip) => !ip.is_unicast_link_local(),
			}
	}
	fn from_interfaces(interfaces: &[if_addrs::Interface]) -> Snapshot {
		let connected = interfaces
			.iter()
			.filter(|entry| routable(entry))
			.any(if_addrs::Interface::is_oper_up);
		let unknown = interfaces
			.iter()
			.filter(|entry| routable(entry))
			.any(|entry| entry.oper_status == if_addrs::IfOperStatus::Unknown);

		if connected {
			Snapshot::connection(Connection::Unknown, Some(true))
		} else if unknown {
			Snapshot::connection(Connection::Unknown, None)
		} else {
			Snapshot::connection(Connection::None, Some(false))
		}
	}
	/// Synchronous host adapter. Change waiting is unsupported on this adapter;
	/// Apple UI hosts can install the NWPathMonitor-backed Swift
	/// implementation.
	#[derive(Debug, Default)]
	pub struct Network;
	impl leptoskit_bridge::Plugin for Network {
		fn name(&self) -> &'static str {
			NAME
		}

		fn methods(&self) -> &'static [MethodId] {
			&[SNAPSHOT]
		}

		fn invoke(&mut self, method: MethodId, payload: Bytes) -> Result<Bytes, Error> {
			if method != SNAPSHOT {
				return Err(Error::Unavailable);
			}

			if !payload.is_empty() {
				return Err(Error::InvalidArgument);
			}

			snapshot()?.encode()
		}
	}

	#[cfg(test)]
	mod tests {
		use std::net::Ipv4Addr;

		use super::*;

		/// A private LAN address, reachable only if a local interface carries
		/// it.
		const LAN: Ipv4Addr = Ipv4Addr::new(192, 168, 1, 2);
		/// The address a failed DHCP lease leaves behind.
		const APIPA: Ipv4Addr = Ipv4Addr::new(169, 254, 10, 2);

		fn interface(ip: Ipv4Addr, oper_status: if_addrs::IfOperStatus) -> if_addrs::Interface {
			if_addrs::Interface {
				name: "test".into(),
				addr: if_addrs::IfAddr::V4(if_addrs::Ifv4Addr {
					ip,
					netmask: Ipv4Addr::new(255, 255, 255, 0),
					prefixlen: 24,
					broadcast: None,
				}),
				index: None,
				oper_status,
				is_p2p: false,
				#[cfg(windows)]
				adapter_name: "test".into(),
			}
		}

		#[test]
		fn local_interfaces_do_not_become_internet_or_invented_transport() {
			let loopback = interface(Ipv4Addr::LOCALHOST, if_addrs::IfOperStatus::Up);
			assert_eq!(from_interfaces(&[loopback]).connected, Some(false));
			let link = interface(LAN, if_addrs::IfOperStatus::Up);
			let reading = from_interfaces(&[link]);
			assert_eq!(reading.connected, Some(true));
			assert_eq!(reading.connection, Connection::Unknown);
			assert_eq!(reading.internet_validated, None);
			let unknown = interface(LAN, if_addrs::IfOperStatus::Unknown);

			assert_eq!(from_interfaces(&[unknown]).connected, None);
			let apipa = interface(APIPA, if_addrs::IfOperStatus::Up);
			assert_eq!(from_interfaces(&[apipa]).connected, Some(false));
			let mut link_local = interface(Ipv4Addr::LOCALHOST, if_addrs::IfOperStatus::Up);
			link_local.addr = if_addrs::IfAddr::V6(if_addrs::Ifv6Addr {
				ip: "fe80::1".parse().unwrap(),
				netmask: "ffff:ffff:ffff:ffff::".parse().unwrap(),
				prefixlen: 64,
				broadcast: None,
			});
			assert_eq!(from_interfaces(&[link_local]).connected, Some(false));
		}
	}
}

/// Reactive network state over this plugin's snapshot sources.
#[cfg(feature = "hooks")]
pub mod signals;

/// Browser connection hints. Unsupported optional properties remain unknown.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web {
	use std::cell::RefCell;
	use std::rc::Rc;

	use futures::channel::oneshot;
	use wasm_bindgen::JsCast;
	use wasm_bindgen::JsValue;

	use super::*;
	fn property(value: &JsValue, key: &str) -> Result<JsValue, Error> {
		js_sys::Reflect::get(value, &JsValue::from_str(key)).map_err(|_| Error::Native)
	}
	fn estimate(value: &JsValue, key: &str, scale: f64) -> Result<Option<u32>, Error> {
		let number = property(value, key)?;

		if number.is_undefined() || number.is_null() {
			return Ok(None);
		}

		let number = number.as_f64().ok_or(Error::Native)? * scale;

		if !number.is_finite() || number < 0.0 || number.round() >= f64::from(u32::MAX) {
			return Err(Error::Native);
		}

		Ok(Some(number.round() as u32))
	}
	/// `navigator.onLine` is an unreliable hint. Never gate application
	/// features solely on it; attempt the actual request and handle its
	/// result.
	pub fn snapshot() -> Result<Snapshot, Error> {
		let navigator = web_sys::window().ok_or(Error::Unavailable)?.navigator();
		let online = navigator.on_line();
		let mut result = Snapshot::connection(
			if online {
				Connection::Unknown
			} else {
				Connection::None
			},
			Some(online),
		);
		let connection = property(navigator.as_ref(), "connection")?;

		if connection.is_null() || connection.is_undefined() {
			return Ok(result);
		}

		if online {
			result.connection = match property(&connection, "type")?.as_string().as_deref() {
				Some("wifi") => Connection::Wifi,
				Some("cellular") => Connection::Cellular,
				Some("ethernet") => Connection::Ethernet,
				Some("bluetooth") => Connection::Bluetooth,
				Some("other") => Connection::Other,
				_ => Connection::Unknown,
			};
		}

		result.data_saver = property(&connection, "saveData")?.as_bool();

		result.downlink_kbps = estimate(&connection, "downlink", 1000.0)?;
		result.round_trip_ms = estimate(&connection, "rtt", 1.0)?;

		result.effective_type = match property(&connection, "effectiveType")?
			.as_string()
			.as_deref()
		{
			Some("slow-2g") => EffectiveType::Slow2g,
			Some("2g") => EffectiveType::TwoG,
			Some("3g") => EffectiveType::ThreeG,
			Some("4g") => EffectiveType::FourG,
			_ => EffectiveType::Unknown,
		};
		Ok(result)
	}

	/// Answers the first differing reading; later events find the slot empty.
	fn notify(
		sender: &RefCell<Option<oneshot::Sender<Result<Snapshot, Error>>>>,
		previous: Snapshot,
	) {
		let current = snapshot();

		if current == Ok(previous) {
			return;
		}

		// Cancellation can drop the receiver before an already dispatched
		// browser callback finishes.
		if let Some(sender) = sender.borrow_mut().take() {
			let _ = sender.send(current);
		}
	}

	/// Waits for a differing reading, listening to online/offline and optional
	/// connection changes. Dropping this future unregisters all listeners.
	/// Events are coalesced into one result; no unbounded event queue is used.
	pub async fn wait_for_change(previous: Snapshot) -> Result<Snapshot, Error> {
		let window = web_sys::window().ok_or(Error::Unavailable)?;
		let (sender, receiver) = oneshot::channel();
		let sender = Rc::new(RefCell::new(Some(sender)));
		let changed = {
			let sender = Rc::clone(&sender);
			move || notify(&sender, previous)
		};
		let mut listeners = Vec::with_capacity(3);

		for event in ["online", "offline"] {
			let changed = changed.clone();

			listeners.push(gloo_events::EventListener::new(&window, event, move |_| {
				changed();
			}));
		}

		let connection = property(window.navigator().as_ref(), "connection")?;

		if !connection.is_undefined() && !connection.is_null() {
			let target = connection
				.dyn_into::<web_sys::EventTarget>()
				.map_err(|_| Error::Native)?;
			let changed = changed.clone();
			listeners.push(gloo_events::EventListener::new(
				&target,
				"change",
				move |_| changed(),
			));
		}

		// Install before sampling to avoid missing a change between them.
		changed();
		let result = receiver.await.map_err(|_| Error::Cancelled)?;
		drop(listeners);
		result
	}
}

/// Cached guest client using the binary bridge.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use super::*;
	/// Resolved network plugin.
	#[derive(Debug, Clone, Copy)]
	pub struct Network(leptoskit_bridge::guest::Client);
	impl Network {
		/// Resolves an installed host plugin once.
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		/// Reads current network hints.
		pub async fn snapshot(self) -> Result<Snapshot, Error> {
			Snapshot::decode(&self.0.call(SNAPSHOT, &[])?.await?)
		}

		/// Waits for a changed snapshot. Intermediate states can coalesce; this
		/// is a latest-state observation, not a durable transition log.
		pub async fn wait_for_change(self, previous: Snapshot) -> Result<Snapshot, Error> {
			Snapshot::decode(&self.0.call(WAIT_FOR_CHANGE, &previous.encode()?)?.await?)
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn binary_snapshot_preserves_unknowns_and_rejects_invalid_fields() {
		// Wifi, connected, and an unknown validation flag, little endian.
		const WIFI_PREFIX: [u8; 12] = [2, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0];

		let reading = Snapshot::connection(Connection::Wifi, Some(true));
		let bytes = reading.encode().unwrap();
		assert_eq!(bytes.len(), SNAPSHOT_BYTES);
		assert_eq!(bytes[..WIFI_PREFIX.len()], WIFI_PREFIX);
		assert_eq!(Snapshot::decode(&bytes), Ok(reading));

		for field in [0, 4, 8, 12, 16, 28] {
			let mut invalid = bytes.to_vec();
			invalid[field] = 99;
			assert_eq!(Snapshot::decode(&invalid), Err(Error::InvalidArgument));
		}

		assert_eq!(Snapshot::decode(&bytes[..31]), Err(Error::InvalidArgument));
		let mut trailing = bytes.to_vec();
		trailing.push(0);
		assert_eq!(Snapshot::decode(&trailing), Err(Error::InvalidArgument));
		let mut invalid = reading;
		invalid.downlink_kbps = Some(u32::MAX);
		assert_eq!(invalid.encode(), Err(Error::InvalidArgument));
	}
	#[test]
	fn connected_does_not_imply_internet_validated() {
		let reading = Snapshot::connection(Connection::Ethernet, Some(true));

		assert_eq!(reading.internet_validated, None);
	}

	#[test]
	fn every_connection_flag_and_estimate_round_trips() {
		let connections = [
			Connection::Unknown,
			Connection::None,
			Connection::Wifi,
			Connection::Cellular,
			Connection::Ethernet,
			Connection::Bluetooth,
			Connection::Vpn,
			Connection::Other,
		];
		let estimates = [
			EffectiveType::Unknown,
			EffectiveType::Slow2g,
			EffectiveType::TwoG,
			EffectiveType::ThreeG,
			EffectiveType::FourG,
		];
		/// An estimate the browser can report, in kilobits per second.
		const DOWNLINK_KBPS: u32 = 10_000;
		/// An estimate the browser can report, in milliseconds.
		const ROUND_TRIP_MS: u32 = 40;

		for (index, connection) in connections.into_iter().enumerate() {
			let reading = Snapshot {
				connection,
				connected: [None, Some(false), Some(true)][index % 3],
				internet_validated: [Some(true), None, Some(false)][index % 3],
				expensive: [Some(false), Some(true), None][index % 3],
				data_saver: Some(index % 2 == 0),
				downlink_kbps: (index % 2 == 0).then_some(DOWNLINK_KBPS),
				round_trip_ms: (index % 2 == 1).then_some(ROUND_TRIP_MS),
				effective_type: estimates[index % estimates.len()],
			};
			assert_eq!(Snapshot::decode(&reading.encode().unwrap()), Ok(reading));
		}
	}

	#[cfg(any(
		target_os = "macos",
		target_os = "windows",
		all(target_os = "linux", not(target_env = "ohos"))
	))]
	#[test]
	fn desktop_adapter_reports_a_valid_snapshot_and_rejects_bad_calls() {
		use leptoskit_bridge::Plugin;
		const UNKNOWN_METHOD: MethodId = MethodId(9);
		let mut network = desktop::Network;
		assert_eq!(network.name(), NAME);
		assert_eq!(network.methods(), &[SNAPSHOT]);
		let reading = Snapshot::decode(&network.invoke(SNAPSHOT, Bytes::new()).unwrap()).unwrap();
		assert_eq!(reading, desktop::snapshot().unwrap());
		assert_eq!(
			reading.internet_validated, None,
			"desktops never validate reachability"
		);
		assert_eq!(
			network.invoke(SNAPSHOT, Bytes::from_static(&[0])),
			Err(Error::InvalidArgument)
		);

		assert_eq!(
			network.invoke(UNKNOWN_METHOD, Bytes::new()),
			Err(Error::Unavailable)
		);
	}
}
