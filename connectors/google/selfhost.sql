CREATE TABLE IF NOT EXISTS handoff (hash TEXT PRIMARY KEY, expires_at INTEGER NOT NULL);
INSERT OR IGNORE INTO user (id, name, email, emailVerified, createdAt, updatedAt)
VALUES ('selfhost-operator', 'Operator', 'operator@localhost.invalid', 1, unixepoch(), unixepoch());
