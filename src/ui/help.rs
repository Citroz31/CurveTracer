//! "Help" page.

use egui::{RichText, Ui};

use super::section;

pub fn show(ui: &mut Ui) {
    egui::ScrollArea::vertical().id_salt("help").show(ui, |ui| {
        ui.heading("CurveTracer – quick guide");
        ui.add_space(6.0);
        section(ui, "1. Load the data (File Select)", |ui| {
            ui.label("• Open or drag & drop CSV / TXT / Excel files. Several sources can be loaded at the same time.");
            ui.label("• The delimiter and the header line are detected automatically; the metadata lines written above the column names are kept as “File information”.");
            ui.label("• ⚙ Options lets you force the delimiter, the header line, the decimal comma or the Excel sheet.");
        });
        section(ui, "2. Add plots one by one", |ui| {
            ui.label("• Click “Add Plot” on a data source, then in Data → Select Data:");
            ui.label("    X-Data: the X column (limit 1) with its modifier (÷1e9: Hz → GHz…).");
            ui.label("    Y-Data: one or several columns.");
            ui.label("    Filter: keep only some values of a column (temperature, comment, state…) or a numeric range.");
            ui.label("    Series: one curve per value of a column (SN, temperature, part number…). Several series columns combine.");
            ui.label("• Drag a column onto a box, or use the X / Y / F / S buttons.");
            ui.label("• Process Data: mean, median, min, max, sum, RMS, standard deviation… of the curves, per X value.");
            ui.label("• View Data: table of the rows used by the plot.");
        });
        section(ui, "3. Explore", |ui| {
            ui.label("• Mouse wheel / pinch: zoom · drag: pan · right-drag: box zoom · drag an axis: zoom this axis.");
            ui.label("• Double-click: reset the view · hover a curve: read its values.");
            ui.label("• Click on a curve: pin a marker showing x / y (click it again to remove it).");
            ui.label("• The legend can be dragged with the mouse. Drop it on an edge of the graph to place it outside: the plot shrinks to make room. Click an entry to hide / show a curve.");
        });
        section(ui, "4. Layout & traces", |ui| {
            ui.label("• Layout: title, axis labels and their fonts / sizes / colors, tick fonts, ranges, tick steps, decimals, grid, legend.");
            ui.label("• Trace Options: palette, widths, line styles, markers, custom legend labels, visibility of each curve.");
            ui.label("• Fonts: Arial, Calibri, Times New Roman… are used when installed; File → Load a font file adds any .ttf/.otf.");
        });
        section(ui, "5. User functions", |ui| {
            ui.label("• Functions page: new columns computed from formulas, e.g. Frequency (GHz) = [Freq] / 1e9, Gain = [S21db] - [Thru Loss].");
        });
        section(ui, "6. Export", |ui| {
            ui.label("• Export page: choose the plots and the slide titles, then Download PowerPoint (one 16:9 slide per plot), PNG images or the plot data as CSV.");
            ui.label("• Save the workspace (QuickSave / Ctrl+S) to find all the plots again later.");
        });
        ui.add_space(8.0);
        ui.label(RichText::new(format!("CurveTracer {}", env!("CARGO_PKG_VERSION"))).weak());
    });
}
