-- Plugin approvals and per-capability grants (docs/specs/30 §Permissions &
-- Approval, 23). An approval covers one version and permission set.
CREATE TABLE plugin_approval (
    plugin_id TEXT PRIMARY KEY,
    version TEXT NOT NULL,
    permissions_hash TEXT NOT NULL,
    approved_at INTEGER NOT NULL,
    enabled INTEGER NOT NULL
);
CREATE TABLE plugin_grant (
    plugin_id TEXT NOT NULL,
    capability TEXT NOT NULL,
    decision TEXT NOT NULL,
    PRIMARY KEY (plugin_id, capability)
);
