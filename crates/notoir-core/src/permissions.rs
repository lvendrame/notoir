//! Permission gate (docs/specs/32-permissions-and-security.md): one place that
//! decides whether an externally originated operation (MCP client, plugin)
//! may run. The app evaluates it on the UI thread for every request; nothing
//! caches its answers, so changes apply to the next (and any waiting) request.

use std::collections::{BTreeMap, HashMap, HashSet};

/// A registered MCP client (`mcp_client.client_id`).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ClientId(pub String);

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Principal {
    McpClient(ClientId),
    Plugin(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Capability {
    // workspace / files
    WorkspaceRead,
    WorkspaceSearch,
    FileCreate,
    FileRename,
    FileDelete,
    FilesystemRead,
    FilesystemWrite,
    FilesystemOutsideWorkspace,
    // documents & editor
    DocumentRead,
    DocumentCreate,
    DocumentModify,
    DocumentSave,
    EditorRead,
    EditorControl,
    // settings
    SettingsRead,
    SettingsModify,
    // git
    GitRead,
    GitCommit,
    GitPush,
    // plugin-only
    Network,
    Process,
    Clipboard,
    ExposeMcpTools,
    // reserved
    /// Running an arbitrary built-in command by id.
    ExecuteCommand(String),
    ModifyPluginConfig,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Decision {
    Allow,
    Deny,
    Ask,
}

impl Decision {
    pub fn as_str(self) -> &'static str {
        match self {
            Decision::Allow => "allow",
            Decision::Deny => "deny",
            Decision::Ask => "ask",
        }
    }

    pub fn parse(s: &str) -> Option<Decision> {
        match s {
            "allow" => Some(Decision::Allow),
            "deny" => Some(Decision::Deny),
            "ask" => Some(Decision::Ask),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GrantScope {
    /// This request only.
    Once,
    /// Until Notoir restarts.
    Session,
    /// Stored with the client's permissions.
    Persistent,
}

/// Risk classes (spec 05 / 32).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiskClass {
    /// Allowed when the capability allows.
    Safe,
    /// Asks unless persistently allowed.
    Sensitive,
    /// Destructive or remote-affecting: denied by default for everyone;
    /// "Always Allow" needs a second confirmation.
    Dangerous,
}

/// The MCP permission matrix rows (spec 32 §Permissions UI): group label key
/// and its capabilities.
pub const MCP_MATRIX: &[(&str, &[Capability])] = &[
    (
        "mcp:group.workspace",
        &[
            Capability::WorkspaceRead,
            Capability::WorkspaceSearch,
            Capability::DocumentRead,
            Capability::DocumentCreate,
            Capability::FileCreate,
            Capability::DocumentModify,
            Capability::DocumentSave,
            Capability::FileRename,
            Capability::FileDelete,
            Capability::FilesystemRead,
            Capability::FilesystemWrite,
            Capability::FilesystemOutsideWorkspace,
        ],
    ),
    (
        "mcp:group.editor",
        &[Capability::EditorRead, Capability::EditorControl],
    ),
    (
        "mcp:group.git",
        &[
            Capability::GitRead,
            Capability::GitCommit,
            Capability::GitPush,
        ],
    ),
    (
        "mcp:group.settings",
        &[Capability::SettingsRead, Capability::SettingsModify],
    ),
];

impl Capability {
    /// Stable key (`document.save`; `command:<id>` for commands).
    pub fn key(&self) -> String {
        let k = match self {
            Capability::WorkspaceRead => "workspace.read",
            Capability::WorkspaceSearch => "workspace.search",
            Capability::FileCreate => "file.create",
            Capability::FileRename => "file.rename",
            Capability::FileDelete => "file.delete",
            Capability::FilesystemRead => "filesystem.read",
            Capability::FilesystemWrite => "filesystem.write",
            Capability::FilesystemOutsideWorkspace => "filesystem.outsideWorkspace",
            Capability::DocumentRead => "document.read",
            Capability::DocumentCreate => "document.create",
            Capability::DocumentModify => "document.modify",
            Capability::DocumentSave => "document.save",
            Capability::EditorRead => "editor.read",
            Capability::EditorControl => "editor.control",
            Capability::SettingsRead => "settings.read",
            Capability::SettingsModify => "settings.modify",
            Capability::GitRead => "git.read",
            Capability::GitCommit => "git.commit",
            Capability::GitPush => "git.push",
            Capability::Network => "network",
            Capability::Process => "process",
            Capability::Clipboard => "clipboard",
            Capability::ExposeMcpTools => "mcp.pluginTools",
            Capability::ModifyPluginConfig => "plugins.modifyConfig",
            Capability::ExecuteCommand(id) => return format!("command:{id}"),
        };
        k.to_owned()
    }

    pub fn from_key(key: &str) -> Option<Capability> {
        if let Some(id) = key.strip_prefix("command:") {
            return Some(Capability::ExecuteCommand(id.to_owned()));
        }
        ALL.iter().find(|c| c.key() == key).cloned()
    }

    /// Translation key of the plain-language name ("Save documents").
    pub fn label_key(&self) -> String {
        match self {
            Capability::ExecuteCommand(_) => "mcp:cap.command".to_owned(),
            c => format!("mcp:cap.{}", c.key().replace('.', "_")),
        }
    }

    pub fn risk(&self) -> RiskClass {
        match self {
            Capability::FileDelete
            | Capability::FilesystemOutsideWorkspace
            | Capability::SettingsModify
            | Capability::GitCommit
            | Capability::GitPush
            | Capability::ExecuteCommand(_)
            | Capability::ModifyPluginConfig => RiskClass::Dangerous,
            Capability::FileCreate
            | Capability::DocumentCreate
            | Capability::DocumentSave
            | Capability::FileRename
            | Capability::FilesystemWrite
            | Capability::Process
            | Capability::Network => RiskClass::Sensitive,
            _ => RiskClass::Safe,
        }
    }

    /// Default for a newly added MCP client (spec 32 §Defaults).
    pub fn mcp_default(&self) -> Decision {
        match self.risk() {
            RiskClass::Dangerous => Decision::Deny,
            RiskClass::Sensitive => match self {
                // Plugin-only capabilities are never offered to MCP clients.
                Capability::Process | Capability::Network => Decision::Deny,
                _ => Decision::Ask,
            },
            RiskClass::Safe => match self {
                Capability::Clipboard | Capability::ExposeMcpTools => Decision::Deny,
                _ => Decision::Allow,
            },
        }
    }

    /// Whether a principal may ever be allowed this capability.
    fn grantable(&self, p: &Principal) -> bool {
        !matches!(
            (p, self),
            (Principal::McpClient(_), Capability::ModifyPluginConfig)
        )
    }
}

/// Every non-parameterised capability (for key lookup).
const ALL: &[Capability] = &[
    Capability::WorkspaceRead,
    Capability::WorkspaceSearch,
    Capability::FileCreate,
    Capability::FileRename,
    Capability::FileDelete,
    Capability::FilesystemRead,
    Capability::FilesystemWrite,
    Capability::FilesystemOutsideWorkspace,
    Capability::DocumentRead,
    Capability::DocumentCreate,
    Capability::DocumentModify,
    Capability::DocumentSave,
    Capability::EditorRead,
    Capability::EditorControl,
    Capability::SettingsRead,
    Capability::SettingsModify,
    Capability::GitRead,
    Capability::GitCommit,
    Capability::GitPush,
    Capability::Network,
    Capability::Process,
    Capability::Clipboard,
    Capability::ExposeMcpTools,
    Capability::ModifyPluginConfig,
];

/// Configured decisions, session grants and revocations.
#[derive(Clone, Debug, Default)]
pub struct PermissionGate {
    /// Per principal: decisions that differ from the defaults (persisted by
    /// the app; "Always Allow" lands here).
    configured: HashMap<Principal, BTreeMap<Capability, Decision>>,
    /// "Allow for Session" grants; never persisted.
    session: HashSet<(Principal, Capability)>,
    revoked: HashSet<Principal>,
}

impl PermissionGate {
    /// Evaluates a request (spec 32 §Evaluation). `in_scope` is false when the
    /// target lies outside what the principal may reach (path boundary,
    /// restricted setting, untrusted workspace).
    pub fn check(&self, p: &Principal, cap: &Capability, in_scope: bool) -> Decision {
        if self.revoked.contains(p) || !in_scope || !cap.grantable(p) {
            return Decision::Deny;
        }
        let configured = self.configured.get(p).and_then(|m| m.get(cap)).copied();
        if configured == Some(Decision::Deny) {
            return Decision::Deny;
        }
        if self.session.contains(&(p.clone(), cap.clone())) {
            return Decision::Allow;
        }
        configured.unwrap_or_else(|| Self::default_for(p, cap))
    }

    /// The matrix value: configured, else the default.
    pub fn decision(&self, p: &Principal, cap: &Capability) -> Decision {
        if !cap.grantable(p) {
            return Decision::Deny;
        }
        self.configured
            .get(p)
            .and_then(|m| m.get(cap))
            .copied()
            .unwrap_or_else(|| Self::default_for(p, cap))
    }

    fn default_for(p: &Principal, cap: &Capability) -> Decision {
        match p {
            Principal::McpClient(_) => cap.mcp_default(),
            // Plugins get only what their approved manifest grants (M22).
            Principal::Plugin(_) => Decision::Deny,
        }
    }

    /// Sets a matrix value. Returns false (and changes nothing) for a
    /// capability the principal can never have.
    pub fn set(&mut self, p: &Principal, cap: Capability, d: Decision) -> bool {
        if !cap.grantable(p) && d != Decision::Deny {
            return false;
        }
        let map = self.configured.entry(p.clone()).or_default();
        if d == Self::default_for(p, &cap) {
            map.remove(&cap);
        } else {
            map.insert(cap.clone(), d);
        }
        if d == Decision::Deny {
            self.session.remove(&(p.clone(), cap));
        }
        true
    }

    /// Records an Allow answer to a prompt. `Once` stores nothing (the caller
    /// runs the waiting request).
    pub fn grant(&mut self, p: &Principal, cap: &Capability, scope: GrantScope) {
        match scope {
            GrantScope::Once => {}
            GrantScope::Session => {
                if cap.grantable(p) {
                    self.session.insert((p.clone(), cap.clone()));
                }
            }
            GrantScope::Persistent => {
                self.set(p, cap.clone(), Decision::Allow);
            }
        }
    }

    /// Back to the defaults (configured values and session grants).
    pub fn reset(&mut self, p: &Principal) {
        self.configured.remove(p);
        self.clear_session(p);
    }

    pub fn session_grants(&self, p: &Principal) -> Vec<Capability> {
        let mut v: Vec<Capability> = self
            .session
            .iter()
            .filter(|(q, _)| q == p)
            .map(|(_, c)| c.clone())
            .collect();
        v.sort();
        v
    }

    pub fn clear_session(&mut self, p: &Principal) {
        self.session.retain(|(q, _)| q != p);
    }

    pub fn revoke(&mut self, p: &Principal) {
        self.revoked.insert(p.clone());
        self.clear_session(p);
    }

    pub fn is_revoked(&self, p: &Principal) -> bool {
        self.revoked.contains(p)
    }

    /// Configured values of a principal (to persist).
    pub fn configured(&self, p: &Principal) -> Vec<(Capability, Decision)> {
        self.configured
            .get(p)
            .map(|m| m.iter().map(|(c, d)| (c.clone(), *d)).collect())
            .unwrap_or_default()
    }

    /// Replaces a principal's configured values (loaded from storage).
    pub fn load(
        &mut self,
        p: &Principal,
        values: impl IntoIterator<Item = (Capability, Decision)>,
    ) {
        self.configured.remove(p);
        for (c, d) in values {
            self.set(p, c, d);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client() -> Principal {
        Principal::McpClient(ClientId("c1".into()))
    }

    #[test]
    fn mcp_defaults_follow_the_spec_table() {
        let gate = PermissionGate::default();
        let p = client();
        let d = |c: Capability| gate.check(&p, &c, true);
        assert_eq!(d(Capability::WorkspaceRead), Decision::Allow);
        assert_eq!(d(Capability::WorkspaceSearch), Decision::Allow);
        assert_eq!(d(Capability::DocumentRead), Decision::Allow);
        assert_eq!(d(Capability::FileCreate), Decision::Ask);
        assert_eq!(d(Capability::DocumentModify), Decision::Allow);
        assert_eq!(d(Capability::DocumentSave), Decision::Ask);
        assert_eq!(d(Capability::FileRename), Decision::Ask);
        assert_eq!(d(Capability::FileDelete), Decision::Deny);
        assert_eq!(d(Capability::FilesystemRead), Decision::Allow);
        assert_eq!(d(Capability::FilesystemWrite), Decision::Ask);
        assert_eq!(d(Capability::FilesystemOutsideWorkspace), Decision::Deny);
        assert_eq!(d(Capability::EditorRead), Decision::Allow);
        assert_eq!(d(Capability::EditorControl), Decision::Allow);
        assert_eq!(d(Capability::GitRead), Decision::Allow);
        assert_eq!(d(Capability::GitCommit), Decision::Deny);
        assert_eq!(d(Capability::GitPush), Decision::Deny);
        assert_eq!(d(Capability::SettingsRead), Decision::Allow);
        assert_eq!(d(Capability::SettingsModify), Decision::Deny);
        assert_eq!(
            d(Capability::ExecuteCommand("file.save".into())),
            Decision::Deny
        );
        assert_eq!(d(Capability::ModifyPluginConfig), Decision::Deny);
    }

    #[test]
    fn evaluation_order() {
        let mut gate = PermissionGate::default();
        let p = client();
        // Out of scope beats an Allow.
        assert_eq!(
            gate.check(&p, &Capability::DocumentRead, false),
            Decision::Deny
        );
        // Session grant → Allow; an explicit Deny beats it.
        gate.grant(&p, &Capability::DocumentSave, GrantScope::Session);
        assert_eq!(
            gate.check(&p, &Capability::DocumentSave, true),
            Decision::Allow
        );
        assert_eq!(gate.session_grants(&p), [Capability::DocumentSave]);
        gate.set(&p, Capability::DocumentSave, Decision::Deny);
        assert_eq!(
            gate.check(&p, &Capability::DocumentSave, true),
            Decision::Deny
        );
        // Persistent grant.
        gate.grant(&p, &Capability::FileRename, GrantScope::Persistent);
        assert_eq!(
            gate.check(&p, &Capability::FileRename, true),
            Decision::Allow
        );
        assert_eq!(
            gate.configured(&p),
            [
                (Capability::FileRename, Decision::Allow),
                (Capability::DocumentSave, Decision::Deny)
            ]
        );
        // Once stores nothing.
        gate.grant(&p, &Capability::FileCreate, GrantScope::Once);
        assert_eq!(gate.check(&p, &Capability::FileCreate, true), Decision::Ask);
        // Revoked → Deny for everything.
        gate.revoke(&p);
        assert_eq!(
            gate.check(&p, &Capability::WorkspaceRead, true),
            Decision::Deny
        );
    }

    // spec 32: "Session grant expires on restart"
    #[test]
    fn session_grants_do_not_survive_a_new_gate() {
        let mut gate = PermissionGate::default();
        let p = client();
        gate.grant(&p, &Capability::FileCreate, GrantScope::Session);
        gate.grant(&p, &Capability::FileRename, GrantScope::Persistent);
        // Restart: only the persisted configuration is loaded again.
        let mut restarted = PermissionGate::default();
        restarted.load(&p, gate.configured(&p));
        assert_eq!(
            restarted.check(&p, &Capability::FileCreate, true),
            Decision::Ask
        );
        assert_eq!(
            restarted.check(&p, &Capability::FileRename, true),
            Decision::Allow
        );
    }

    #[test]
    fn plugin_config_can_never_be_granted_to_mcp() {
        let mut gate = PermissionGate::default();
        let p = client();
        assert!(!gate.set(&p, Capability::ModifyPluginConfig, Decision::Allow));
        gate.grant(&p, &Capability::ModifyPluginConfig, GrantScope::Session);
        assert_eq!(
            gate.check(&p, &Capability::ModifyPluginConfig, true),
            Decision::Deny
        );
    }

    #[test]
    fn reset_and_keys() {
        let mut gate = PermissionGate::default();
        let p = client();
        gate.set(&p, Capability::FileDelete, Decision::Allow);
        gate.reset(&p);
        assert_eq!(gate.decision(&p, &Capability::FileDelete), Decision::Deny);
        for c in ALL {
            assert_eq!(Capability::from_key(&c.key()).as_ref(), Some(c));
        }
        let cmd = Capability::ExecuteCommand("file.save".into());
        assert_eq!(Capability::from_key(&cmd.key()), Some(cmd));
        assert_eq!(
            Capability::DocumentSave.label_key(),
            "mcp:cap.document_save"
        );
    }
}
