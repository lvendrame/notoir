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
/// and text-editing keys reach the field.
pub fn dispatch_keys(ctx: &egui::Context, core: &mut AppCore, context: &ContextSnapshot) {
    // Other text fields (picker filter…) keep their keys; the editor does not.
    if ctx.egui_wants_keyboard_input() && !core.editor_focused {
        core.keymap.cancel_pending();
        return;
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
                } => Some((*key, *modifiers)),
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

#[cfg(test)]
mod tests {
    use super::*;

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
