-- docs/specs/23-session-persistence.md, schema v1.
-- Later features (MCP, plugins, workspace trust) add their own migrations.
CREATE TABLE meta (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

CREATE TABLE workspace_session (
  workspace_key TEXT PRIMARY KEY,
  updated_at INTEGER NOT NULL,
  session_json TEXT NOT NULL,
  bookmarks_json TEXT NOT NULL DEFAULT '{}'
);

CREATE TABLE window_state (
  slot INTEGER PRIMARY KEY,
  workspace_key TEXT,
  geometry_json TEXT NOT NULL,
  updated_at INTEGER NOT NULL
);

CREATE TABLE recent (
  kind TEXT NOT NULL CHECK (kind IN ('file', 'workspace')),
  path TEXT NOT NULL,
  display_name TEXT,
  last_opened INTEGER NOT NULL,
  pinned INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (kind, path)
);

CREATE TABLE scratchpad (
  scratch_id TEXT PRIMARY KEY,
  title TEXT NOT NULL,
  language TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  open INTEGER NOT NULL
);

CREATE TABLE palette_recent (
  command_id TEXT PRIMARY KEY,
  used_at INTEGER NOT NULL
);

CREATE TABLE notice_dismissed (
  key TEXT PRIMARY KEY,
  at INTEGER NOT NULL
);
