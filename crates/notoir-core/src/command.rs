//! The single command infrastructure (docs/specs/05-command-system-and-keybindings.md).
//!
//! Menus, toolbar, shortcuts, the palette, plugins and MCP all produce a
//! [`CommandInvocation`]; handlers are plain functions registered per module
//! (no giant `match`). The registry is generic over the handler context so the
//! app crate can register handlers that operate on its `AppCore`.

use std::borrow::Cow;
use std::collections::{BTreeMap, VecDeque};
use std::fmt;

use serde_json::Value;

use crate::keymap::KeyBindingDef;
use crate::when::{ContextSnapshot, When, WhenError};

/// Stable command id, e.g. `file.save`, `view.toggleMinimap`,
/// `plugin.word-count.count`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CommandId(Cow<'static, str>);

impl CommandId {
    pub const fn from_static(id: &'static str) -> Self {
        Self(Cow::Borrowed(id))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&'static str> for CommandId {
    fn from(id: &'static str) -> Self {
        Self(Cow::Borrowed(id))
    }
}

impl From<String> for CommandId {
    fn from(id: String) -> Self {
        Self(Cow::Owned(id))
    }
}

impl fmt::Display for CommandId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Category {
    File,
    Edit,
    Selection,
    View,
    Navigate,
    Git,
    Tools,
    Plugins,
    Help,
    Markdown,
    Json,
    Yaml,
    Compare,
    Mcp,
    Preferences,
}

impl Category {
    /// i18n key of the palette prefix ("File", "View", …).
    pub fn title_key(self) -> &'static str {
        match self {
            Category::File => "commands:category.file",
            Category::Edit => "commands:category.edit",
            Category::Selection => "commands:category.selection",
            Category::View => "commands:category.view",
            Category::Navigate => "commands:category.navigate",
            Category::Git => "commands:category.git",
            Category::Tools => "commands:category.tools",
            Category::Plugins => "commands:category.plugins",
            Category::Help => "commands:category.help",
            Category::Markdown => "commands:category.markdown",
            Category::Json => "commands:category.json",
            Category::Yaml => "commands:category.yaml",
            Category::Compare => "commands:category.compare",
            Category::Mcp => "commands:category.mcp",
            Category::Preferences => "commands:category.preferences",
        }
    }
}

/// Consulted by the permission gate for external principals (spec 32).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Risk {
    Safe,
    Sensitive,
    Dangerous,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InvocationSource {
    Keyboard,
    Menu,
    Toolbar,
    Palette,
    ContextMenu,
    StatusBar,
    Internal,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CommandInvocation {
    pub id: CommandId,
    pub args: Value,
    pub source: InvocationSource,
}

impl CommandInvocation {
    pub fn new(id: impl Into<CommandId>, source: InvocationSource) -> Self {
        Self {
            id: id.into(),
            args: Value::Null,
            source,
        }
    }

    pub fn with_args(mut self, args: Value) -> Self {
        self.args = args;
        self
    }
}

/// A user-facing error: an i18n key plus interpolation args, with optional
/// untranslated diagnostic detail (spec 35).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserError {
    pub key: String,
    pub args: Vec<(String, String)>,
    pub detail: Option<String>,
}

impl UserError {
    pub fn new(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            args: Vec::new(),
            detail: None,
        }
    }

    pub fn arg(mut self, name: impl Into<String>, value: impl fmt::Display) -> Self {
        self.args.push((name.into(), value.to_string()));
        self
    }

    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum CommandOutcome {
    Done,
    /// Background work started; the result arrives as a job result.
    Pending(u64),
    /// The user must confirm (dirty prompt, permission prompt…); `kind`
    /// identifies the prompt and `payload` carries its data.
    NeedsConfirmation {
        kind: &'static str,
        payload: Value,
    },
    /// The command's `when` is false in the current context.
    Disabled,
    Rejected(UserError),
}

pub type HandlerFn<C> = fn(&mut C, &CommandInvocation) -> CommandOutcome;

pub struct CommandSpec<C> {
    pub id: CommandId,
    /// i18n key of the title (`commands:file.save`).
    pub title_key: String,
    pub category: Category,
    /// lucide icon name for menus/palette.
    pub icon: Option<&'static str>,
    pub enablement: When,
    /// Listed in the Command Palette.
    pub palette: bool,
    pub default_keys: Vec<KeyBindingDef>,
    pub risk: Risk,
    pub handler: HandlerFn<C>,
}

impl<C> CommandSpec<C> {
    /// A palette command titled `commands:<id>`, always enabled, safe.
    pub fn new(id: &'static str, category: Category, handler: HandlerFn<C>) -> Self {
        Self {
            id: CommandId::from_static(id),
            title_key: format!("commands:{id}"),
            category,
            icon: None,
            enablement: When::ALWAYS,
            palette: true,
            default_keys: Vec::new(),
            risk: Risk::Safe,
            handler,
        }
    }

    pub fn when(mut self, expr: &str) -> Result<Self, WhenError> {
        self.enablement = When::parse(expr)?;
        Ok(self)
    }

    pub fn keys(mut self, keys: impl IntoIterator<Item = KeyBindingDef>) -> Self {
        self.default_keys.extend(keys);
        self
    }

    pub fn icon(mut self, icon: &'static str) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn risk(mut self, risk: Risk) -> Self {
        self.risk = risk;
        self
    }

    pub fn hidden(mut self) -> Self {
        self.palette = false;
        self
    }
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum RegistryError {
    #[error("command {0} is already registered")]
    Duplicate(CommandId),
}

pub struct CommandRegistry<C> {
    commands: BTreeMap<CommandId, CommandSpec<C>>,
    aliases: BTreeMap<CommandId, CommandId>,
}

impl<C> Default for CommandRegistry<C> {
    fn default() -> Self {
        Self {
            commands: BTreeMap::new(),
            aliases: BTreeMap::new(),
        }
    }
}

impl<C> CommandRegistry<C> {
    pub fn register(&mut self, spec: CommandSpec<C>) -> Result<(), RegistryError> {
        if self.commands.contains_key(&spec.id) {
            return Err(RegistryError::Duplicate(spec.id));
        }
        self.commands.insert(spec.id.clone(), spec);
        Ok(())
    }

    /// Removes a command (plugins unloading); returns it.
    pub fn unregister(&mut self, id: &CommandId) -> Option<CommandSpec<C>> {
        self.commands.remove(id)
    }

    /// Keeps a renamed command reachable under its old id.
    pub fn alias(&mut self, old: impl Into<CommandId>, new: impl Into<CommandId>) {
        self.aliases.insert(old.into(), new.into());
    }

    pub fn get(&self, id: &CommandId) -> Option<&CommandSpec<C>> {
        let id = self.aliases.get(id).unwrap_or(id);
        self.commands.get(id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &CommandSpec<C>> {
        self.commands.values()
    }

    /// Commands shown in the palette for `ctx`.
    pub fn palette_entries<'a>(
        &'a self,
        ctx: &'a ContextSnapshot,
    ) -> impl Iterator<Item = &'a CommandSpec<C>> {
        self.commands
            .values()
            .filter(move |c| c.palette && c.enablement.eval(ctx))
    }

    /// Looks up and checks enablement; returns the handler to call. Copying
    /// the fn pointer out lets the caller pass `&mut C` even when `C` owns
    /// this registry.
    pub fn resolve(
        &self,
        inv: &CommandInvocation,
        ctx: &ContextSnapshot,
    ) -> Result<HandlerFn<C>, CommandOutcome> {
        let Some(spec) = self.get(&inv.id) else {
            return Err(CommandOutcome::Rejected(
                UserError::new("errors:unknownCommand").arg("command", &inv.id),
            ));
        };
        if !spec.enablement.eval(ctx) {
            return Err(CommandOutcome::Disabled);
        }
        Ok(spec.handler)
    }

    /// Resolves and runs `inv` against `target`.
    pub fn execute(
        &self,
        target: &mut C,
        inv: &CommandInvocation,
        ctx: &ContextSnapshot,
    ) -> CommandOutcome {
        match self.resolve(inv, ctx) {
            Ok(handler) => handler(target, inv),
            Err(outcome) => outcome,
        }
    }
}

/// FIFO of invocations processed on the UI thread once per frame.
#[derive(Debug, Default)]
pub struct CommandQueue {
    queue: VecDeque<CommandInvocation>,
}

impl CommandQueue {
    pub fn push(&mut self, inv: CommandInvocation) {
        self.queue.push_back(inv);
    }

    pub fn pop(&mut self) -> Option<CommandInvocation> {
        self.queue.pop_front()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Ctx {
        saves: Vec<InvocationSource>,
        deleted_lines: usize,
    }

    fn save(c: &mut Ctx, inv: &CommandInvocation) -> CommandOutcome {
        c.saves.push(inv.source.clone());
        CommandOutcome::Done
    }

    fn delete_line(c: &mut Ctx, _: &CommandInvocation) -> CommandOutcome {
        c.deleted_lines += 1;
        CommandOutcome::Done
    }

    fn registry() -> CommandRegistry<Ctx> {
        let mut r = CommandRegistry::default();
        r.register(CommandSpec::new("file.save", Category::File, save))
            .unwrap();
        r.register(
            CommandSpec::new("edit.deleteLine", Category::Edit, delete_line)
                .when("editorFocus && !readOnly")
                .unwrap(),
        )
        .unwrap();
        r
    }

    // spec 05: "Menu, shortcut, and palette all save through the same handler"
    #[test]
    fn every_source_runs_the_same_handler() {
        let r = registry();
        let mut c = Ctx::default();
        let ctx = ContextSnapshot::default();
        for source in [
            InvocationSource::Menu,
            InvocationSource::Keyboard,
            InvocationSource::Palette,
        ] {
            let out = r.execute(&mut c, &CommandInvocation::new("file.save", source), &ctx);
            assert_eq!(out, CommandOutcome::Done);
        }
        assert_eq!(
            c.saves,
            vec![
                InvocationSource::Menu,
                InvocationSource::Keyboard,
                InvocationSource::Palette
            ]
        );
    }

    // spec 05: "Disabled command is not executed"
    #[test]
    fn disabled_command_is_not_executed() {
        let r = registry();
        let mut c = Ctx::default();
        let mut ctx = ContextSnapshot::default();
        ctx.set_bool("editorFocus", true).set_bool("readOnly", true);
        let inv = CommandInvocation::new("edit.deleteLine", InvocationSource::Keyboard);
        assert_eq!(r.execute(&mut c, &inv, &ctx), CommandOutcome::Disabled);
        assert_eq!(c.deleted_lines, 0);
        assert_eq!(r.palette_entries(&ctx).count(), 1);
    }

    #[test]
    fn unknown_and_duplicate_commands() {
        let mut r = registry();
        let mut c = Ctx::default();
        let out = r.execute(
            &mut c,
            &CommandInvocation::new("nope", InvocationSource::Menu),
            &ContextSnapshot::default(),
        );
        assert!(matches!(out, CommandOutcome::Rejected(e) if e.key == "errors:unknownCommand"));
        assert!(
            r.register(CommandSpec::new("file.save", Category::File, save))
                .is_err()
        );
    }

    #[test]
    fn aliases_resolve() {
        let mut r = registry();
        r.alias("workbench.action.files.save", "file.save");
        assert!(r.get(&"workbench.action.files.save".into()).is_some());
    }
}
