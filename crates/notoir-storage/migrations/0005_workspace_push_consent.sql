-- Explicit Auto Push choices made by the local user. Repository files cannot
-- grant this consent or affect another workspace's choice.
CREATE TABLE workspace_push_consent (
    workspace_key TEXT PRIMARY KEY,
    enabled INTEGER NOT NULL CHECK (enabled IN (0, 1))
);
