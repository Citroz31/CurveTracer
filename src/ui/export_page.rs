//! "Export" page: plot thumbnails, slide titles, PowerPoint / PNG / SVG / CSV export.

use std::sync::Arc;

use egui::{Align2, Color32, FontId, RichText, Sense, Ui, Vec2};

use super::{NAVY, section};
use crate::app::App;
use crate::export::pptx::SlideFormat;
use crate::plot::render::ImageFormat;
use crate::plot::view::show_thumbnail;

pub fn show(app: &mut App, ui: &mut Ui) {
    ui.horizontal(|ui| {
        ui.heading("Export");
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
        ui.add(
            egui::DragValue::new(&mut app.export.width)
                .range(400..=6000)
                .suffix(" px"),
        );
        ui.label("×");
        ui.add(
            egui::DragValue::new(&mut app.export.height)
                .range(300..=6000)
                .suffix(" px"),
        );
        ui.checkbox(&mut app.export.use_view, "Use the current zoom of each plot");
    });
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
    let ew = app.export.width.max(1) as f32;
    let eh = app.export.height.max(1) as f32;
    let h = (w * eh / ew).clamp(w * 0.3, w * 1.5);
    let export_scale = crate::plot::render::RenderOptions::new(app.export.width, app.export.height).scale;
    let thumb_scale = w / (ew / export_scale);
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
                app.export_pptx_dialog();
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
                app.export_images_dialog();
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
            RichText::new("Each selected plot becomes one 16:9 slide (title + high resolution image, plus the plot settings in “Plot + Settings” format).")
                .weak()
                .small(),
        );
    });
}
