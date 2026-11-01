//! `huddle-load`: many simulated people in Huddle at once, over the same
//! wire protocol the apps speak.
//!
//! It signs in `--accounts` teammates through the server's dev sign-in,
//! starts `--channels` channels with all of them in, and opens `--clients`
//! live connections, each watching one channel's messages. Then it sends
//! `--rate` messages a second, round-robin across the channels, for
//! `--seconds`, and measures, for every connection that should receive each
//! message, the time from commit to update: from the message's
//! `created_at`, which the server stamps inside the transaction that
//! commits it, to the moment the connection reads the frame that carries
//! it. Both clocks are this machine's, so run it next to the server.
//!
//! With several `--url`s, one per server, the connections and the messages
//! take turns between them: a message sent to one server reaches the
//! connections on the others through the live relay.
//!
//! ```sh
//! # The server, optimized, with dev sign-in compiled in:
//! CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=true cargo run --release -p huddle
//! # Then, in another terminal (raise the open-file limit first):
//! ulimit -n 20000
//! cargo run --release -p huddle_load -- --clients 500 --channels 10
//! ```

use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;

use clap::Parser;
use futures::SinkExt;
use futures::StreamExt;
use leptoskit_rpc::live::ClientFrame;
use leptoskit_rpc::live::ServerFrame;
use leptoskit_rpc::live::Update;
use serde_json::Value;
use serde_json::json;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

/// Simulated Huddle clients, and how hard they talk.
#[derive(Debug, Parser)]
#[command(version, about)]
struct Options {
	/// The server, or each server: repeat it to spread connections and
	/// messages across several.
	#[arg(long, default_value = "http://127.0.0.1:3000")]
	url: Vec<String>,
	/// Live connections to open.
	#[arg(long, default_value_t = 5_000)]
	clients: usize,
	/// Channels to spread them across.
	#[arg(long, default_value_t = 50)]
	channels: usize,
	/// Distinct people the connections sign in as; each has several open.
	#[arg(long, default_value_t = 100)]
	accounts: usize,
	/// Messages a second, across every channel.
	#[arg(long, default_value_t = 20.0)]
	rate: f64,
	/// How long to send for, in seconds.
	#[arg(long, default_value_t = 30)]
	seconds: u64,
	/// How many connections open at once while starting.
	#[arg(long, default_value_t = 64)]
	connect_concurrency: usize,
}

/// What every connection records.
#[derive(Default)]
struct Totals {
	latencies: Mutex<Vec<f64>>,
	delivered: AtomicU64,
	errors: AtomicU64,
	full_resends: AtomicU64,
}

impl Totals {
	fn error(&self, what: impl std::fmt::Display) {
		let count = self.errors.fetch_add(1, Ordering::Relaxed);

		if count < 10 {
			eprintln!("error: {what}");
		}
	}
}

/// A teammate signed in: their session cookie.
#[derive(Clone)]
struct Account {
	cookie: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
	let options = Options::parse();
	let bases: Vec<String> = options
		.url
		.iter()
		.map(|url| url.trim_end_matches('/').to_owned())
		.collect();
	let base = bases[0].clone();
	let http = reqwest::Client::builder()
		.pool_max_idle_per_host(64)
		.build()?;
	let run = chrono::Utc::now().format("%H%M%S").to_string();

	println!(
		"huddle-load: {} clients, {} channels, {} accounts, {} messages/s for {}s against {}",
		options.clients,
		options.channels,
		options.accounts,
		options.rate,
		options.seconds,
		bases.join(", ")
	);

	let started = Instant::now();
	let accounts = sign_in_all(&http, &base, &run, options.accounts).await?;
	println!(
		"signed in {} accounts in {:.1}s",
		accounts.len(),
		started.elapsed().as_secs_f64()
	);

	let channels = start_channels(&http, &base, &run, &accounts, options.channels).await?;
	println!("started {} channels", channels.len());

	let totals = Arc::new(Totals::default());
	let connected = Instant::now();
	let ready = Arc::new(tokio::sync::Semaphore::new(options.connect_concurrency));
	let (stop, _) = tokio::sync::broadcast::channel::<()>(1);
	let mut clients = Vec::with_capacity(options.clients);

	for index in 0..options.clients {
		let account = accounts[index % accounts.len()].clone();
		let channel = channels[index % channels.len()].clone();
		let base = bases[index % bases.len()].clone();
		let shared = Arc::clone(&totals);
		let permit = Arc::clone(&ready).acquire_owned().await?;
		let (subscribed, done) = tokio::sync::oneshot::channel();
		let stop = stop.subscribe();
		clients.push(tokio::spawn(async move {
			client(&base, &account, &channel, &shared, subscribed, stop).await;
		}));

		match done.await {
			Ok(true) => {}
			_ => totals.error(format!("client {index} didn't subscribe")),
		}

		drop(permit);
	}

	println!(
		"opened {} live connections in {:.1}s",
		clients.len(),
		connected.elapsed().as_secs_f64()
	);

	let per_channel = per_channel(options.clients, channels.len());
	let sent = send_messages(&http, &bases, &accounts, &channels, &options, &totals).await;
	// Let the last messages arrive.
	tokio::time::sleep(Duration::from_secs(5)).await;
	let _ = stop.send(());

	for client in clients {
		let _ = tokio::time::timeout(Duration::from_secs(5), client).await;
	}

	report(&options, &totals, &sent, &per_channel);
	Ok(())
}

/// Signs in `count` teammates through the server's dev sign-in.
async fn sign_in_all(
	http: &reqwest::Client,
	base: &str,
	run: &str,
	count: usize,
) -> Result<Vec<Account>, Box<dyn std::error::Error>> {
	let mut accounts = Vec::with_capacity(count);

	for index in 0..count {
		let response = http
			.post(format!("{base}/auth/dev/sign-in"))
			.header("origin", base)
			.header("content-type", "application/json")
			.body(json!({ "email": format!("load.{run}.{index}@huddle.test") }).to_string())
			.send()
			.await?;

		if !response.status().is_success() {
			return Err(format!(
				"dev sign-in answered {}: run the server as a debug build, or with \
				 CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=true",
				response.status()
			)
			.into());
		}

		let cookie = response
			.headers()
			.get("set-cookie")
			.and_then(|value| value.to_str().ok())
			.and_then(|value| value.split(';').next())
			.ok_or("dev sign-in set no cookie")?
			.to_owned();
		accounts.push(Account { cookie });
	}

	Ok(accounts)
}

async fn call(
	http: &reqwest::Client,
	base: &str,
	account: &Account,
	name: &str,
	args: &Value,
) -> Result<Value, String> {
	let response = http
		.post(format!("{base}/api/{name}"))
		.header("origin", base)
		.header("cookie", &account.cookie)
		.header("content-type", "application/json")
		.body(args.to_string())
		.send()
		.await
		.map_err(|error| error.to_string())?;
	let status = response.status();
	let text = response.text().await.map_err(|error| error.to_string())?;

	if status.is_success() {
		serde_json::from_str(&text).map_err(|error| error.to_string())
	} else {
		Err(format!("{name} answered {status}: {text}"))
	}
}

/// Starts `count` channels, each with every account in it.
async fn start_channels(
	http: &reqwest::Client,
	base: &str,
	run: &str,
	accounts: &[Account],
	count: usize,
) -> Result<Vec<Value>, Box<dyn std::error::Error>> {
	let owner = &accounts[0];
	let mut ids = Vec::new();

	for account in accounts {
		let me = call(http, base, account, "whoami", &json!({})).await?;
		ids.push(me["id"].clone());
	}

	let mut channels = Vec::with_capacity(count);

	for index in 0..count {
		let channel = call(
			http,
			base,
			owner,
			"create_channel",
			&json!({ "name": format!("load-{run}-{index}"), "private": true, "members": ids }),
		)
		.await?;
		channels.push(channel);
	}

	Ok(channels)
}

/// One simulated client: a live connection watching one channel's messages,
/// recording when each new message arrives.
async fn client(
	base: &str,
	account: &Account,
	channel: &Value,
	totals: &Totals,
	subscribed: tokio::sync::oneshot::Sender<bool>,
	mut stop: tokio::sync::broadcast::Receiver<()>,
) {
	let url = format!("{}/live", base.replace("http://", "ws://"));
	let Ok(mut request) = url.into_client_request() else {
		let _ = subscribed.send(false);
		return;
	};
	let Ok(cookie) = account.cookie.parse() else {
		let _ = subscribed.send(false);
		return;
	};
	request.headers_mut().insert("cookie", cookie);
	let mut socket = match tokio_tungstenite::connect_async(request).await {
		Ok((socket, _)) => socket,
		Err(error) => {
			totals.error(format!("connect: {error}"));
			let _ = subscribed.send(false);
			return;
		}
	};
	let sub = ClientFrame::Sub {
		id: 1,
		query: "messages".into(),
		args: json!({ "channel": channel, "limit": 20 }),
	};
	let Ok(text) = serde_json::to_string(&sub) else {
		let _ = subscribed.send(false);
		return;
	};

	if socket.send(Message::Text(text.into())).await.is_err() {
		let _ = subscribed.send(false);
		return;
	}

	let mut seen: HashSet<String> = HashSet::new();
	let mut subscribed = Some(subscribed);

	loop {
		let message = tokio::select! {
			message = socket.next() => message,
			_ = stop.recv() => break,
		};
		let Some(Ok(message)) = message else {
			totals.error("a live connection closed early");
			break;
		};
		let Message::Text(text) = message else {
			continue;
		};
		let Ok(frame) = serde_json::from_str::<ServerFrame>(&text) else {
			totals.error("a frame didn't parse");
			continue;
		};
		let arrived = chrono::Utc::now();

		match frame {
			ServerFrame::Done {
				request: 1,
				ok,
				err,
				..
			} => {
				if let Some(error) = err {
					totals.error(format!("subscribe: {error}"));
				}

				for row in ok.as_ref().and_then(Value::as_array).into_iter().flatten() {
					if let Some(id) = row["id"].as_str() {
						seen.insert(id.to_owned());
					}
				}

				if let Some(subscribed) = subscribed.take() {
					let _ = subscribed.send(true);
				}
			}
			ServerFrame::Transition(transition) => {
				if transition.full {
					totals.full_resends.fetch_add(1, Ordering::Relaxed);
				}

				for update in transition.updates {
					let rows = match update.update {
						Update::Ok(value) => value.as_array().cloned().unwrap_or_default(),
						Update::Patch(patch) => {
							patch
								.upsert
								.into_iter()
								.map(|upsert| upsert.value)
								.collect()
						}
						Update::Err(error) => {
							totals.error(format!("update: {error}"));
							Vec::new()
						}
					};
					record(&rows, &mut seen, arrived, totals);
				}
			}
			_ => {}
		}
	}

	if let Some(subscribed) = subscribed.take() {
		let _ = subscribed.send(false);
	}
}

/// Records how long each row this connection hasn't seen took to arrive.
fn record(
	rows: &[Value],
	seen: &mut HashSet<String>,
	arrived: chrono::DateTime<chrono::Utc>,
	totals: &Totals,
) {
	for row in rows {
		let Some(id) = row["id"].as_str() else {
			continue;
		};

		if !seen.insert(id.to_owned()) {
			continue;
		}

		let Some(sent) = row["sent_at"]
			.as_str()
			.and_then(|sent| chrono::DateTime::parse_from_rfc3339(sent).ok())
		else {
			continue;
		};
		#[expect(
			clippy::cast_precision_loss,
			reason = "latencies are microseconds, far below f64's integer limit"
		)]
		let latency = (arrived - sent.with_timezone(&chrono::Utc))
			.num_microseconds()
			.unwrap_or_default() as f64
			/ 1_000.0;
		totals.delivered.fetch_add(1, Ordering::Relaxed);
		totals
			.latencies
			.lock()
			.unwrap_or_else(std::sync::PoisonError::into_inner)
			.push(latency.max(0.0));
	}
}

/// How many connections watch each channel.
fn per_channel(clients: usize, channels: usize) -> Vec<u64> {
	(0..channels)
		.map(|channel| {
			let count = (0..clients)
				.filter(|index| index % channels == channel)
				.count();
			u64::try_from(count).unwrap_or(u64::MAX)
		})
		.collect()
}

/// Sends messages at the rate asked for, round-robin across the channels,
/// and returns how many went to each.
async fn send_messages(
	http: &reqwest::Client,
	bases: &[String],
	accounts: &[Account],
	channels: &[Value],
	options: &Options,
	totals: &Arc<Totals>,
) -> HashMap<usize, u64> {
	let interval = Duration::from_secs_f64(1.0 / options.rate.max(0.001));
	let mut ticker = tokio::time::interval(interval);
	ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
	let deadline = Instant::now() + Duration::from_secs(options.seconds);
	let sent = Arc::new(Mutex::new(HashMap::new()));
	let mut in_flight = Vec::new();
	let mut sequence = 0_usize;

	while Instant::now() < deadline {
		ticker.tick().await;
		let channel_index = sequence % channels.len();
		let account = accounts[sequence % accounts.len()].clone();
		let channel = channels[channel_index].clone();
		let http = http.clone();
		// The next server along, so messages come in through every one.
		let base = bases[(sequence / channels.len()) % bases.len()].clone();
		let totals = Arc::clone(totals);
		let sent = Arc::clone(&sent);
		let key = format!("load-{sequence}");
		in_flight.push(tokio::spawn(async move {
			let args =
				json!({ "channel": channel, "body": format!("message {key}"), "client_key": key });

			match call(&http, &base, &account, "send_message", &args).await {
				Ok(_) => {
					*sent
						.lock()
						.unwrap_or_else(std::sync::PoisonError::into_inner)
						.entry(channel_index)
						.or_insert(0) += 1;
				}
				Err(error) => totals.error(error),
			}
		}));
		sequence += 1;
	}

	for task in in_flight {
		let _ = task.await;
	}

	sent.lock()
		.unwrap_or_else(std::sync::PoisonError::into_inner)
		.clone()
}

fn percentile(sorted: &[f64], fraction: f64) -> f64 {
	if sorted.is_empty() {
		return f64::NAN;
	}

	#[expect(
		clippy::cast_possible_truncation,
		clippy::cast_sign_loss,
		clippy::cast_precision_loss,
		reason = "an index into a list of samples"
	)]
	let index = ((sorted.len() as f64 - 1.0) * fraction).round() as usize;
	sorted[index.min(sorted.len() - 1)]
}

fn report(options: &Options, totals: &Totals, sent: &HashMap<usize, u64>, per_channel: &[u64]) {
	let mut latencies = totals
		.latencies
		.lock()
		.unwrap_or_else(std::sync::PoisonError::into_inner)
		.clone();
	latencies.sort_by(f64::total_cmp);
	let messages: u64 = sent.values().sum();
	let expected: u64 = sent
		.iter()
		.map(|(channel, count)| count * per_channel.get(*channel).copied().unwrap_or(0))
		.sum();
	let delivered = totals.delivered.load(Ordering::Relaxed);

	println!();
	println!("messages sent      {messages}");
	println!("updates expected   {expected}");
	println!("updates received   {delivered}");
	println!(
		"full resends       {}",
		totals.full_resends.load(Ordering::Relaxed)
	);
	println!(
		"errors             {}",
		totals.errors.load(Ordering::Relaxed)
	);
	println!(
		"commit → update    p50 {:.1} ms · p95 {:.1} ms · p99 {:.1} ms · max {:.1} ms",
		percentile(&latencies, 0.50),
		percentile(&latencies, 0.95),
		percentile(&latencies, 0.99),
		latencies.last().copied().unwrap_or(f64::NAN)
	);
	println!(
		"({} clients across {} channels, {} messages/s for {}s)",
		options.clients, options.channels, options.rate, options.seconds
	);
}
