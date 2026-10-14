//! Start screen: create a new workspace or open an existing / recent one.

use egui::{Align, Color32, CornerRadius, Frame, Layout, Margin, RichText, Sense, Stroke, Ui, Vec2};

use super::{ACCENT, ERR, NAVY, NAVY_LIGHT};
use crate::app::{App, Switch, WORKSPACE_EXT};
use crate::recent;

fn card(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui)) {
    Frame::new()
        .fill(ui.visuals().panel_fill)
        .stroke(Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::same(16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).size(17.0).strong());
            ui.add_space(8.0);
            add(ui);
        });
}

fn big_button(ui: &mut Ui, text: &str, primary: bool) -> egui::Response {
    let b = egui::Button::new(RichText::new(text).size(15.0).color(if primary {
        Color32::WHITE
    } else {
        ui.visuals().text_color()
    }))
    .min_size(Vec2::new(ui.available_width(), 38.0))
    .corner_radius(CornerRadius::same(6));
    ui.add(if primary { b.fill(NAVY) } else { b })
}

pub fn show(app: &mut App, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    // ---- header band ----------------------------------------------------------
    egui::Panel::top("ct_start_top")
        .frame(Frame::new().fill(NAVY).inner_margin(Margin::symmetric(24, 18)))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                super::logo(ui);
                ui.add_space(16.0);
                ui.label(
                    RichText::new("Fast curves from your CSV / Excel measurement files").color(Color32::from_gray(200)),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(RichText::new(format!("v{}", env!("CARGO_PKG_VERSION"))).color(Color32::from_gray(170)));
                });
            });
        });

    egui::CentralPanel::default().show(ui, |ui| {
        let w = ui.available_width().min(1080.0);
        let side = ((ui.available_width() - w) * 0.5).max(0.0);
        ui.add_space(26.0);
        ui.horizontal_top(|ui| {
            ui.add_space(side);
            ui.vertical(|ui| {
                ui.set_width(w);
                ui.label(RichText::new("Choose a workspace").size(24.0).strong());
                ui.label(
                    RichText::new(
                        "A workspace (.ctw file) keeps your data files, user functions, plots and export settings.",
                    )
                    .weak(),
                );
                ui.add_space(14.0);
                ui.columns(2, |cols| {
                    new_and_open(app, &mut cols[0], &ctx);
                    recent_list(app, &mut cols[1], &ctx);
                });
            });
        });
    });
}

/// Workspace name usable as a file name (spaces kept, forbidden characters removed).
fn file_name_safe(name: &str) -> String {
    let t: String = name
        .chars()
        .map(|c| {
            if "<>:\"/\\|?*".contains(c) || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    t.trim().trim_end_matches('.').trim().to_owned()
}

fn new_and_open(app: &mut App, ui: &mut Ui, ctx: &egui::Context) {
    card(ui, "+  New workspace", |ui| {
        let mut submitted = false;
        egui::Grid::new("new_ws")
            .num_columns(2)
            .spacing([10.0, 8.0])
            .show(ui, |ui| {
                ui.label("Name");
                let r = ui.add(
                    egui::TextEdit::singleline(&mut app.start.new_name)
                        .hint_text("e.g. CGY2272 S-parameters")
                        .desired_width(ui.available_width()),
                );
                if r.changed() {
                    app.start.error = None;
                }
                submitted = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.end_row();
                ui.label("Folder");
                ui.horizontal(|ui| {
                    if ui.button("📂 Browse…").clicked() {
                        let mut dlg = rfd::FileDialog::new().set_title("Folder of the new workspace");
                        if app.start.new_dir.is_dir() {
                            dlg = dlg.set_directory(&app.start.new_dir);
                        }
                        if let Some(d) = dlg.pick_folder() {
                            app.start.new_dir = d;
                        }
                    }
                    ui.add(egui::Label::new(RichText::new(app.start.new_dir.display().to_string()).small()).truncate());
                });
                ui.end_row();
            });
        let name = file_name_safe(&app.start.new_name);
        let path = app.start.new_dir.join(format!("{name}.{WORKSPACE_EXT}"));
        if !name.is_empty() {
            ui.label(RichText::new(format!("→ {}", path.display())).small().weak());
        }
        ui.add_space(6.0);
        let create = ui
            .add_enabled_ui(!name.is_empty(), |ui| big_button(ui, "Create the workspace", true))
            .inner;
        if create.clicked() || (submitted && !name.is_empty()) {
            match app.create_workspace(&path) {
                Ok(()) => app.start.new_name.clear(),
                Err(e) => app.start.error = Some(e),
            }
        }
        if let Some(e) = &app.start.error {
            ui.label(RichText::new(format!("⚠ {e}")).color(ERR));
        }
    });
    ui.add_space(12.0);
    card(ui, "📂  Open a workspace", |ui| {
        ui.label(RichText::new("Pick a .ctw file in one of your folders (or drop it on the window).").weak());
        ui.add_space(6.0);
        if big_button(ui, "Open a workspace file…", false).clicked() {
            app.request_switch(Switch::OpenDialog, ctx);
        }
        ui.add_space(8.0);
        let back = app.workspace_name();
        let label = match &back {
            Some(n) => format!("← Back to “{n}”"),
            None if !app.sources.is_empty() || !app.plots.is_empty() => "← Back to the current session".to_owned(),
            None => "Continue without a workspace".to_owned(),
        };
        if ui
            .link(label)
            .on_hover_text("You can save the workspace later (File → Save workspace)")
            .clicked()
        {
            app.show_start = false;
        }
    });
}

fn recent_list(app: &mut App, ui: &mut Ui, ctx: &egui::Context) {
    card(ui, "🕘  Recent workspaces", |ui| {
        if app.recent.is_empty() {
            ui.label(RichText::new("The workspaces you create or open will be listed here.").weak());
            return;
        }
        let mut open = None;
        let mut remove = None;
        egui::ScrollArea::vertical()
            .id_salt("recent_ws")
            .max_height(430.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for (i, r) in app.recent.iter().enumerate() {
                    let exists = r.exists();
                    let bg = ui.painter().add(egui::Shape::Noop);
                    let current = app
                        .workspace_path
                        .as_ref()
                        .is_some_and(|p| std::fs::canonicalize(p).map(|c| c == r.path).unwrap_or(*p == r.path));
                    let row = Frame::new()
                        .inner_margin(Margin::symmetric(10, 7))
                        .corner_radius(CornerRadius::same(6))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("📄").size(20.0).color(if exists {
                                    NAVY_LIGHT
                                } else {
                                    Color32::GRAY
                                }));
                                ui.vertical(|ui| {
                                    ui.horizontal(|ui| {
                                        let mut t = RichText::new(r.name()).size(15.0).strong();
                                        if !exists {
                                            t = t.weak().strikethrough();
                                        }
                                        ui.label(t);
                                        if current {
                                            ui.label(RichText::new("open").small().color(ACCENT));
                                        }
                                    });
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(if exists {
                                                format!("{}  ·  {}", r.path.display(), recent::ago(r.last))
                                            } else {
                                                format!("{}  ·  file not found", r.path.display())
                                            })
                                            .small()
                                            .weak(),
                                        )
                                        .truncate(),
                                    );
                                });
                            });
                        })
                        .response;
                    let resp = ui.interact(row.rect, ui.id().with(("recent", i)), Sense::click());
                    if resp.hovered() {
                        let fill = ui.visuals().widgets.hovered.weak_bg_fill;
                        ui.painter().set(bg, egui::Shape::rect_filled(row.rect, 6.0, fill));
                    }
                    let resp = resp.on_hover_text(if exists {
                        "Open this workspace"
                    } else {
                        "File not found (moved or deleted)"
                    });
                    if resp.clicked() && exists {
                        open = Some(r.path.clone());
                    }
                    resp.context_menu(|ui| {
                        if ui.button("Remove from the list").clicked() {
                            remove = Some(r.path.clone());
                            ui.close();
                        }
                    });
                    if !exists {
                        let x = egui::Rect::from_center_size(
                            row.rect.right_center() - egui::vec2(18.0, 0.0),
                            Vec2::splat(20.0),
                        );
                        if ui
                            .put(x, egui::Button::new("🗙").small())
                            .on_hover_text("Remove from the list")
                            .clicked()
                        {
                            remove = Some(r.path.clone());
                        }
                    }
                }
            });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui.small_button("Clear the list").clicked() {
                recent::clear(&mut app.recent);
            }
        });
        if let Some(p) = remove {
            recent::remove(&mut app.recent, &p);
        }
        if let Some(p) = open {
            app.request_switch(Switch::Open(p), ctx);
        }
    });
}

/// "Save the changes?" dialog shown before changing of workspace.
pub fn pending_switch_dialog(app: &mut App, ctx: &egui::Context) {
    let Some(sw) = app.pending_switch.clone() else {
        return;
    };
    let name = app.workspace_name().unwrap_or_else(|| "this session".to_owned());
    let mut decision: Option<u8> = None;
    egui::Window::new("Unsaved changes")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
        .show(ctx, |ui| {
            ui.label(format!("Save the changes of “{name}” before leaving it?"));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(RichText::new("💾 Save").strong()).clicked() {
                    decision = Some(0);
                }
                if ui.button("Don't save").clicked() {
                    decision = Some(1);
                }
                if ui.button("Cancel").clicked() {
                    decision = Some(2);
                }
            });
        });
    match decision {
        Some(0) => {
            app.save_workspace_dialog(app.workspace_path.is_none());
            if !app.is_dirty() {
                app.perform_switch(sw, ctx);
            }
        }
        Some(1) => app.perform_switch(sw, ctx),
        Some(2) => app.pending_switch = None,
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn workspace_file_names() {
        assert_eq!(super::file_name_safe("  CGY2272 S-params  "), "CGY2272 S-params");
        assert_eq!(super::file_name_safe("a/b:c?"), "a_b_c_");
        assert_eq!(super::file_name_safe(" .. "), "");
    }
}
