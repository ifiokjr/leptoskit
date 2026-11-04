CREATE TABLE IF NOT EXISTS hosted_accounts (
	id TEXT PRIMARY KEY,
	key_hash TEXT NOT NULL UNIQUE,
	customer TEXT NOT NULL UNIQUE,
	status TEXT NOT NULL,
	plan TEXT NOT NULL,
	valid_until INTEGER NOT NULL DEFAULT 0,
	checkout_nonce TEXT,
	checkout_plan TEXT,
	checkout_started INTEGER NOT NULL DEFAULT 0,
	checkout_url TEXT,
	checkout_until INTEGER NOT NULL DEFAULT 0,
	subscription TEXT,
	synced_at INTEGER NOT NULL DEFAULT 0,
	managed_enabled INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS hosted_installations (
	id TEXT PRIMARY KEY,
	account TEXT NOT NULL REFERENCES hosted_accounts (id),
	project TEXT NOT NULL,
	environment TEXT NOT NULL,
	ownership TEXT NOT NULL,
	worker_hash TEXT UNIQUE,
	pair_hash TEXT UNIQUE,
	pair_until INTEGER NOT NULL DEFAULT 0,
	last_seen INTEGER NOT NULL DEFAULT 0,
	UNIQUE (account, project)
);

CREATE TABLE IF NOT EXISTS hosted_states (
	account TEXT NOT NULL,
	project TEXT NOT NULL,
	version INTEGER NOT NULL,
	body BLOB NOT NULL,
	created INTEGER NOT NULL,
	PRIMARY KEY (account, project, version)
);

CREATE TABLE IF NOT EXISTS hosted_locks (
	account TEXT NOT NULL,
	project TEXT NOT NULL,
	id TEXT NOT NULL,
	body TEXT NOT NULL,
	PRIMARY KEY (account, project)
);

CREATE TABLE IF NOT EXISTS hosted_billing_events (id TEXT PRIMARY KEY, created INTEGER NOT NULL);

CREATE TABLE IF NOT EXISTS hosted_key_verifier (
	id INTEGER PRIMARY KEY CHECK (id = 1),
	VALUE TEXT NOT NULL
);
