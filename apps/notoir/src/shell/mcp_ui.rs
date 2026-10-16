//! MCP UI (docs/specs/29 §Activity & Connections UI, 32 §Ask Prompt,
//! §Permissions UI): status bar item and popover, the non-modal permission
//! prompt above the status bar, the Connections and Permissions dialogs, and
//! the Activity panel.

use egui::{Align, Align2, Key, Layout, RichText, Stroke, Vec2};
use notoir_core::command::{CommandInvocation, InvocationSource};
use notoir_core::permissions::{Decision, MCP_MATRIX, Principal, RiskClass};
use serde_json::{Value, json};

use super::{Actions, Kit};
use crate::core::AppCore;
use crate::mcp::{ActivityEntry, McpDialog};
use crate::t;
use crate::theme::Tok;

fn cmd(actions: &mut Actions, id: &'static str, args: Value) {
    actions.push(CommandInvocation::new(id, InvocationSource::Internal).with_args(args));
}

/// `HH:MM:SS` (UTC) of a Unix-millisecond time.
fn hms(at_ms: i64) -> String {
    let s = (at_ms / 1000).rem_euclid(86_400);
    format!("{:02}:{:02}:{:02}", s / 3600, s % 3600 / 60, s % 60)
}

/// `YYYY-MM-DD HH:MM` (UTC).
fn date_time(at_ms: i64) -> String {
    let secs = at_ms / 1000;
    let s = secs.rem_euclid(86_400);
    format!(
        "{} {:02}:{:02}",
        super::overlays::ymd(secs),
        s / 3600,
        s % 3600 / 60
    )
}

fn outcome_text(o: &str) -> String {
    match o {
        "ok" => t!("mcp:activity.ok"),
        "denied" => t!("mcp:activity.denied"),
        o => match o.strip_prefix("rejected: ") {
            Some(reason) => t!("mcp:activity.rejected", reason = reason),
            None => o.to_owned(),
        },
    }
}

fn outcome_color(kit: &Kit, o: &str) -> egui::Color32 {
    if o == "ok" {
        kit.color(Tok::TextSecondary)
    } else if o == "denied" || o.starts_with("rejected") {
        kit.color(Tok::Danger)
    } else {
        kit.color(Tok::Warning)
    }
}

fn decision_label(d: Decision) -> String {
    match d {
        Decision::Allow => t!("mcp:perm.allow"),
        Decision::Ask => t!("mcp:perm.ask"),
        Decision::Deny => t!("mcp:perm.deny"),
    }
}

// ── status bar ───────────────────────────────────────────────────────────

/// `MCP ●2` while the server runs; click opens the popover.
pub fn status_item(ui: &mut egui::Ui, kit: &mut Kit, core: &AppCore, actions: &mut Actions) {
    if !core.mcp.running() {
        return;
    }
    let n = core.mcp.connected_count();
    let color = if n > 0 {
        kit.color(Tok::Accent)
    } else {
        kit.color(Tok::TextSecondary)
    };
    // `MCP ●2`: the UI fonts have no "●", so the dot is painted.
    let text = t!("mcp:status", count = n);
    let (before, after) = text.split_once('●').unwrap_or((text.as_str(), ""));
    let font = egui::FontId::proportional(12.0);
    let (g1, g2) = ui.fonts_mut(|f| {
        (
            f.layout_no_wrap(before.trim_end().to_owned(), font.clone(), color),
            f.layout_no_wrap(after.trim_start().to_owned(), font.clone(), color),
        )
    });
    const DOT: f32 = 6.0;
    const GAP: f32 = 4.0;
    let size = egui::vec2(
        g1.size().x + GAP + DOT + GAP + g2.size().x,
        g1.size().y.max(g2.size().y),
    );
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &text));
    let painter = ui.painter();
    let y = rect.center().y;
    painter.galley(
        egui::pos2(rect.left(), y - g1.size().y / 2.0),
        g1.clone(),
        color,
    );
    let dot_x = rect.left() + g1.size().x + GAP + DOT / 2.0;
    painter.circle_filled(egui::pos2(dot_x, y), DOT / 2.0, color);
    painter.galley(
        egui::pos2(dot_x + DOT / 2.0 + GAP, y - g2.size().y / 2.0),
        g2,
        color,
    );
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    let tip = t!("mcp:statusA11y", count = n);
    if resp.on_hover_text(tip).clicked() {
        cmd(actions, "mcp.togglePopover", json!({}));
    }
    ui.add_space(8.0);
}

pub fn popover(
    ctx: &egui::Context,
    kit: &mut Kit,
    core: &AppCore,
    bottom_offset: f32,
    actions: &mut Actions,
) {
    if !core.mcp.popover || !core.mcp.running() {
        return;
    }
    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
        cmd(actions, "mcp.togglePopover", json!({}));
        return;
    }
    let screen = ctx.content_rect();
    egui::Area::new(egui::Id::new("mcp-popover"))
        .order(egui::Order::Foreground)
        .anchor(Align2::LEFT_BOTTOM, Vec2::new(8.0, -(bottom_offset + 4.0)))
        .constrain_to(screen)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style())
                .fill(kit.color(Tok::BgPanelRaised))
                .inner_margin(egui::Margin::same(12))
                .show(ui, |ui| {
                    ui.set_width(340.0);
                    if let Some(url) = core.mcp.url() {
                        ui.label(
                            RichText::new(t!("mcp:popover.url", url = url))
                                .font(crate::fonts::mono(12.0))
                                .color(kit.color(Tok::TextPrimary)),
                        );
                    }
                    ui.add_space(8.0);
                    super::section_label(ui, kit, &t!("mcp:popover.clients"));
                    let connected: Vec<_> = core
                        .mcp
                        .clients
                        .iter()
                        .filter(|c| core.mcp.connected.get(&c.id).is_some_and(|n| *n > 0))
                        .collect();
                    if connected.is_empty() {
                        ui.label(
                            RichText::new(t!("mcp:popover.none"))
                                .size(12.0)
                                .color(kit.color(Tok::TextTertiary)),
                        );
                    }
                    for c in connected {
                        let seen = c.last_seen.map(hms).unwrap_or_default();
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(&c.name)
                                    .size(12.0)
                                    .color(kit.color(Tok::TextPrimary)),
                            );
                            ui.label(
                                RichText::new(seen)
                                    .font(crate::fonts::mono(11.0))
                                    .color(kit.color(Tok::TextTertiary)),
                            );
                        });
                    }
                    ui.add_space(8.0);
                    super::section_label(ui, kit, &t!("mcp:popover.recent"));
                    if core.mcp.activity.is_empty() {
                        ui.label(
                            RichText::new(t!("mcp:activity.empty"))
                                .size(12.0)
                                .color(kit.color(Tok::TextTertiary)),
                        );
                    }
                    for e in core.mcp.activity.iter().take(5) {
                        activity_row(ui, kit, core, e, false, actions);
                    }
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui.button(t!("mcp:popover.connections")).clicked() {
                            cmd(actions, "mcp.connections", json!({}));
                        }
                        if ui.button(t!("mcp:popover.activity")).clicked() {
                            cmd(actions, "mcp.activity", json!({}));
                        }
                        if ui.button(t!("commands:mcp.stopServer")).clicked() {
                            cmd(actions, "mcp.stopServer", json!({}));
                        }
                    });
                });
        });
}

// ── permission prompt ────────────────────────────────────────────────────

/// The front prompt, anchored above the status bar. Non-modal: the editor
/// keeps focus unless F6 (or a click) moves it here.
pub fn ask_bar(
    ctx: &egui::Context,
    kit: &mut Kit,
    core: &mut AppCore,
    bottom_offset: f32,
    actions: &mut Actions,
) {
    let focus = std::mem::take(&mut core.mcp.focus_prompt);
    let Some(p) = core.mcp.prompts.front() else {
        return;
    };
    let client = core.mcp.client_name(&p.client);
    let what = t!(&p.capability.label_key());
    let remaining = p.deadline.saturating_sub(core.now_ms).div_ceil(1000);
    let more = core.mcp.prompts.len() - 1;
    let dangerous = p.capability.risk() == RiskClass::Dangerous;
    let screen = ctx.content_rect();
    egui::Area::new(egui::Id::new("mcp-ask"))
        .order(egui::Order::Foreground)
        .anchor(
            Align2::CENTER_BOTTOM,
            Vec2::new(0.0, -(bottom_offset + 8.0)),
        )
        .constrain_to(screen)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style())
                .fill(kit.color(Tok::BgPanelRaised))
                .stroke(Stroke::new(1.0, kit.color(Tok::Warning)))
                .inner_margin(egui::Margin::symmetric(14, 10))
                .show(ui, |ui| {
                    ui.set_max_width((screen.width() - 48.0).min(720.0));
                    ui.horizontal_wrapped(|ui| {
                        ui.add(kit.icon(
                            ui.ctx(),
                            crate::icons::Icon::ShieldAlert,
                            crate::theme::ICON_SM,
                            kit.color(Tok::Warning),
                        ));
                        let title = match &p.target {
                            Some(target) => t!(
                                "mcp:prompt.wantsTarget",
                                client = client.as_str(),
                                what = what.as_str(),
                                target = target.as_str()
                            ),
                            None => t!(
                                "mcp:prompt.wants",
                                client = client.as_str(),
                                what = what.as_str()
                            ),
                        };
                        ui.label(
                            RichText::new(title)
                                .size(13.0)
                                .color(kit.color(Tok::TextPrimary)),
                        );
                    });
                    if p.confirm_always {
                        ui.label(
                            RichText::new(t!("mcp:prompt.dangerConfirm"))
                                .size(12.0)
                                .color(kit.color(Tok::Danger)),
                        );
                    }
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        let once = ui.button(t!("mcp:prompt.once"));
                        if focus {
                            once.request_focus();
                        }
                        if once.clicked() {
                            cmd(actions, "mcp.answerPrompt", json!({ "answer": "once" }));
                        }
                        if ui.button(t!("mcp:prompt.session")).clicked() {
                            cmd(actions, "mcp.answerPrompt", json!({ "answer": "session" }));
                        }
                        let always = if p.confirm_always {
                            RichText::new(t!("mcp:prompt.confirmAlways"))
                                .color(kit.color(Tok::Danger))
                        } else if dangerous {
                            RichText::new(t!("mcp:prompt.always")).color(kit.color(Tok::Danger))
                        } else {
                            RichText::new(t!("mcp:prompt.always"))
                        };
                        if ui.button(always).clicked() {
                            cmd(actions, "mcp.answerPrompt", json!({ "answer": "always" }));
                        }
                        if ui.button(t!("mcp:prompt.deny")).clicked() {
                            cmd(actions, "mcp.answerPrompt", json!({ "answer": "deny" }));
                        }
                        let mut status = t!("mcp:prompt.remaining", seconds = remaining);
                        if more > 0 {
                            status.push_str("  ");
                            status.push_str(&t!("mcp:prompt.more", count = more));
                        }
                        ui.label(
                            RichText::new(status)
                                .size(11.0)
                                .color(kit.color(Tok::TextTertiary)),
                        );
                    });
                });
        });
}

// ── dialogs ──────────────────────────────────────────────────────────────

/// Connections, Add Client and Permissions. `name` is the Add Client
/// field's text (UI state).
pub fn dialogs(
    ctx: &egui::Context,
    kit: &mut Kit,
    core: &AppCore,
    name: &mut String,
    actions: &mut Actions,
) {
    let Some(dialog) = core.mcp.dialog.clone() else {
        name.clear();
        return;
    };
    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
        cmd(actions, "mcp.closeDialog", json!({}));
        return;
    }
    let modal = egui::Modal::new(egui::Id::new("mcp-dialog")).show(ctx, |ui| match &dialog {
        McpDialog::Connections => connections(ui, kit, core, actions),
        McpDialog::AddClient(_) => add_client(ui, kit, name, actions),
        McpDialog::Permissions(id) => permissions(ui, kit, core, id, actions),
    });
    if modal.should_close() {
        cmd(actions, "mcp.closeDialog", json!({}));
    }
}

fn title(ui: &mut egui::Ui, kit: &Kit, text: &str) {
    ui.label(
        RichText::new(text)
            .font(crate::fonts::semibold(15.0))
            .color(kit.color(Tok::TextPrimary)),
    );
    ui.add_space(8.0);
}

fn connections(ui: &mut egui::Ui, kit: &mut Kit, core: &AppCore, actions: &mut Actions) {
    ui.set_width(640.0);
    title(ui, kit, &t!("mcp:connections.title"));
    if let Some(new) = &core.mcp.new_client {
        token_box(ui, kit, core, new, actions);
        ui.add_space(10.0);
    }
    match core.mcp.url() {
        Some(url) => {
            ui.label(
                RichText::new(t!("mcp:popover.url", url = url))
                    .font(crate::fonts::mono(12.0))
                    .color(kit.color(Tok::TextSecondary)),
            );
        }
        None => {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(t!("mcp:connections.serverOff"))
                        .size(12.0)
                        .color(kit.color(Tok::TextSecondary)),
                );
                if ui.button(t!("commands:mcp.startServer")).clicked() {
                    cmd(actions, "mcp.startServer", json!({}));
                }
            });
        }
    }
    ui.add_space(8.0);
    if core.mcp.clients.is_empty() {
        ui.label(
            RichText::new(t!("mcp:connections.empty"))
                .size(13.0)
                .color(kit.color(Tok::TextTertiary)),
        );
    } else {
        egui::Grid::new("mcp-clients")
            .striped(true)
            .num_columns(6)
            .spacing([12.0, 6.0])
            .show(ui, |ui| {
                for h in [
                    "mcp:connections.name",
                    "mcp:connections.status",
                    "mcp:connections.lastSeen",
                    "mcp:connections.created",
                    "mcp:connections.permissions",
                ] {
                    ui.label(
                        RichText::new(t!(h))
                            .size(11.0)
                            .color(kit.color(Tok::TextTertiary)),
                    );
                }
                ui.label("");
                ui.end_row();
                for c in &core.mcp.clients {
                    let mut name = c.name.clone();
                    if let Some(r) = c.reported.as_ref().filter(|r| **r != c.name) {
                        name.push_str(&format!(" ({r})"));
                    }
                    ui.label(RichText::new(name).size(12.0));
                    let status = if c.revoked {
                        t!("mcp:connections.revoked")
                    } else if core.mcp.connected.get(&c.id).is_some_and(|n| *n > 0) {
                        t!("mcp:connections.connected")
                    } else {
                        t!("mcp:connections.idle")
                    };
                    ui.label(RichText::new(status).size(12.0));
                    let seen = c
                        .last_seen
                        .map_or_else(|| t!("mcp:connections.never"), date_time);
                    ui.label(RichText::new(seen).font(crate::fonts::mono(11.0)));
                    ui.label(RichText::new(date_time(c.created_at)).font(crate::fonts::mono(11.0)));
                    let (a, q, d) = core.mcp.summary(&c.id);
                    ui.label(
                        RichText::new(t!("mcp:connections.summary", allow = a, ask = q, deny = d))
                            .size(11.0)
                            .color(kit.color(Tok::TextSecondary)),
                    );
                    ui.horizontal(|ui| {
                        if c.revoked {
                            return;
                        }
                        let id = json!({ "client": c.id.0 });
                        if ui.small_button(t!("mcp:connections.edit")).clicked() {
                            cmd(actions, "mcp.permissions", id.clone());
                        }
                        if ui.small_button(t!("mcp:connections.copyConfig")).clicked() {
                            cmd(
                                actions,
                                "mcp.copyConfig",
                                json!({ "client": c.id.0, "kind": "claudeCode" }),
                            );
                        }
                        if ui
                            .small_button(
                                RichText::new(t!("mcp:connections.revoke"))
                                    .color(kit.color(Tok::Danger)),
                            )
                            .clicked()
                        {
                            cmd(actions, "mcp.revokeClient", id);
                        }
                    });
                    ui.end_row();
                }
            });
    }
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        if ui.button(t!("mcp:connections.add")).clicked() {
            cmd(actions, "mcp.addClient", json!({}));
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.button(t!("common:close")).clicked() {
                cmd(actions, "mcp.closeDialog", json!({}));
            }
        });
    });
}

/// The new client's token and config snippets (shown once).
fn token_box(
    ui: &mut egui::Ui,
    kit: &mut Kit,
    core: &AppCore,
    new: &crate::mcp::NewClient,
    actions: &mut Actions,
) {
    egui::Frame::new()
        .fill(kit.color(Tok::BgPanel))
        .stroke(Stroke::new(1.0, kit.color(Tok::Accent)))
        .inner_margin(egui::Margin::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new(t!("mcp:token.title", name = new.name.as_str()))
                    .font(crate::fonts::semibold(13.0))
                    .color(kit.color(Tok::TextPrimary)),
            );
            ui.label(
                RichText::new(t!("mcp:token.once"))
                    .size(12.0)
                    .color(kit.color(Tok::Warning)),
            );
            ui.add_space(4.0);
            ui.label(
                RichText::new(&new.token)
                    .font(crate::fonts::mono(12.0))
                    .color(kit.color(Tok::TextPrimary)),
            );
            let (code, desktop) = core.mcp.snippets(&new.token);
            for (label, kind, snippet) in [
                ("mcp:token.claudeCode", "claudeCode", code),
                ("mcp:token.claudeDesktop", "claudeDesktop", desktop),
            ] {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(t!(label))
                            .size(12.0)
                            .color(kit.color(Tok::TextSecondary)),
                    );
                    if ui.small_button(t!("mcp:token.copy")).clicked() {
                        cmd(
                            actions,
                            "mcp.copyConfig",
                            json!({ "client": new.id.0, "kind": kind }),
                        );
                    }
                });
                ui.label(
                    RichText::new(snippet)
                        .font(crate::fonts::mono(11.0))
                        .color(kit.color(Tok::TextPrimary)),
                );
            }
        });
}

fn add_client(ui: &mut egui::Ui, kit: &mut Kit, name: &mut String, actions: &mut Actions) {
    ui.set_width(420.0);
    title(ui, kit, &t!("mcp:add.title"));
    ui.label(
        RichText::new(t!("mcp:add.name"))
            .size(12.0)
            .color(kit.color(Tok::TextSecondary)),
    );
    let resp = ui.add(
        egui::TextEdit::singleline(name)
            .hint_text(t!("mcp:add.placeholder"))
            .desired_width(f32::INFINITY),
    );
    if !resp.has_focus() && name.is_empty() {
        resp.request_focus();
    }
    let submit = resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
    ui.add_space(12.0);
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        let ok = !name.trim().is_empty();
        if (ui
            .add_enabled(ok, egui::Button::new(t!("mcp:add.create")))
            .clicked()
            || submit)
            && ok
        {
            cmd(actions, "mcp.addClient", json!({ "name": name.trim() }));
            name.clear();
        }
        if ui.button(t!("common:cancel")).clicked() {
            cmd(actions, "mcp.connections", json!({}));
        }
    });
}

fn permissions(
    ui: &mut egui::Ui,
    kit: &mut Kit,
    core: &AppCore,
    id: &notoir_core::permissions::ClientId,
    actions: &mut Actions,
) {
    ui.set_width(560.0);
    let name = core.mcp.client_name(id);
    title(ui, kit, &t!("mcp:perm.title", name = name.as_str()));
    // Client switcher.
    let others: Vec<_> = core.mcp.clients.iter().filter(|c| !c.revoked).collect();
    if others.len() > 1 {
        egui::ComboBox::from_id_salt("mcp-perm-client")
            .selected_text(name.as_str())
            .show_ui(ui, |ui| {
                for c in others {
                    if ui.selectable_label(&c.id == id, &c.name).clicked() {
                        cmd(actions, "mcp.permissions", json!({ "client": c.id.0 }));
                    }
                }
            });
        ui.add_space(6.0);
    }
    let p = Principal::McpClient(id.clone());
    let revoked = core.mcp.gate.is_revoked(&p);
    egui::ScrollArea::vertical()
        .max_height(380.0)
        .show(ui, |ui| {
            egui::Grid::new("mcp-matrix")
                .num_columns(2)
                .spacing([16.0, 4.0])
                .show(ui, |ui| {
                    for (group, caps) in MCP_MATRIX {
                        ui.label(
                            RichText::new(t!(group))
                                .font(crate::fonts::semibold(12.0))
                                .color(kit.color(Tok::TextPrimary)),
                        );
                        ui.end_row();
                        for cap in *caps {
                            let label = t!(&cap.label_key());
                            ui.label(RichText::new(format!("   {label}")).size(12.0));
                            let current = core.mcp.gate.decision(&p, cap);
                            ui.add_enabled_ui(!revoked, |ui| {
                                egui::ComboBox::from_id_salt(("mcp-cap", cap.key()))
                                    .selected_text(decision_label(current))
                                    .width(110.0)
                                    .show_ui(ui, |ui| {
                                        for d in [Decision::Allow, Decision::Ask, Decision::Deny] {
                                            if ui
                                                .selectable_label(current == d, decision_label(d))
                                                .clicked()
                                                && d != current
                                            {
                                                cmd(
                                                    actions,
                                                    "mcp.setPermission",
                                                    json!({
                                                        "client": id.0,
                                                        "capability": cap.key(),
                                                        "decision": d.as_str(),
                                                    }),
                                                );
                                            }
                                        }
                                    });
                            });
                            ui.end_row();
                        }
                    }
                });
        });
    ui.add_space(8.0);
    let session = core.mcp.gate.session_grants(&p);
    ui.horizontal(|ui| {
        let text = if session.is_empty() {
            t!("mcp:perm.sessionNone")
        } else {
            let names: Vec<String> = session.iter().map(|c| t!(&c.label_key())).collect();
            t!("mcp:perm.session", list = names.join(", "))
        };
        ui.label(
            RichText::new(text)
                .size(12.0)
                .color(kit.color(Tok::TextSecondary)),
        );
        if !session.is_empty() && ui.small_button(t!("mcp:perm.clear")).clicked() {
            cmd(actions, "mcp.clearSessionGrants", json!({ "client": id.0 }));
        }
    });
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        if ui.button(t!("mcp:perm.reset")).clicked() {
            cmd(actions, "mcp.resetPermissions", json!({ "client": id.0 }));
        }
        if ui
            .button(RichText::new(t!("mcp:perm.revokeAll")).color(kit.color(Tok::Danger)))
            .clicked()
        {
            cmd(actions, "mcp.denyAll", json!({ "client": id.0 }));
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.button(t!("common:close")).clicked() {
                cmd(actions, "mcp.closeDialog", json!({}));
            }
            if ui.button(t!("mcp:popover.connections")).clicked() {
                cmd(actions, "mcp.connections", json!({}));
            }
        });
    });
}

// ── activity panel ───────────────────────────────────────────────────────

fn activity_row(
    ui: &mut egui::Ui,
    kit: &mut Kit,
    core: &AppCore,
    e: &ActivityEntry,
    wide: bool,
    actions: &mut Actions,
) {
    let client = e
        .client
        .as_ref()
        .map(|c| core.mcp.client_name(c))
        .unwrap_or_else(|| "—".to_owned());
    ui.horizontal(|ui| {
        let mono = crate::fonts::mono(11.0);
        ui.label(
            RichText::new(hms(e.at))
                .font(mono.clone())
                .color(kit.color(Tok::TextTertiary)),
        );
        ui.label(RichText::new(client).size(12.0));
        ui.label(
            RichText::new(&e.tool)
                .font(mono.clone())
                .color(kit.color(Tok::TextPrimary)),
        );
        if let Some(target) = &e.target {
            let link = ui.add(
                egui::Button::new(
                    RichText::new(target)
                        .size(12.0)
                        .color(kit.color(Tok::Accent)),
                )
                .frame(false),
            );
            if link.clicked()
                && let Some(ws) = &core.workspace
            {
                let path = ws.root.join(target);
                if path.is_file() {
                    cmd(
                        actions,
                        "explorer.open",
                        json!({ "path": path.to_string_lossy() }),
                    );
                }
            }
        }
        ui.label(
            RichText::new(outcome_text(&e.outcome))
                .size(12.0)
                .color(outcome_color(kit, &e.outcome)),
        );
        if wide && let Some(ms) = e.duration_ms {
            ui.label(
                RichText::new(format!("{ms} ms"))
                    .font(mono)
                    .color(kit.color(Tok::TextTertiary)),
            );
        }
    });
}

pub fn activity_panel(ui: &mut egui::Ui, kit: &mut Kit, core: &AppCore, actions: &mut Actions) {
    ui.horizontal(|ui| {
        let filter = core
            .mcp
            .activity_filter
            .as_ref()
            .map_or_else(|| t!("mcp:activity.all"), |c| core.mcp.client_name(c));
        egui::ComboBox::from_id_salt("mcp-activity-filter")
            .selected_text(filter)
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(core.mcp.activity_filter.is_none(), t!("mcp:activity.all"))
                    .clicked()
                {
                    cmd(actions, "mcp.filterActivity", json!({}));
                }
                for c in &core.mcp.clients {
                    let on = core.mcp.activity_filter.as_ref() == Some(&c.id);
                    if ui.selectable_label(on, &c.name).clicked() {
                        cmd(actions, "mcp.filterActivity", json!({ "client": c.id.0 }));
                    }
                }
            });
        if ui.button(t!("mcp:activity.clear")).clicked() {
            cmd(actions, "mcp.clearActivity", json!({}));
        }
    });
    ui.add_space(4.0);
    let rows: Vec<&ActivityEntry> = core
        .mcp
        .activity
        .iter()
        .filter(|e| {
            core.mcp
                .activity_filter
                .as_ref()
                .is_none_or(|f| e.client.as_ref() == Some(f))
        })
        .collect();
    if rows.is_empty() {
        ui.label(
            RichText::new(t!("mcp:activity.empty"))
                .size(12.0)
                .color(kit.color(Tok::TextTertiary)),
        );
        return;
    }
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for e in rows {
                activity_row(ui, kit, core, e, true, actions);
            }
        });
}
