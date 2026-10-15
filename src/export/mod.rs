pub mod data;
pub mod pptx;
pub mod zip;

use crate::plot::compute::PlotData;
use crate::plot::config::{FilterMode, PlotConfig};

/// Human readable description of a plot (shown next to the plot on slides).
pub fn plot_settings(cfg: &PlotConfig, data: &PlotData, source_name: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    out.push(("Data source".to_owned(), source_name.to_owned()));
    let mut x = cfg.x.clone();
    if !cfg.x_scale.modifier.short().is_empty() {
        x = format!("{x}  ({})", cfg.x_scale.modifier.short());
    }
    out.push(("X".to_owned(), x));
    let mut y = cfg.ys.join(", ");
    if !cfg.y_scale.modifier.short().is_empty() {
        y = format!("{y}  ({})", cfg.y_scale.modifier.short());
    }
    out.push(("Y".to_owned(), y));
    if !cfg.series.is_empty() {
        out.push(("Series".to_owned(), cfg.series.join(", ")));
    }
    let filters: Vec<String> = cfg
        .filters
        .iter()
        .filter(|f| f.enabled)
        .map(|f| match f.mode {
            FilterMode::Values => {
                let shown: Vec<&str> = f.values.iter().take(8).map(|s| s.as_str()).collect();
                let more = if f.values.len() > 8 {
                    format!(" … (+{})", f.values.len() - 8)
                } else {
                    String::new()
                };
                format!("{} = {}{more}", f.column, shown.join(", "))
            }
            FilterMode::Range => format!(
                "{} ∈ [{}, {}]",
                f.column,
                f.min.map_or("−∞".to_owned(), |v| v.to_string()),
                f.max.map_or("+∞".to_owned(), |v| v.to_string())
            ),
        })
        .collect();
    if !filters.is_empty() {
        out.push(("Filters".to_owned(), filters.join("\n")));
    }
    if cfg.process.enabled {
        let over = if cfg.process.collapse.is_empty() {
            "points with the same X".to_owned()
        } else {
            cfg.process.collapse.join(", ")
        };
        out.push((
            "Processing".to_owned(),
            format!("{} over {over}", cfg.process.func.label()),
        ));
    }
    out.push((
        "Content".to_owned(),
        format!(
            "{} curve(s), {} points, {} rows",
            data.traces.len(),
            data.total_points,
            data.rows.len()
        ),
    ));
    out
}
