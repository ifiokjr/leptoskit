CREATE TABLE projects (
	id TEXT PRIMARY KEY,
	environment TEXT NOT NULL,
	root TEXT NOT NULL,
	revision INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE operations (
	id TEXT PRIMARY KEY,
	project TEXT NOT NULL REFERENCES projects (id),
	idempotency TEXT NOT NULL,
	request TEXT NOT NULL,
	revision INTEGER NOT NULL,
	phase TEXT NOT NULL,
	created INTEGER NOT NULL,
	updated INTEGER NOT NULL,
	message TEXT NOT NULL,
	RESULT TEXT NOT NULL DEFAULT 'null',
	UNIQUE (project, idempotency)
);

CREATE UNIQUE INDEX one_active_operation ON operations (project)
WHERE
	phase IN ('queued', 'running');

CREATE TABLE operation_events (
	sequence INTEGER PRIMARY KEY AUTOINCREMENT,
	operation TEXT NOT NULL REFERENCES operations (id) ON DELETE CASCADE,
	phase TEXT NOT NULL,
	message TEXT NOT NULL,
	created INTEGER NOT NULL
);

CREATE TRIGGER operation_created AFTER INSERT ON operations BEGIN
INSERT INTO
	operation_events (operation, phase, message, created)
VALUES
	(new.id, new.phase, new.message, new.updated);

END;

CREATE TRIGGER operation_changed AFTER
UPDATE OF phase,
message ON operations WHEN new.phase != old.phase
OR new.message != old.message BEGIN
INSERT INTO
	operation_events (operation, phase, message, created)
VALUES
	(new.id, new.phase, new.message, new.updated);

END;

CREATE TABLE tokens (
	hash BLOB PRIMARY KEY,
	SCOPE TEXT NOT NULL,
	project TEXT REFERENCES projects (id),
	expires INTEGER NOT NULL,
	session INTEGER NOT NULL DEFAULT 0,
	revoked INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE producers (
	project TEXT NOT NULL REFERENCES projects (id),
	producer TEXT NOT NULL,
	RELEASE TEXT NOT NULL,
	seen INTEGER NOT NULL,
	sample TEXT NOT NULL,
	PRIMARY KEY (project, producer)
);

CREATE TABLE metric_windows (
	project TEXT NOT NULL REFERENCES projects (id),
	producer TEXT NOT NULL,
	RELEASE TEXT NOT NULL,
	series TEXT NOT NULL,
	START INTEGER NOT NULL,
	count INTEGER NOT NULL,
	payload TEXT NOT NULL,
	PRIMARY KEY (project, producer, series, START)
);

CREATE INDEX metric_retention ON metric_windows (project, START);
