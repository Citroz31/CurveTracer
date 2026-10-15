//! Virtualized data table: only the visible rows are laid out, so tables of
//! millions of rows scroll smoothly.

use egui::{Align2, Color32, FontId, Id, Rect, RichText, Sense, Stroke, Ui, Vec2};

use crate::data::dataset::{Column, Dataset};

const ROW_H: f32 = 20.0;

fn col_width(c: &Column, rows: &[usize]) -> f32 {
    let mut chars = c.name.chars().count() + 4;
    for &r in rows.iter().take(64) {
        chars = chars.max(c.text(r).chars().count());
    }
    (chars as f32 * 7.2 + 14.0).clamp(56.0, 260.0)
}

/// Shows `rows` (all rows when `None`) of the given columns.
pub fn data_table(ui: &mut Ui, id: Id, ds: &Dataset, rows: Option<&[u32]>, cols: &[usize]) {
    let n = rows.map_or(ds.n_rows, |r| r.len());
    let row_index = |i: usize| -> usize { rows.map_or(i, |r| r[i] as usize) };
    let sample: Vec<usize> = (0..n.min(64)).map(row_index).filter(|&r| r < ds.n_rows).collect();
    let widths: Vec<f32> = cols.iter().map(|&c| col_width(&ds.columns[c], &sample)).collect();
    let idx_w = ((n.max(1) as f32).log10().floor() + 1.0) * 8.0 + 16.0;
    let total_w = idx_w + widths.iter().sum::<f32>();
    let font = FontId::monospace(12.5);
    let header_font = FontId::proportional(13.0);
    let text_color = ui.visuals().text_color();
    let weak = ui.visuals().weak_text_color();
    let stripe = ui.visuals().faint_bg_color;
    let header_bg = ui.visuals().widgets.inactive.bg_fill;
    let grid = Stroke::new(
        1.0,
        ui.visuals().widgets.noninteractive.bg_stroke.color.gamma_multiply(0.6),
    );

    egui::ScrollArea::horizontal()
        .id_salt(id.with("h"))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.set_min_width(total_w);
            // header
            let (hrect, _) = ui.allocate_exact_size(Vec2::new(total_w, ROW_H + 6.0), Sense::hover());
            let p = ui.painter_at(hrect);
            p.rect_filled(hrect, 0.0, header_bg);
            let mut x = hrect.left() + idx_w;
            p.text(
                hrect.left_center() + egui::vec2(4.0, 0.0),
                Align2::LEFT_CENTER,
                "#",
                header_font.clone(),
                weak,
            );
            for (k, &c) in cols.iter().enumerate() {
                let col = &ds.columns[c];
                let r = Rect::from_min_size(egui::pos2(x, hrect.top()), Vec2::new(widths[k], hrect.height()));
                let pc = ui.painter_at(r.shrink2(egui::vec2(3.0, 0.0)));
                let color = if col.formula.is_some() {
                    Color32::from_rgb(40, 110, 200)
                } else {
                    text_color
                };
                pc.text(
                    r.left_center() + egui::vec2(4.0, 0.0),
                    Align2::LEFT_CENTER,
                    &col.name,
                    header_font.clone(),
                    color,
                );
                p.line_segment([r.right_top(), r.right_bottom()], grid);
                let resp = ui.interact(r, id.with(("hdr", c)), Sense::hover());
                resp.on_hover_ui(|ui| {
                    ui.label(RichText::new(&col.name).strong());
                    ui.label(format!("{:?} column · {} unique value(s)", col.kind, col.n_unique));
                    if col.min.is_finite() {
                        ui.label(format!("min {} · max {}", col.min, col.max));
                    }
                    if let Some(f) = &col.formula {
                        ui.label(format!("User function: {f}"));
                    }
                });
                x += widths[k];
            }
            // body
            egui::ScrollArea::vertical()
                .id_salt(id.with("v"))
                .auto_shrink([false, false])
                .show_rows(ui, ROW_H, n, |ui, range| {
                    for i in range {
                        let r_idx = row_index(i);
                        let (rect, _) = ui.allocate_exact_size(Vec2::new(total_w, ROW_H), Sense::hover());
                        // (rows of an older version of the data: skipped)
                        if r_idx >= ds.n_rows || !ui.is_rect_visible(rect) {
                            continue;
                        }
                        let p = ui.painter_at(rect);
                        if i % 2 == 1 {
                            p.rect_filled(rect, 0.0, stripe);
                        }
                        p.text(
                            rect.left_center() + egui::vec2(4.0, 0.0),
                            Align2::LEFT_CENTER,
                            (r_idx + 1).to_string(),
                            font.clone(),
                            weak,
                        );
                        let clip = ui.clip_rect();
                        let mut x = rect.left() + idx_w;
                        for (k, &c) in cols.iter().enumerate() {
                            let cell = Rect::from_min_size(egui::pos2(x, rect.top()), Vec2::new(widths[k], ROW_H));
                            x += widths[k];
                            if cell.right() < clip.left() || cell.left() > clip.right() {
                                continue;
                            }
                            let col = &ds.columns[c];
                            let t = col.text(r_idx);
                            let pc = ui.painter_at(cell.shrink2(egui::vec2(3.0, 0.0)));
                            let numeric = !col.value(r_idx).is_nan();
                            if numeric {
                                pc.text(
                                    cell.right_center() - egui::vec2(5.0, 0.0),
                                    Align2::RIGHT_CENTER,
                                    t.as_ref(),
                                    font.clone(),
                                    text_color,
                                );
                            } else {
                                pc.text(
                                    cell.left_center() + egui::vec2(4.0, 0.0),
                                    Align2::LEFT_CENTER,
                                    t.as_ref(),
                                    font.clone(),
                                    text_color,
                                );
                            }
                            p.line_segment([cell.right_top(), cell.right_bottom()], grid);
                        }
                    }
                });
        });
}
