//! Keyboard → keymap → commands (docs/specs/05 §Keybindings).

use notoir_core::command::{CommandInvocation, InvocationSource};
use notoir_core::keymap::{KeyResult, Keystroke, Modifiers, Platform};
use notoir_core::when::ContextSnapshot;

use crate::core::AppCore;

/// Converts an egui key press into a keystroke for `platform`.
pub fn keystroke(key: egui::Key, mods: egui::Modifiers, platform: Platform) -> Option<Keystroke> {
    let name = match key {
        egui::Key::ArrowUp
        | egui::Key::ArrowDown
        | egui::Key::ArrowLeft
        | egui::Key::ArrowRight => key.name(),
        egui::Key::Minus => "-",
        other => other.symbol_or_name(),
    };
    let mods = if platform == Platform::Mac {
        Modifiers {
            ctrl: mods.ctrl,
            alt: mods.alt,
            shift: mods.shift,
            cmd: mods.mac_cmd || mods.command,
        }
    } else {
        Modifiers {
            ctrl: mods.ctrl || mods.command,
            alt: mods.alt,
            shift: mods.shift,
            cmd: false,
        }
    };
    Keystroke::new(mods, name)
}

/// Feeds this frame's key presses to the keymap; matched presses are consumed
/// and their commands queued. Skipped while a text field has focus so typing
/// and text-editing keys reach the field — except F1 (context help), which no
/// text field uses.
pub fn dispatch_keys(ctx: &egui::Context, core: &mut AppCore, context: &ContextSnapshot) {
    // Other text fields (picker filter…) keep their keys; the editor does not.
    // The Explorer tree takes focus too but leaves its shortcuts to the keymap.
    let explorer_tree = ctx.memory(|m| m.has_focus(crate::shell::explorer::tree_id()));
    let text_field = ctx.egui_wants_keyboard_input() && !core.editor_focused && !explorer_tree;
    if text_field {
        core.keymap.cancel_pending();
    }
    let platform = core.platform;
    let presses: Vec<(egui::Key, egui::Modifiers)> = ctx.input(|i| {
        i.events
            .iter()
            .filter_map(|e| match e {
                egui::Event::Key {
                    key,
                    pressed: true,
                    repeat: false,
                    modifiers,
                    ..
                } if !text_field || (*key == egui::Key::F1 && modifiers.is_none()) => {
                    Some((*key, *modifiers))
                }
                _ => None,
            })
            .collect()
    });
    for (key, mods) in presses {
        let Some(stroke) = keystroke(key, mods, platform) else {
            continue;
        };
        match core.keymap.press(stroke, context) {
            KeyResult::Command { command, args } => {
                ctx.input_mut(|i| i.consume_key(mods, key));
                core.enqueue(
                    CommandInvocation::new(command, InvocationSource::Keyboard).with_args(args),
                );
            }
            KeyResult::Pending(_) => {
                ctx.input_mut(|i| i.consume_key(mods, key));
            }
            KeyResult::NoMatch => {}
        }
    }
}

/// macOS Control-click is a secondary click. winit reports it as a primary
/// press with Control held, so the press and its release are rewritten here.
#[derive(Default)]
pub struct CtrlClick {
    active: bool,
}

impl CtrlClick {
    pub fn apply(&mut self, raw: &mut egui::RawInput) {
        for event in &mut raw.events {
            if let egui::Event::PointerButton {
                button,
                pressed,
                modifiers,
                ..
            } = event
                && *button == egui::PointerButton::Primary
            {
                if *pressed {
                    self.active = modifiers.ctrl && !modifiers.mac_cmd;
                }
                if self.active {
                    *button = egui::PointerButton::Secondary;
                    modifiers.ctrl = false;
                }
                if !*pressed {
                    self.active = false;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn button(pressed: bool, ctrl: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos: egui::pos2(1.0, 1.0),
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers {
                ctrl,
                ..Default::default()
            },
        }
    }

    fn buttons(raw: &egui::RawInput) -> Vec<egui::PointerButton> {
        raw.events
            .iter()
            .filter_map(|e| match e {
                egui::Event::PointerButton { button, .. } => Some(*button),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn ctrl_click_becomes_secondary_until_release() {
        use egui::PointerButton::{Primary, Secondary};
        let mut remap = CtrlClick::default();
        // Control released before the button: the release still pairs.
        let mut raw = egui::RawInput {
            events: vec![button(true, true)],
            ..Default::default()
        };
        remap.apply(&mut raw);
        assert_eq!(buttons(&raw), [Secondary]);
        raw.events = vec![button(false, false)];
        remap.apply(&mut raw);
        assert_eq!(buttons(&raw), [Secondary]);
        // Plain clicks are untouched.
        raw.events = vec![button(true, false), button(false, false)];
        remap.apply(&mut raw);
        assert_eq!(buttons(&raw), [Primary, Primary]);
    }

    #[test]
    fn egui_keys_map_to_keystrokes() {
        let cmd = egui::Modifiers {
            mac_cmd: true,
            command: true,
            ..Default::default()
        };
        let k = keystroke(egui::Key::B, cmd, Platform::Mac).unwrap();
        assert_eq!(k.display(Platform::Mac), "⌘B");
        let ctrl = egui::Modifiers {
            ctrl: true,
            command: true,
            ..Default::default()
        };
        assert_eq!(
            keystroke(egui::Key::Equals, ctrl, Platform::Windows)
                .unwrap()
                .display(Platform::Windows),
            "Ctrl+="
        );
        assert_eq!(
            keystroke(egui::Key::ArrowUp, egui::Modifiers::NONE, Platform::Linux)
                .unwrap()
                .key,
            "Up"
        );
        assert_eq!(
            keystroke(egui::Key::F11, egui::Modifiers::NONE, Platform::Linux)
                .unwrap()
                .key,
            "F11"
        );
    }
}
