//! Themes and design tokens (docs/specs/21-themes-and-design-system.md).
//!
//! Built-in themes are TOML files in `assets/themes/` using the same format as
//! user themes. Every chrome colour resolves to a [`Tok`]; widgets never
//! hard-code colour literals.

use std::collections::HashMap;

use egui::{Color32, CornerRadius, Stroke, Visuals};
use serde::Deserialize;

macro_rules! tokens {
    ($($variant:ident => $name:literal),* $(,)?) => {
        /// Every design token. Names match the theme file keys (`ui` keys bare,
        /// other sections prefixed with the section name).
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum Tok { $($variant),* }

        impl Tok {
            pub const ALL: &'static [Tok] = &[$(Tok::$variant),*];

            pub fn name(self) -> &'static str {
                match self { $(Tok::$variant => $name),* }
            }
        }
    };
}

tokens! {
    BgApp => "bg.app",
    BgPanel => "bg.panel",
    BgPanelRaised => "bg.panelRaised",
    BgEditor => "bg.editor",
    BgOverlay => "bg.overlay",
    Border => "border",
    BorderStrong => "border.strong",
    TextPrimary => "text.primary",
    TextSecondary => "text.secondary",
    TextTertiary => "text.tertiary",
    TextDisabled => "text.disabled",
    Accent => "accent",
    AccentStrong => "accent.strong",
    AccentSoft => "accent.soft",
    Danger => "danger",
    Warning => "warning",
    Success => "success",
    Info => "info",
    FocusRing => "focus.ring",
    ShadowFloat => "shadow.float",

    EditorBackground => "editor.background",
    EditorForeground => "editor.foreground",
    EditorLineHighlight => "editor.lineHighlight",
    EditorSelection => "editor.selection",
    EditorSelectionInactive => "editor.selectionInactive",
    EditorCursor => "editor.cursor",
    EditorWhitespace => "editor.whitespace",
    EditorIndentGuide => "editor.indentGuide",
    EditorRuler => "editor.ruler",
    EditorFindMatch => "editor.findMatch",
    EditorFindMatchCurrent => "editor.findMatchCurrent",
    EditorBracketMatch => "editor.bracketMatch",
    GutterBackground => "editor.gutter.background",
    GutterLineNumber => "editor.gutter.lineNumber",
    GutterLineNumberActive => "editor.gutter.lineNumberActive",
    MinimapBlock => "editor.minimap.block",
    MinimapViewport => "editor.minimap.viewport",
    MinimapViewportHover => "editor.minimap.viewportHover",

    SyntaxComment => "syntax.comment",
    SyntaxKeyword => "syntax.keyword",
    SyntaxString => "syntax.string",
    SyntaxNumber => "syntax.number",
    SyntaxConstant => "syntax.constant",
    SyntaxFunction => "syntax.function",
    SyntaxType => "syntax.type",
    SyntaxVariable => "syntax.variable",
    SyntaxProperty => "syntax.property",
    SyntaxOperator => "syntax.operator",
    SyntaxPunctuation => "syntax.punctuation",
    SyntaxTag => "syntax.tag",
    SyntaxAttribute => "syntax.attribute",
    SyntaxRegex => "syntax.regex",
    SyntaxEscape => "syntax.escape",
    SyntaxHeading => "syntax.heading",
    SyntaxEmphasis => "syntax.emphasis",
    SyntaxStrong => "syntax.strong",
    SyntaxLink => "syntax.link",
    SyntaxCode => "syntax.code",
    SyntaxQuote => "syntax.quote",
    SyntaxListMarker => "syntax.listMarker",
    SyntaxInvalid => "syntax.invalid",

    GitAdded => "git.added",
    GitModified => "git.modified",
    GitDeleted => "git.deleted",
    GitUntracked => "git.untracked",
    GitRenamed => "git.renamed",
    GitConflict => "git.conflict",
    GitIgnored => "git.ignored",

    DiffInsertedLine => "diff.insertedLine",
    DiffRemovedLine => "diff.removedLine",
    DiffInsertedText => "diff.insertedText",
    DiffRemovedText => "diff.removedText",
    DiffGutterInserted => "diff.gutterInserted",
    DiffGutterRemoved => "diff.gutterRemoved",
}

/// Built-in theme choice. `Auto` follows the OS light/dark preference.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ThemeId {
    Midnight,
    Paper,
    Dracula,
}

impl ThemeId {
    pub const ALL: [ThemeId; 3] = [ThemeId::Midnight, ThemeId::Paper, ThemeId::Dracula];

    pub fn id(self) -> &'static str {
        match self {
            ThemeId::Midnight => "notoir-midnight",
            ThemeId::Paper => "notoir-paper",
            ThemeId::Dracula => "dracula",
        }
    }

    pub fn from_id(id: &str) -> Option<ThemeId> {
        ThemeId::ALL.into_iter().find(|t| t.id() == id)
    }

    fn source(self) -> &'static str {
        match self {
            ThemeId::Midnight => include_str!("../../../assets/themes/notoir-midnight.toml"),
            ThemeId::Paper => include_str!("../../../assets/themes/notoir-paper.toml"),
            ThemeId::Dracula => include_str!("../../../assets/themes/dracula.toml"),
        }
    }

    /// Theme for `workbench.theme = "auto"`.
    pub fn for_system(dark: bool) -> ThemeId {
        if dark {
            ThemeId::Midnight
        } else {
            ThemeId::Paper
        }
    }

    pub fn load(self) -> Theme {
        Theme::parse(self.source()).unwrap_or_else(|e| panic!("built-in theme {}: {e}", self.id()))
    }

    /// The built-in theme custom themes of `kind` inherit from.
    pub fn base_for(kind: ThemeKind) -> ThemeId {
        match kind {
            ThemeKind::Dark => ThemeId::Midnight,
            ThemeKind::Light => ThemeId::Paper,
        }
    }
}

/// Custom themes (spec 21): every `*.toml` in `dir`, sorted by file name;
/// an unreadable or invalid file yields `(file name, error)`.
pub fn load_custom_dir(dir: &std::path::Path) -> Vec<Result<Theme, (String, String)>> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<std::path::PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("toml"))
        })
        .collect();
    files.sort();
    files
        .into_iter()
        .map(|p| {
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            std::fs::read_to_string(&p)
                .map_err(|e| e.to_string())
                .and_then(|s| Theme::parse_custom(&s).map_err(|e| e.to_string()))
                .map_err(|e| (name, e))
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeKind {
    Dark,
    Light,
}

/// A resolved colour plus optional font style (syntax tokens).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    pub color: Color32,
    pub italic: bool,
    pub bold: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum ThemeError {
    #[error("invalid theme TOML: {0}")]
    Toml(String),
    #[error("{token}: invalid colour {value:?}")]
    BadColor { token: String, value: String },
    #[error("{token}: unknown base token {base:?}")]
    UnknownBase { token: String, base: String },
    #[error("{0}: base references form a cycle")]
    Cycle(String),
    #[error("missing token {0}")]
    Missing(&'static str),
    #[error("theme id {0:?} is reserved")]
    ReservedId(String),
}

#[derive(Deserialize)]
struct Meta {
    id: String,
    name: String,
    kind: ThemeKind,
}

#[derive(Deserialize)]
struct MetaOnly {
    meta: Meta,
}

#[derive(Deserialize)]
struct ThemeFile {
    meta: Meta,
    #[serde(default)]
    ui: HashMap<String, RawValue>,
    #[serde(default)]
    editor: HashMap<String, RawValue>,
    #[serde(default)]
    syntax: HashMap<String, RawValue>,
    #[serde(default)]
    git: HashMap<String, RawValue>,
    #[serde(default)]
    diff: HashMap<String, RawValue>,
}

#[derive(Clone, Deserialize)]
#[serde(untagged)]
enum RawValue {
    Hex(String),
    Spec {
        color: Option<String>,
        base: Option<String>,
        alpha: Option<f32>,
        #[serde(default)]
        italic: bool,
        #[serde(default)]
        bold: bool,
    },
}

#[derive(Clone, Debug)]
pub struct Theme {
    pub id: String,
    pub name: String,
    pub kind: ThemeKind,
    styles: HashMap<&'static str, Style>,
}

impl Theme {
    /// Parses a theme file; every [`Tok`] must resolve.
    pub fn parse(source: &str) -> Result<Theme, ThemeError> {
        Self::parse_over(source, None)
    }

    /// Parses a user theme: tokens it leaves out (or bases it names but does
    /// not define) come from the built-in theme of the same kind.
    pub fn parse_custom(source: &str) -> Result<Theme, ThemeError> {
        let kind = toml::from_str::<MetaOnly>(source)
            .map_err(|e| ThemeError::Toml(e.to_string()))?
            .meta
            .kind;
        let theme = Self::parse_over(source, Some(&ThemeId::base_for(kind).load()))?;
        if theme.id == "auto" || ThemeId::from_id(&theme.id).is_some() || theme.id.trim().is_empty()
        {
            return Err(ThemeError::ReservedId(theme.id));
        }
        Ok(theme)
    }

    fn parse_over(source: &str, base: Option<&Theme>) -> Result<Theme, ThemeError> {
        let file: ThemeFile =
            toml::from_str(source).map_err(|e| ThemeError::Toml(e.to_string()))?;
        let mut raw: HashMap<String, RawValue> = file.ui;
        for (section, map) in [
            ("editor", file.editor),
            ("syntax", file.syntax),
            ("git", file.git),
            ("diff", file.diff),
        ] {
            raw.extend(map.into_iter().map(|(k, v)| (format!("{section}.{k}"), v)));
        }
        let mut resolved: HashMap<String, Style> = HashMap::new();
        for name in raw.keys() {
            resolve(name, &raw, &mut resolved, base, 0)?;
        }
        let mut styles = HashMap::new();
        for tok in Tok::ALL {
            let style = resolved
                .get(tok.name())
                .copied()
                .or_else(|| base.map(|b| b.style(*tok)))
                .ok_or(ThemeError::Missing(tok.name()))?;
            styles.insert(tok.name(), style);
        }
        Ok(Theme {
            id: file.meta.id,
            name: file.meta.name,
            kind: file.meta.kind,
            styles,
        })
    }

    pub fn style(&self, tok: Tok) -> Style {
        self.styles[tok.name()]
    }

    pub fn color(&self, tok: Tok) -> Color32 {
        self.style(tok).color
    }

    pub fn is_dark(&self) -> bool {
        self.kind == ThemeKind::Dark
    }

    /// Applies the tokens to egui's global style.
    pub fn apply(&self, ctx: &egui::Context) {
        let c = |t| self.color(t);
        ctx.all_styles_mut(|style| {
            let mut v = if self.is_dark() {
                Visuals::dark()
            } else {
                Visuals::light()
            };
            v.panel_fill = c(Tok::BgPanel);
            v.window_fill = c(Tok::BgOverlay);
            v.extreme_bg_color = c(Tok::BgEditor);
            v.faint_bg_color = c(Tok::BgPanelRaised);
            v.code_bg_color = c(Tok::BgPanelRaised);
            v.override_text_color = None;
            v.hyperlink_color = c(Tok::Accent);
            v.warn_fg_color = c(Tok::Warning);
            v.error_fg_color = c(Tok::Danger);
            v.selection.bg_fill = c(Tok::EditorSelection);
            v.selection.stroke = Stroke::new(1.0, c(Tok::TextPrimary));
            v.window_stroke = Stroke::new(1.0, c(Tok::Border));
            v.window_corner_radius = CornerRadius::same(RADIUS_OVERLAY);
            v.menu_corner_radius = CornerRadius::same(RADIUS_OVERLAY);
            v.window_shadow.color = c(Tok::ShadowFloat);
            v.popup_shadow.color = c(Tok::ShadowFloat);

            let w = &mut v.widgets;
            for (state, bg, fg, border) in [
                (
                    &mut w.noninteractive,
                    Tok::BgPanel,
                    Tok::TextPrimary,
                    Tok::Border,
                ),
                (
                    &mut w.inactive,
                    Tok::BgPanelRaised,
                    Tok::TextSecondary,
                    Tok::Border,
                ),
                (
                    &mut w.hovered,
                    Tok::BgPanelRaised,
                    Tok::TextPrimary,
                    Tok::BorderStrong,
                ),
                (&mut w.active, Tok::AccentSoft, Tok::Accent, Tok::FocusRing),
                (
                    &mut w.open,
                    Tok::BgPanelRaised,
                    Tok::TextPrimary,
                    Tok::BorderStrong,
                ),
            ] {
                state.bg_fill = c(bg);
                state.weak_bg_fill = c(bg);
                state.fg_stroke = Stroke::new(1.0, c(fg));
                state.bg_stroke = Stroke::new(1.0, c(border));
                state.corner_radius = CornerRadius::same(RADIUS);
                state.expansion = 0.0;
            }
            style.visuals = v;

            let s = &mut style.spacing;
            s.item_spacing = egui::vec2(8.0, 4.0);
            s.button_padding = egui::vec2(8.0, 4.0);
            s.interact_size.y = 28.0;
        });
    }
}

fn parse_hex(token: &str, value: &str) -> Result<Color32, ThemeError> {
    let bad = || ThemeError::BadColor {
        token: token.to_owned(),
        value: value.to_owned(),
    };
    let hex = value.strip_prefix('#').ok_or_else(bad)?;
    let byte =
        |i: usize| u8::from_str_radix(hex.get(i..i + 2).ok_or_else(bad)?, 16).map_err(|_| bad());
    match hex.len() {
        6 => Ok(Color32::from_rgb(byte(0)?, byte(2)?, byte(4)?)),
        8 => Ok(Color32::from_rgba_unmultiplied(
            byte(0)?,
            byte(2)?,
            byte(4)?,
            byte(6)?,
        )),
        _ => Err(bad()),
    }
}

fn with_alpha(c: Color32, alpha: f32) -> Color32 {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    let a = (f32::from(a) * alpha.clamp(0.0, 1.0)).round() as u8;
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

fn resolve(
    name: &str,
    raw: &HashMap<String, RawValue>,
    resolved: &mut HashMap<String, Style>,
    inherit: Option<&Theme>,
    depth: usize,
) -> Result<Style, ThemeError> {
    if let Some(s) = resolved.get(name) {
        return Ok(*s);
    }
    if depth > 16 {
        return Err(ThemeError::Cycle(name.to_owned()));
    }
    let style = match &raw[name] {
        RawValue::Hex(hex) => Style {
            color: parse_hex(name, hex)?,
            italic: false,
            bold: false,
        },
        RawValue::Spec {
            color,
            base,
            alpha,
            italic,
            bold,
        } => {
            let mut color = match (color, base) {
                (Some(hex), _) => parse_hex(name, hex)?,
                (None, Some(base)) => {
                    if raw.contains_key(base) {
                        resolve(base, raw, resolved, inherit, depth + 1)?.color
                    } else if let Some(s) = inherit.and_then(|t| t.styles.get(base.as_str())) {
                        s.color
                    } else {
                        return Err(ThemeError::UnknownBase {
                            token: name.to_owned(),
                            base: base.clone(),
                        });
                    }
                }
                (None, None) => {
                    return Err(ThemeError::BadColor {
                        token: name.to_owned(),
                        value: "neither color nor base".to_owned(),
                    });
                }
            };
            if let Some(alpha) = alpha {
                color = with_alpha(color, *alpha);
            }
            Style {
                color,
                italic: *italic,
                bold: *bold,
            }
        }
    };
    resolved.insert(name.to_owned(), style);
    Ok(style)
}

/// Small radius: buttons, inputs, tab corners.
pub const RADIUS: u8 = 4;
/// Overlays: menus, palette, toasts.
pub const RADIUS_OVERLAY: u8 = 6;
pub const ICON: f32 = 16.0;
pub const ICON_SM: f32 = 14.0;

/// WCAG 2.x contrast ratio between two opaque colours.
pub fn contrast(a: Color32, b: Color32) -> f32 {
    fn luminance(c: Color32) -> f32 {
        let lin = |v: u8| {
            let v = f32::from(v) / 255.0;
            if v <= 0.040_45 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(c.r()) + 0.7152 * lin(c.g()) + 0.0722 * lin(c.b())
    }
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// The theme token for a syntax class (`None` = plain text colour).
/// Colour token of a Git status (spec 07 §Git Decorations).
pub fn git_token(s: notoir_git::FileStatus) -> Tok {
    use notoir_git::FileStatus as F;
    match s {
        F::Modified => Tok::GitModified,
        F::Added => Tok::GitAdded,
        F::Untracked => Tok::GitUntracked,
        F::Deleted => Tok::GitDeleted,
        F::Conflicted => Tok::GitConflict,
        F::Renamed => Tok::GitRenamed,
        F::Ignored => Tok::GitIgnored,
    }
}

pub fn syntax_token(kind: notoir_editor::syntax::TokenKind) -> Option<Tok> {
    use notoir_editor::syntax::TokenKind as K;
    Some(match kind {
        K::Plain => return None,
        K::Comment => Tok::SyntaxComment,
        K::Keyword => Tok::SyntaxKeyword,
        K::String => Tok::SyntaxString,
        K::Number => Tok::SyntaxNumber,
        K::Constant => Tok::SyntaxConstant,
        K::Function => Tok::SyntaxFunction,
        K::Type => Tok::SyntaxType,
        K::Variable => Tok::SyntaxVariable,
        K::Property => Tok::SyntaxProperty,
        K::Operator => Tok::SyntaxOperator,
        K::Punctuation => Tok::SyntaxPunctuation,
        K::Tag => Tok::SyntaxTag,
        K::Attribute => Tok::SyntaxAttribute,
        K::Regex => Tok::SyntaxRegex,
        K::Escape => Tok::SyntaxEscape,
        K::Heading => Tok::SyntaxHeading,
        K::Emphasis => Tok::SyntaxEmphasis,
        K::Strong => Tok::SyntaxStrong,
        K::Link => Tok::SyntaxLink,
        K::Code => Tok::SyntaxCode,
        K::Quote => Tok::SyntaxQuote,
        K::ListMarker => Tok::SyntaxListMarker,
        K::Invalid => Tok::SyntaxInvalid,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_themes_define_every_token() {
        for id in ThemeId::ALL {
            let theme = Theme::parse(id.source()).unwrap_or_else(|e| panic!("{}: {e}", id.id()));
            assert_eq!(theme.id, id.id());
        }
    }

    // spec 21: "Built-in themes meet contrast"
    #[test]
    fn built_in_themes_meet_contrast() {
        for id in ThemeId::ALL {
            let t = id.load();
            for bg in [Tok::BgEditor, Tok::BgPanel, Tok::BgApp] {
                for (fg, min) in [
                    (Tok::TextPrimary, 4.5),
                    (Tok::TextSecondary, 4.5),
                    (Tok::TextTertiary, 3.0),
                    (Tok::FocusRing, 3.0),
                ] {
                    let ratio = contrast(t.color(fg), t.color(bg));
                    assert!(
                        ratio >= min,
                        "{}: {} on {} = {ratio:.2} < {min}",
                        t.name,
                        fg.name(),
                        bg.name()
                    );
                }
            }
        }
    }

    #[test]
    fn derived_tokens_follow_their_base() {
        let t = ThemeId::Midnight.load();
        assert_eq!(t.color(Tok::EditorBackground), t.color(Tok::BgEditor));
        let soft = t.color(Tok::AccentSoft).to_srgba_unmultiplied();
        let accent = t.color(Tok::Accent).to_srgba_unmultiplied();
        // Color32 is premultiplied, so low-alpha channels round slightly.
        for i in 0..3 {
            assert!(soft[i].abs_diff(accent[i]) <= 3, "{soft:?} vs {accent:?}");
        }
        assert_eq!(soft[3], (255.0_f32 * 0.14).round() as u8);
        assert!(t.style(Tok::SyntaxComment).italic);
    }

    #[test]
    fn missing_tokens_and_cycles_are_errors() {
        let minimal =
            "[meta]\nid = \"x\"\nname = \"X\"\nkind = \"dark\"\n[ui]\naccent = \"#ffffff\"\n";
        assert!(matches!(Theme::parse(minimal), Err(ThemeError::Missing(_))));
        let cycle = "[meta]\nid = \"x\"\nname = \"X\"\nkind = \"dark\"\n[ui]\na = { base = \"b\" }\nb = { base = \"a\" }\n";
        assert!(matches!(Theme::parse(cycle), Err(ThemeError::Cycle(_))));
    }

    #[test]
    fn contrast_of_black_on_white_is_21() {
        assert!((contrast(Color32::BLACK, Color32::WHITE) - 21.0).abs() < 0.01);
    }

    #[test]
    fn custom_themes_inherit_from_the_base_of_their_kind() {
        let light = Theme::parse_custom(
            "[meta]\nid = \"sand\"\nname = \"Sand\"\nkind = \"light\"\n[ui]\n\"accent.soft\" = { base = \"accent\", alpha = 0.5 }\n",
        )
        .unwrap();
        let paper = ThemeId::Paper.load();
        // A base defined only in the built-in theme resolves from it.
        assert_eq!(light.color(Tok::Accent), paper.color(Tok::Accent));
        assert_eq!(
            light.color(Tok::AccentSoft),
            with_alpha(paper.color(Tok::Accent), 0.5)
        );
        assert_eq!(light.color(Tok::BgEditor), paper.color(Tok::BgEditor));
        for reserved in ["notoir-paper", "auto", " "] {
            let src = format!("[meta]\nid = \"{reserved}\"\nname = \"X\"\nkind = \"dark\"\n");
            assert!(matches!(
                Theme::parse_custom(&src),
                Err(ThemeError::ReservedId(_))
            ));
        }
        assert!(
            Theme::parse_custom(
                "[meta]\nid = \"x\"\nname = \"X\"\nkind = \"dark\"\n[ui]\na = { base = \"nope\" }\n"
            )
            .is_err()
        );
    }
}
