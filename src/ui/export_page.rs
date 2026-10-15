//! "Export" page: plot thumbnails, slide titles, PowerPoint / PNG / SVG / CSV export.

use std::sync::Arc;

use egui::{Align2, Color32, FontId, RichText, Sense, Ui, Vec2};

use super::{NAVY, section};
use crate::app::{App, ImageExport, ImageSize};
use crate::export::pptx::SlideFormat;
use crate::plot::render::ImageFormat;
use crate::plot::view::show_thumbnail;

pub fn show(app: &mut App, ui: &mut Ui) {
    ui.horizontal(|ui| {
        ui.heading("Export Plots");
        ui.add_space(12.0);
        ui.label("Slide format:");
        egui::ComboBox::from_id_salt("slide_format")
            .selected_text(app.export.format.label())
            .show_ui(ui, |ui| {
                for f in [SlideFormat::PlotOnly, SlideFormat::PlotAndSettings] {
                    ui.selectable_value(&mut app.export.format, f, f.label());
                }
            });
        ui.add_space(12.0);
        ui.label("Image size:");
        size_editor(ui, &mut app.export.size, None);
        if ui
            .add_enabled(
                app.export.size != ImageSize::STANDARD,
                egui::Button::new("6 × 4.2").small(),
            )
            .on_hover_text("Standard format of the reports: 6 × 4.2 in, 300 dpi")
            .clicked()
        {
            app.export.size = ImageSize::STANDARD;
        }
        ui.checkbox(&mut app.export.ask_size, "Ask at each export");
        ui.checkbox(&mut app.export.use_view, "Use the current zoom of each plot");
    });
    if !app.export.size.standard_ratio() {
        ui.label(
            RichText::new("The size is not in the 6 : 4.2 ratio: the format will be asked at each export.")
                .color(super::WARN)
                .small(),
        );
    }
    ui.add_space(4.0);

    let fonts = Arc::clone(&app.fonts);
    let n = app.plots.len();
    if n == 0 {
        ui.label("No plot to export yet.");
        return;
    }
    let avail = ui.available_width();
    let thumbs_h = (ui.available_height() - 150.0).max(220.0);
    let cols = ((avail / 330.0).floor() as usize).clamp(1, 6);
    let w = (avail - 12.0 * cols as f32) / cols as f32;
    // miniatures of the exported images: same proportions and same relative
    // text / line sizes as the PNG / SVG / slide images
    let size = app.export.size;
    let o = crate::plot::render::RenderOptions::physical(size.w_in, size.h_in, size.dpi as f32);
    let (ew, eh) = (o.width.max(1) as f32, o.height.max(1) as f32);
    let h = (w * eh / ew).clamp(w * 0.3, w * 1.5);
    let thumb_scale = w / (ew / o.scale);
    let mut remove: Option<usize> = None;
    let mut open: Option<usize> = None;
    egui::Frame::group(ui.style()).show(ui, |ui| {
        egui::ScrollArea::vertical()
            .id_salt("export_thumbs")
            .max_height(thumbs_h)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                egui::Grid::new("thumb_grid").spacing([10.0, 10.0]).show(ui, |ui| {
                    for i in 0..n {
                        ui.vertical(|ui| {
                            ui.set_width(w);
                            let (rect, _) = ui.allocate_exact_size(Vec2::new(w, h), Sense::hover());
                            let p = &mut app.plots[i];
                            show_thumbnail(ui, rect, &mut p.cfg, &mut p.state, &fonts, thumb_scale);
                            // on top of the (non interactive) plot
                            let resp = ui.interact(rect, ui.id().with(("thumb", i)), Sense::click());
                            let painter = ui.painter();
                            if !p.cfg.export {
                                painter.rect_filled(rect, 4.0, Color32::from_white_alpha(170));
                            }
                            painter.circle_filled(rect.left_top() + Vec2::new(14.0, 14.0), 11.0, NAVY);
                            painter.text(
                                rect.left_top() + Vec2::new(14.0, 14.0),
                                Align2::CENTER_CENTER,
                                (i + 1).to_string(),
                                FontId::proportional(12.0),
                                Color32::WHITE,
                            );
                            let close = egui::Rect::from_center_size(
                                rect.right_top() + Vec2::new(-14.0, 14.0),
                                Vec2::splat(22.0),
                            );
                            let cresp = ui.interact(close, ui.id().with(("thumb_close", i)), Sense::click());
                            painter.circle_filled(
                                close.center(),
                                10.0,
                                if cresp.hovered() {
                                    Color32::from_rgb(190, 50, 50)
                                } else {
                                    Color32::from_gray(90)
                                },
                            );
                            painter.text(
                                close.center(),
                                Align2::CENTER_CENTER,
                                "🗙",
                                FontId::proportional(11.0),
                                Color32::WHITE,
                            );
                            if cresp.clicked() {
                                remove = Some(i);
                            }
                            if resp.double_clicked() {
                                open = Some(i);
                            }
                            resp.on_hover_text("Double-click to open the plot");
                            ui.horizontal(|ui| {
                                ui.checkbox(&mut p.cfg.export, "");
                                ui.add(
                                    egui::TextEdit::singleline(&mut p.cfg.slide_title)
                                        .hint_text(format!("Slide title ({})", p.cfg.name))
                                        .desired_width(w - 34.0),
                                );
                            });
                        });
                        if (i + 1) % cols == 0 {
                            ui.end_row();
                        }
                    }
                });
            });
    });
    if let Some(i) = remove {
        app.plots[i].cfg.export = false;
    }
    if let Some(i) = open {
        app.active = i;
        app.page = crate::app::Page::Plots;
    }

    ui.add_space(8.0);
    let selected = app.plots.iter().filter(|p| p.cfg.export).count();
    section(ui, &format!("Export {selected} plot(s)"), |ui| {
        ui.horizontal(|ui| {
            ui.label("File name");
            ui.add(egui::TextEdit::singleline(&mut app.export.file_name).desired_width(320.0));
        });
        ui.horizontal(|ui| {
            ui.label("Images");
            for f in ImageFormat::ALL {
                ui.radio_value(&mut app.export.image_format, f, f.label());
            }
            ui.add_space(16.0);
            ui.checkbox(&mut app.export.pptx_svg, "Vector (SVG) plots in the PowerPoint")
                .on_hover_text(
                    "The slides embed each plot as an SVG image (sharp at any zoom, can be converted to shapes in PowerPoint) \
                     with a PNG copy for the applications that do not read SVG.",
                );
        });
        ui.add_space(4.0);
        let busy = app.busy.is_some();
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !busy && selected > 0,
                    egui::Button::new(RichText::new("📊  Download PowerPoint").strong().color(Color32::WHITE))
                        .fill(NAVY)
                        .min_size(Vec2::new(240.0, 34.0)),
                )
                .clicked()
            {
                app.request_image_export(ImageExport::Pptx);
            }
            if ui
                .add_enabled(
                    !busy && selected > 0,
                    egui::Button::new(format!(
                        "🖼  Download Images ({})",
                        app.export.image_format.ext().to_uppercase()
                    ))
                    .min_size(Vec2::new(200.0, 34.0)),
                )
                .clicked()
            {
                app.request_image_export(ImageExport::Images);
            }
            if ui
                .add_enabled(
                    !busy && selected > 0,
                    egui::Button::new("⬇  Download Data (CSV)").min_size(Vec2::new(200.0, 34.0)),
                )
                .clicked()
            {
                app.export_csv_dialog();
            }
            if busy {
                ui.spinner();
            }
        });
        ui.label(
            RichText::new("Each selected plot becomes one 16:9 slide: title + the plot at its size (6 × 4.2 in by default), plus the plot settings in “Plot + Settings” format.")
                .weak()
                .small(),
        );
    });
}

/// Width × height (inches) and resolution of the images. `keep`: the height
/// follows the width in the 6 : 4.2 ratio.
fn size_editor(ui: &mut Ui, size: &mut ImageSize, keep: Option<&mut bool>) {
    let keep_ratio = keep.as_ref().is_some_and(|k| **k);
    let w = ui.add(
        egui::DragValue::new(&mut size.w_in)
            .range(1.0..=20.0)
            .speed(0.05)
            .max_decimals(2)
            .suffix(" in"),
    );
    if w.changed() && keep_ratio {
        size.h_in = (size.w_in / ImageSize::RATIO * 100.0).round() / 100.0;
    }
    ui.label("×");
    let h = ui.add_enabled(
        !keep_ratio,
        egui::DragValue::new(&mut size.h_in)
            .range(1.0..=20.0)
            .speed(0.05)
            .max_decimals(2)
            .suffix(" in"),
    );
    let _ = h;
    if let Some(k) = keep
        && ui
            .checkbox(k, "6 : 4.2")
            .on_hover_text("Keep the 6 : 4.2 ratio")
            .changed()
        && *k
    {
        size.h_in = (size.w_in / ImageSize::RATIO * 100.0).round() / 100.0;
    }
    ui.add(
        egui::DragValue::new(&mut size.dpi)
            .range(72..=1200)
            .speed(2.0)
            .suffix(" dpi"),
    );
    let (pw, ph) = size.pixels();
    ui.label(RichText::new(format!("{pw} × {ph} px")).weak().small());
}

/// "Image format" dialog shown before an export of images when the format
/// is not the 6 : 4.2 one (or when "Ask at each export" is checked).
pub fn size_dialog(app: &mut App, ctx: &egui::Context) {
    let Some((what, mut size, mut keep)) = app.size_dialog else {
        return;
    };
    let mut decision = None;
    egui::Window::new("Image format")
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
        .show(ctx, |ui| {
            let what_text = match what {
                ImageExport::Pptx => "PowerPoint report".to_owned(),
                ImageExport::Images => format!("images ({})", app.export.image_format.ext().to_uppercase()),
                ImageExport::Plot(_, f) => format!("{} image", f.ext().to_uppercase()),
            };
            ui.label(format!("Size of the plots of the {what_text}:"));
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                for (label, s) in [
                    ("6 × 4.2 in (standard)", ImageSize::STANDARD),
                    (
                        "8 × 5.6 in",
                        ImageSize {
                            w_in: 8.0,
                            h_in: 5.6,
                            dpi: size.dpi,
                        },
                    ),
                    (
                        "4 × 2.8 in",
                        ImageSize {
                            w_in: 4.0,
                            h_in: 2.8,
                            dpi: size.dpi,
                        },
                    ),
                ] {
                    if ui
                        .selectable_label(size.w_in == s.w_in && size.h_in == s.h_in, label)
                        .clicked()
                    {
                        size.w_in = s.w_in;
                        size.h_in = s.h_in;
                        keep = true;
                    }
                }
            });
            ui.horizontal(|ui| size_editor(ui, &mut size, Some(&mut keep)));
            ui.checkbox(&mut app.export.ask_size, "Ask at each export");
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui
                    .add(egui::Button::new(RichText::new("Export").strong().color(Color32::WHITE)).fill(NAVY))
                    .clicked()
                {
                    decision = Some(true);
                }
                if ui.button("Cancel").clicked() {
                    decision = Some(false);
                }
            });
        });
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        decision = Some(false);
    }
    match decision {
        Some(true) => {
            app.size_dialog = None;
            app.export.size = size;
            app.run_image_export(what);
        }
        Some(false) => app.size_dialog = None,
        None => app.size_dialog = Some((what, size, keep)),
    }
}
