//! "Data" page: browse the content of a data source.

use egui::{RichText, Ui};

use super::{table::data_table, thousands};
use crate::app::App;

pub fn show(app: &mut App, ui: &mut Ui) {
    if app.sources.is_empty() {
        ui.heading("Data");
        ui.label("No data source loaded. Go to File Select to open a file.");
        return;
    }
    if app.data_page.source >= app.sources.len() {
        app.data_page.source = 0;
    }
    ui.horizontal(|ui| {
        ui.heading("Data");
        ui.add_space(10.0);
        egui::ComboBox::from_id_salt("data_source")
            .width(320.0)
            .selected_text(app.sources[app.data_page.source].name())
            .show_ui(ui, |ui| {
                for (i, s) in app.sources.iter().enumerate() {
                    ui.selectable_value(&mut app.data_page.source, i, s.name());
                }
            });
        ui.label("Columns:");
        ui.add(
            egui::TextEdit::singleline(&mut app.data_page.search)
                .hint_text("filter columns…")
                .desired_width(180.0),
        );
    });
    let Some(ds) = app.sources[app.data_page.source].ds.clone() else {
        ui.spinner();
        return;
    };
    let search = app.data_page.search.to_lowercase();
    let cols: Vec<usize> = (0..ds.columns.len())
        .filter(|&c| search.is_empty() || ds.columns[c].name.to_lowercase().contains(&search))
        .collect();
    ui.label(
        RichText::new(format!(
            "{} rows × {} columns ({} shown) · hover a column header for its statistics · blue = user function",
            thousands(ds.n_rows),
            ds.columns.len(),
            cols.len()
        ))
        .weak(),
    );
    ui.add_space(4.0);
    data_table(
        ui,
        egui::Id::new(("data_page_table", app.data_page.source)),
        &ds,
        None,
        &cols,
    );
}
