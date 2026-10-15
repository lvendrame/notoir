//! Notifications and toasts (docs/specs/19-status-bar-and-notifications.md).
//!
//! Messages are translation keys plus arguments, rendered at display time so
//! a language switch re-renders them. Info/Success toasts expire after 4 s,
//! warnings after 8 s, errors stay until dismissed; identical notifications
//! (same dedupe key) collapse into one toast with a counter.

use crate::command::{CommandInvocation, UserError};

pub const INFO_MS: u64 = 4_000;
pub const WARNING_MS: u64 = 8_000;
pub const MAX_TOASTS: usize = 3;
const HISTORY_LIMIT: usize = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Info,
    Success,
    Warning,
    Error,
}

/// An i18n key with interpolation arguments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Translatable {
    pub key: String,
    pub args: Vec<(String, String)>,
}

impl Translatable {
    pub fn new(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            args: Vec::new(),
        }
    }

    pub fn arg(mut self, name: impl Into<String>, value: impl std::fmt::Display) -> Self {
        self.args.push((name.into(), value.to_string()));
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct NotificationAction {
    pub label_key: String,
    pub command: CommandInvocation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NotificationSource {
    App,
    Settings,
    Storage,
    Recovery,
    Git,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Notification {
    pub id: u64,
    pub severity: Severity,
    pub message: Translatable,
    /// Untranslated diagnostic detail behind "Show Details".
    pub detail: Option<String>,
    pub actions: Vec<NotificationAction>,
    pub source: NotificationSource,
    pub dedupe_key: Option<String>,
    /// Collapsed repetitions (×N when > 1).
    pub count: u32,
    pub created_ms: u64,
    /// `None` = sticky until dismissed.
    pub expires_ms: Option<u64>,
    pub toast_visible: bool,
}

impl Notification {
    pub fn new(severity: Severity, message: Translatable) -> Self {
        Self {
            id: 0,
            severity,
            message,
            detail: None,
            actions: Vec::new(),
            source: NotificationSource::App,
            dedupe_key: None,
            count: 1,
            created_ms: 0,
            expires_ms: None,
            toast_visible: true,
        }
    }

    pub fn from_error(error: &UserError) -> Self {
        let mut message = Translatable::new(error.key.clone());
        message.args = error.args.clone();
        Self {
            detail: error.detail.clone(),
            ..Self::new(Severity::Error, message)
        }
    }

    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn action(mut self, label_key: impl Into<String>, command: CommandInvocation) -> Self {
        self.actions.push(NotificationAction {
            label_key: label_key.into(),
            command,
        });
        self
    }

    pub fn source(mut self, source: NotificationSource) -> Self {
        self.source = source;
        self
    }

    pub fn dedupe(mut self, key: impl Into<String>) -> Self {
        self.dedupe_key = Some(key.into());
        self
    }

    fn lifetime(&self) -> Option<u64> {
        match self.severity {
            Severity::Info | Severity::Success => Some(INFO_MS),
            Severity::Warning => Some(WARNING_MS),
            Severity::Error => None,
        }
    }
}

#[derive(Debug, Default)]
pub struct NotificationCenter {
    items: Vec<Notification>,
    next_id: u64,
    /// Zen Mode: only warnings and errors toast.
    pub quiet: bool,
    unread: usize,
}

impl NotificationCenter {
    /// Adds (or collapses into an existing visible toast) and returns its id.
    pub fn notify(&mut self, mut n: Notification, now_ms: u64) -> u64 {
        let lifetime = n.lifetime();
        if let Some(key) = &n.dedupe_key
            && let Some(existing) = self
                .items
                .iter_mut()
                .find(|e| e.toast_visible && e.dedupe_key.as_ref() == Some(key))
        {
            existing.count += 1;
            existing.expires_ms = lifetime.map(|l| now_ms + l);
            return existing.id;
        }
        self.next_id += 1;
        n.id = self.next_id;
        n.created_ms = now_ms;
        n.expires_ms = lifetime.map(|l| now_ms + l);
        n.toast_visible = !self.quiet || n.severity >= Severity::Warning;
        self.items.push(n);
        self.unread += 1;
        if self.items.len() > HISTORY_LIMIT {
            self.items.remove(0);
        }
        self.next_id
    }

    /// Hides expired toasts (they stay in the history).
    pub fn tick(&mut self, now_ms: u64) {
        for n in &mut self.items {
            if n.toast_visible && n.expires_ms.is_some_and(|e| now_ms >= e) {
                n.toast_visible = false;
            }
        }
    }

    /// Hovering a toast pauses its timer.
    pub fn hold(&mut self, id: u64, now_ms: u64) {
        if let Some(n) = self.items.iter_mut().find(|n| n.id == id)
            && let (Some(exp), Some(life)) = (n.expires_ms, n.lifetime())
        {
            n.expires_ms = Some(exp.max(now_ms + life.min(2_000)));
        }
    }

    pub fn dismiss(&mut self, id: u64) {
        if let Some(n) = self.items.iter_mut().find(|n| n.id == id) {
            n.toast_visible = false;
        }
    }

    /// Up to [`MAX_TOASTS`] newest visible toasts, oldest first.
    pub fn toasts(&self) -> Vec<&Notification> {
        let visible: Vec<&Notification> = self.items.iter().filter(|n| n.toast_visible).collect();
        visible[visible.len().saturating_sub(MAX_TOASTS)..].to_vec()
    }

    /// Session history, newest first.
    pub fn history(&self) -> impl Iterator<Item = &Notification> {
        self.items.iter().rev()
    }

    pub fn unread(&self) -> usize {
        self.unread
    }

    pub fn mark_read(&mut self) {
        self.unread = 0;
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.unread = 0;
    }

    /// Next instant a toast expires (for scheduling a repaint).
    pub fn next_expiry(&self) -> Option<u64> {
        self.items
            .iter()
            .filter(|n| n.toast_visible)
            .filter_map(|n| n.expires_ms)
            .min()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(key: &str) -> Notification {
        Notification::new(Severity::Info, Translatable::new(key))
    }

    #[test]
    fn info_expires_error_sticks() {
        let mut c = NotificationCenter::default();
        c.notify(info("a"), 0);
        c.notify(
            Notification::new(Severity::Error, Translatable::new("e")),
            0,
        );
        c.tick(INFO_MS);
        let toasts = c.toasts();
        assert_eq!(toasts.len(), 1);
        assert_eq!(toasts[0].severity, Severity::Error);
        assert_eq!(c.history().count(), 2, "expired toasts stay in history");
    }

    // spec 19: "Duplicate notifications collapse"
    #[test]
    fn duplicates_collapse_with_counter() {
        let mut c = NotificationCenter::default();
        for t in 0..3 {
            c.notify(
                Notification::new(Severity::Warning, Translatable::new("autosaveFailed"))
                    .dedupe("autosave:a.md"),
                t,
            );
        }
        let toasts = c.toasts();
        assert_eq!(toasts.len(), 1);
        assert_eq!(toasts[0].count, 3);
    }

    #[test]
    fn at_most_three_toasts_and_quiet_mode() {
        let mut c = NotificationCenter::default();
        for i in 0..5 {
            c.notify(info(&format!("n{i}")), 0);
        }
        let keys: Vec<_> = c.toasts().iter().map(|n| n.message.key.clone()).collect();
        assert_eq!(keys, ["n2", "n3", "n4"]);
        c.clear();
        c.quiet = true;
        c.notify(info("quiet"), 0);
        assert!(c.toasts().is_empty());
        c.notify(
            Notification::new(Severity::Warning, Translatable::new("w")),
            0,
        );
        assert_eq!(c.toasts().len(), 1);
    }

    #[test]
    fn hover_extends_and_dismiss_hides() {
        let mut c = NotificationCenter::default();
        let id = c.notify(info("a"), 0);
        c.hold(id, INFO_MS - 10);
        c.tick(INFO_MS);
        assert_eq!(c.toasts().len(), 1);
        c.dismiss(id);
        assert!(c.toasts().is_empty());
        assert_eq!(c.unread(), 1);
        c.mark_read();
        assert_eq!(c.unread(), 0);
    }
}
