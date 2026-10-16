-- MCP clients, their permission matrix and the activity log
-- (docs/specs/29-mcp.md, 32-permissions-and-security.md). Tokens are stored
-- only as hashes; activity never holds document contents.
CREATE TABLE mcp_client (
    client_id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    token_hash TEXT NOT NULL,
    created_at INTEGER,
    last_seen INTEGER,
    revoked INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE mcp_grant (
    client_id TEXT NOT NULL,
    capability TEXT NOT NULL,
    decision TEXT NOT NULL,
    PRIMARY KEY (client_id, capability)
);
CREATE TABLE mcp_activity (
    id INTEGER PRIMARY KEY,
    at INTEGER NOT NULL,
    client_id TEXT,
    tool TEXT NOT NULL,
    target TEXT,
    outcome TEXT NOT NULL,
    duration_ms INTEGER
);
CREATE INDEX mcp_activity_at ON mcp_activity (at);
