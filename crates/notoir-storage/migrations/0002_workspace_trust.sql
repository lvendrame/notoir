-- Workspaces the user trusted (docs/specs/07 §Workspace trust), keyed by the
-- canonical path key.
CREATE TABLE workspace_trust (
    workspace_key TEXT PRIMARY KEY,
    trusted_at INTEGER NOT NULL
);
