//! Keybindings (docs/specs/05-command-system-and-keybindings.md §Keybindings).
//!
//! Bindings are written with the `Mod` modifier (⌘ on macOS, Ctrl elsewhere);
//! chords have one or two keystrokes (`Mod+K Mod+O`). User bindings override
//! plugin bindings, which override defaults; a user entry whose command starts
//! with `-` removes matching bindings.

use std::fmt;

use serde_json::Value;

use crate::command::CommandId;
use crate::when::{ContextSnapshot, When, WhenError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Mac,
    Windows,
    Linux,
}

impl Platform {
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            Platform::Mac
        } else if cfg!(target_os = "windows") {
            Platform::Windows
        } else {
            Platform::Linux
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Modifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// ⌘ on macOS, Windows/Super key elsewhere.
    pub cmd: bool,
}

/// One key press: modifiers plus a normalized key name (`S`, `F3`, `Enter`,
/// `Up`, `+`, `\\`…).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Keystroke {
    pub mods: Modifiers,
    pub key: String,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum KeyParseError {
    #[error("empty key in {0:?}")]
    Empty(String),
    #[error("unknown modifier {0:?}")]
    UnknownModifier(String),
    #[error("chords have at most two keystrokes: {0:?}")]
    TooLong(String),
}

const NAMED: &[&str] = &[
    "Enter",
    "Escape",
    "Tab",
    "Space",
    "Backspace",
    "Delete",
    "Insert",
    "Up",
    "Down",
    "Left",
    "Right",
    "Home",
    "End",
    "PageUp",
    "PageDown",
];

fn normalize_key(key: &str) -> Option<String> {
    if key.is_empty() {
        return None;
    }
    let lower = key.to_ascii_lowercase();
    let alias = match lower.as_str() {
        "esc" => Some("Escape"),
        "return" => Some("Enter"),
        "del" => Some("Delete"),
        "arrowup" | "↑" => Some("Up"),
        "arrowdown" | "↓" => Some("Down"),
        "arrowleft" | "←" => Some("Left"),
        "arrowright" | "→" => Some("Right"),
        "pgup" => Some("PageUp"),
        "pgdn" => Some("PageDown"),
        _ => None,
    };
    if let Some(a) = alias {
        return Some(a.to_owned());
    }
    if let Some(n) = NAMED.iter().find(|n| n.eq_ignore_ascii_case(key)) {
        return Some((*n).to_owned());
    }
    if lower.len() >= 2
        && lower.starts_with('f')
        && lower[1..]
            .parse::<u8>()
            .is_ok_and(|n| (1..=24).contains(&n))
    {
        return Some(lower.to_ascii_uppercase());
    }
    let mut chars = key.chars();
    let c = chars.next()?;
    chars.next().is_none().then(|| c.to_uppercase().collect())
}

impl Keystroke {
    pub fn new(mods: Modifiers, key: &str) -> Option<Self> {
        Some(Self {
            mods,
            key: normalize_key(key)?,
        })
    }

    /// Parses `Mod+Shift+S`, `Ctrl++`, `Alt+Up`.
    pub fn parse(s: &str, platform: Platform) -> Result<Self, KeyParseError> {
        let (mods_part, key) = if let Some(rest) = s.strip_suffix("++") {
            (rest, "+")
        } else if s == "+" {
            ("", "+")
        } else {
            match s.rsplit_once('+') {
                Some((m, k)) => (m, k),
                None => ("", s),
            }
        };
        let mut mods = Modifiers::default();
        for m in mods_part.split('+').filter(|m| !m.is_empty()) {
            match m.to_ascii_lowercase().as_str() {
                "mod" => {
                    if platform == Platform::Mac {
                        mods.cmd = true;
                    } else {
                        mods.ctrl = true;
                    }
                }
                "ctrl" | "control" => mods.ctrl = true,
                "alt" | "option" | "opt" => mods.alt = true,
                "shift" => mods.shift = true,
                "cmd" | "command" | "meta" | "super" | "win" => mods.cmd = true,
                _ => return Err(KeyParseError::UnknownModifier(m.to_owned())),
            }
        }
        let key = normalize_key(key).ok_or_else(|| KeyParseError::Empty(s.to_owned()))?;
        Ok(Self { mods, key })
    }

    /// `=` and `+` are the same physical key for zoom-style bindings.
    fn matches(&self, other: &Keystroke) -> bool {
        let canon = |k: &str| {
            if k == "=" {
                "+".to_owned()
            } else {
                k.to_owned()
            }
        };
        self.mods == other.mods && canon(&self.key) == canon(&other.key)
    }

    pub fn display(&self, platform: Platform) -> String {
        let key = match (platform, self.key.as_str()) {
            (Platform::Mac, "Up") => "↑",
            (Platform::Mac, "Down") => "↓",
            (Platform::Mac, "Left") => "←",
            (Platform::Mac, "Right") => "→",
            (Platform::Mac, "Enter") => "↩",
            (Platform::Mac, "Backspace") => "⌫",
            (Platform::Mac, "Escape") => "⎋",
            (_, k) => k,
        };
        if platform == Platform::Mac {
            let mut s = String::new();
            for (on, sym) in [
                (self.mods.ctrl, "⌃"),
                (self.mods.alt, "⌥"),
                (self.mods.shift, "⇧"),
                (self.mods.cmd, "⌘"),
            ] {
                if on {
                    s.push_str(sym);
                }
            }
            s.push_str(key);
            s
        } else {
            let meta = if platform == Platform::Windows {
                "Win"
            } else {
                "Super"
            };
            let mut parts = Vec::new();
            for (on, name) in [
                (self.mods.ctrl, "Ctrl"),
                (self.mods.alt, "Alt"),
                (self.mods.shift, "Shift"),
                (self.mods.cmd, meta),
            ] {
                if on {
                    parts.push(name);
                }
            }
            parts.push(key);
            parts.join("+")
        }
    }
}

/// One or two keystrokes.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct KeyChord(Vec<Keystroke>);

impl KeyChord {
    pub fn parse(s: &str, platform: Platform) -> Result<Self, KeyParseError> {
        let strokes: Vec<Keystroke> = s
            .split_whitespace()
            .map(|p| Keystroke::parse(p, platform))
            .collect::<Result<_, _>>()?;
        match strokes.len() {
            0 => Err(KeyParseError::Empty(s.to_owned())),
            1 | 2 => Ok(Self(strokes)),
            _ => Err(KeyParseError::TooLong(s.to_owned())),
        }
    }

    pub fn strokes(&self) -> &[Keystroke] {
        &self.0
    }

    fn matches(&self, strokes: &[Keystroke]) -> bool {
        self.0.len() == strokes.len() && self.0.iter().zip(strokes).all(|(a, b)| a.matches(b))
    }

    pub fn display(&self, platform: Platform) -> String {
        self.0
            .iter()
            .map(|k| k.display(platform))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

impl fmt::Display for KeyChord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.display(Platform::current()))
    }
}

/// A default binding declared on a command spec.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyBindingDef {
    /// Chord using `Mod`; used on every platform unless overridden.
    pub key: &'static str,
    /// macOS-specific chord.
    pub mac: Option<&'static str>,
    pub when: &'static str,
}

impl KeyBindingDef {
    pub const fn new(key: &'static str) -> Self {
        Self {
            key,
            mac: None,
            when: "",
        }
    }

    pub const fn mac(mut self, mac: &'static str) -> Self {
        self.mac = Some(mac);
        self
    }

    pub const fn when(mut self, when: &'static str) -> Self {
        self.when = when;
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindingSource {
    Default,
    Plugin,
    User,
}

#[derive(Clone, Debug, PartialEq)]
pub struct KeyBinding {
    pub chord: KeyChord,
    pub command: CommandId,
    pub args: Value,
    pub when: When,
    pub source: BindingSource,
}

impl KeyBinding {
    pub fn from_def(
        def: &KeyBindingDef,
        command: CommandId,
        platform: Platform,
    ) -> Result<Self, KeymapError> {
        let key = match (platform, def.mac) {
            (Platform::Mac, Some(mac)) => mac,
            _ => def.key,
        };
        Ok(Self {
            chord: KeyChord::parse(key, platform)?,
            command,
            args: Value::Null,
            when: When::parse(def.when)?,
            source: BindingSource::Default,
        })
    }
}

/// An entry of `keybindings.toml`.
#[derive(Clone, Debug, PartialEq)]
pub struct UserBinding {
    pub key: Option<String>,
    /// `-command.id` removes bindings.
    pub command: String,
    pub args: Option<Value>,
    pub when: Option<String>,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum KeymapError {
    #[error(transparent)]
    Key(#[from] KeyParseError),
    #[error(transparent)]
    When(#[from] WhenError),
    #[error("binding for {0} has no key")]
    MissingKey(String),
}

/// Result of feeding one keystroke.
#[derive(Clone, Debug, PartialEq)]
pub enum KeyResult {
    Command {
        command: CommandId,
        args: Value,
    },
    /// First stroke of a chord; show "(Mod+K) was pressed. Waiting for second key…".
    Pending(Keystroke),
    NoMatch,
}

#[derive(Clone, Debug, Default)]
pub struct Keymap {
    /// Highest priority first.
    bindings: Vec<KeyBinding>,
    pending: Option<Keystroke>,
}

impl Keymap {
    /// Builds the effective keymap. Invalid user entries are skipped and
    /// reported.
    pub fn build(
        defaults: Vec<KeyBinding>,
        plugins: Vec<KeyBinding>,
        user: &[UserBinding],
        platform: Platform,
    ) -> (Keymap, Vec<KeymapError>) {
        let mut bindings: Vec<KeyBinding> = plugins.into_iter().chain(defaults).collect();
        let mut errors = Vec::new();
        for entry in user {
            if let Some(removed) = entry.command.strip_prefix('-') {
                let chord = match entry.key.as_deref().map(|k| KeyChord::parse(k, platform)) {
                    Some(Ok(c)) => Some(c),
                    Some(Err(e)) => {
                        errors.push(e.into());
                        continue;
                    }
                    None => None,
                };
                bindings.retain(|b| {
                    b.command.as_str() != removed || chord.as_ref().is_some_and(|c| *c != b.chord)
                });
                continue;
            }
            let parsed = (|| -> Result<KeyBinding, KeymapError> {
                let key = entry
                    .key
                    .as_deref()
                    .ok_or_else(|| KeymapError::MissingKey(entry.command.clone()))?;
                Ok(KeyBinding {
                    chord: KeyChord::parse(key, platform)?,
                    command: CommandId::from(entry.command.clone()),
                    args: entry.args.clone().unwrap_or(Value::Null),
                    when: When::parse(entry.when.as_deref().unwrap_or(""))?,
                    source: BindingSource::User,
                })
            })();
            match parsed {
                Ok(b) => bindings.insert(0, b),
                Err(e) => errors.push(e),
            }
        }
        (
            Keymap {
                bindings,
                pending: None,
            },
            errors,
        )
    }

    pub fn pending(&self) -> Option<&Keystroke> {
        self.pending.as_ref()
    }

    pub fn cancel_pending(&mut self) {
        self.pending = None;
    }

    /// Feeds one keystroke.
    pub fn press(&mut self, stroke: Keystroke, ctx: &ContextSnapshot) -> KeyResult {
        if let Some(first) = self.pending.take() {
            let seq = [first, stroke];
            return self
                .find(&seq, ctx)
                .map_or(KeyResult::NoMatch, |b| KeyResult::Command {
                    command: b.command.clone(),
                    args: b.args.clone(),
                });
        }
        let single = [stroke.clone()];
        if let Some(b) = self.find(&single, ctx) {
            return KeyResult::Command {
                command: b.command.clone(),
                args: b.args.clone(),
            };
        }
        let starts_chord = self
            .bindings
            .iter()
            .any(|b| b.chord.0.len() == 2 && b.chord.0[0].matches(&stroke) && b.when.eval(ctx));
        if starts_chord {
            self.pending = Some(stroke.clone());
            return KeyResult::Pending(stroke);
        }
        KeyResult::NoMatch
    }

    fn find(&self, strokes: &[Keystroke], ctx: &ContextSnapshot) -> Option<&KeyBinding> {
        self.bindings
            .iter()
            .find(|b| b.chord.matches(strokes) && b.when.eval(ctx))
    }

    /// Chords bound to `command`, highest priority first (menu/palette display).
    pub fn chords_for(&self, command: &CommandId) -> Vec<&KeyChord> {
        self.bindings
            .iter()
            .filter(|b| &b.command == command)
            .map(|b| &b.chord)
            .collect()
    }

    /// Pairs of bindings sharing a chord and `when` (shown in the shortcuts
    /// editor with a warning).
    pub fn conflicts(&self) -> Vec<(&KeyBinding, &KeyBinding)> {
        let mut out = Vec::new();
        for (i, a) in self.bindings.iter().enumerate() {
            for b in &self.bindings[i + 1..] {
                if a.chord == b.chord && a.when == b.when && a.command != b.command {
                    out.push((a, b));
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ks(s: &str, p: Platform) -> Keystroke {
        Keystroke::parse(s, p).unwrap()
    }

    fn defaults(p: Platform) -> Vec<KeyBinding> {
        [
            ("file.save", KeyBindingDef::new("Mod+S")),
            ("file.saveAs", KeyBindingDef::new("Mod+Shift+S")),
            ("edit.deleteLine", KeyBindingDef::new("Mod+Shift+K")),
            ("view.zenMode", KeyBindingDef::new("Mod+K Z")),
            ("file.openWorkspace", KeyBindingDef::new("Mod+K Mod+O")),
            ("view.zoomIn", KeyBindingDef::new("Mod++")),
            ("edit.replace", KeyBindingDef::new("Mod+H").mac("Mod+Alt+F")),
        ]
        .into_iter()
        .map(|(c, d)| KeyBinding::from_def(&d, c.into(), p).unwrap())
        .collect()
    }

    fn run(map: &mut Keymap, s: &str, p: Platform) -> KeyResult {
        map.press(ks(s, p), &ContextSnapshot::default())
    }

    fn cmd(id: &str) -> KeyResult {
        KeyResult::Command {
            command: CommandId::from(id.to_owned()),
            args: Value::Null,
        }
    }

    // spec 05: "Platform modifiers"
    #[test]
    fn mod_maps_per_platform() {
        assert!(ks("Mod+S", Platform::Mac).mods.cmd);
        assert!(ks("Mod+S", Platform::Windows).mods.ctrl);
        assert_eq!(ks("Mod+S", Platform::Mac).display(Platform::Mac), "⌘S");
        assert_eq!(
            ks("Mod+S", Platform::Windows).display(Platform::Windows),
            "Ctrl+S"
        );
        assert_eq!(
            ks("Mod+Shift+P", Platform::Mac).display(Platform::Mac),
            "⇧⌘P"
        );
        assert_eq!(ks("Ctrl++", Platform::Linux).key, "+");
    }

    // spec 05: "User binding overrides default"
    #[test]
    fn user_binding_overrides_default() {
        let p = Platform::Windows;
        let user = [UserBinding {
            key: Some("Mod+Shift+S".into()),
            command: "file.saveAll".into(),
            args: None,
            when: None,
        }];
        let (mut map, errors) = Keymap::build(defaults(p), vec![], &user, p);
        assert!(errors.is_empty());
        assert_eq!(run(&mut map, "Ctrl+Shift+S", p), cmd("file.saveAll"));
    }

    // spec 05: "Removing a default binding"
    #[test]
    fn removing_a_default_binding() {
        let p = Platform::Linux;
        let user = [UserBinding {
            key: Some("Mod+Shift+K".into()),
            command: "-edit.deleteLine".into(),
            args: None,
            when: None,
        }];
        let (mut map, _) = Keymap::build(defaults(p), vec![], &user, p);
        assert_eq!(run(&mut map, "Ctrl+Shift+K", p), KeyResult::NoMatch);
    }

    // spec 05: "Chord binding"
    #[test]
    fn chords_wait_for_second_key() {
        let p = Platform::Mac;
        let (mut map, _) = Keymap::build(defaults(p), vec![], &[], p);
        assert!(matches!(run(&mut map, "Cmd+K", p), KeyResult::Pending(_)));
        assert!(map.pending().is_some());
        assert_eq!(run(&mut map, "Z", p), cmd("view.zenMode"));
        assert!(map.pending().is_none());
        run(&mut map, "Cmd+K", p);
        assert_eq!(run(&mut map, "Cmd+O", p), cmd("file.openWorkspace"));
        run(&mut map, "Cmd+K", p);
        assert_eq!(run(&mut map, "X", p), KeyResult::NoMatch);
    }

    #[test]
    fn plus_and_equals_are_the_same_key() {
        let p = Platform::Windows;
        let (mut map, _) = Keymap::build(defaults(p), vec![], &[], p);
        assert_eq!(run(&mut map, "Ctrl+=", p), cmd("view.zoomIn"));
    }

    #[test]
    fn mac_specific_default() {
        let p = Platform::Mac;
        let (map, _) = Keymap::build(defaults(p), vec![], &[], p);
        let chords = map.chords_for(&"edit.replace".into());
        assert_eq!(chords[0].display(p), "⌥⌘F");
    }

    #[test]
    fn when_clauses_gate_bindings() {
        let p = Platform::Linux;
        let user = [UserBinding {
            key: Some("Mod+B".into()),
            command: "markdown.toggleBold".into(),
            args: None,
            when: Some("editorFocus && languageId == 'markdown'".into()),
        }];
        let base = vec![
            KeyBinding::from_def(
                &KeyBindingDef::new("Mod+B"),
                "view.togglePrimarySidebar".into(),
                p,
            )
            .unwrap(),
        ];
        let (mut map, _) = Keymap::build(base, vec![], &user, p);
        let mut ctx = ContextSnapshot::default();
        assert_eq!(
            map.press(ks("Ctrl+B", p), &ctx),
            cmd("view.togglePrimarySidebar")
        );
        ctx.set_bool("editorFocus", true)
            .set_str("languageId", "markdown");
        assert_eq!(map.press(ks("Ctrl+B", p), &ctx), cmd("markdown.toggleBold"));
    }

    #[test]
    fn invalid_user_entries_are_reported() {
        let p = Platform::Linux;
        let user = [
            UserBinding {
                key: Some("Hyper+X".into()),
                command: "a".into(),
                args: None,
                when: None,
            },
            UserBinding {
                key: Some("Mod+X".into()),
                command: "b".into(),
                args: None,
                when: Some("editorFocus &&".into()),
            },
            UserBinding {
                key: None,
                command: "c".into(),
                args: None,
                when: None,
            },
        ];
        let (_, errors) = Keymap::build(vec![], vec![], &user, p);
        assert_eq!(errors.len(), 3);
    }

    #[test]
    fn conflicts_are_listed() {
        let p = Platform::Linux;
        let user = [UserBinding {
            key: Some("Mod+S".into()),
            command: "x".into(),
            args: None,
            when: None,
        }];
        let (map, _) = Keymap::build(defaults(p), vec![], &user, p);
        assert_eq!(map.conflicts().len(), 1);
    }
}
