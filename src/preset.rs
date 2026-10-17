//! Style presets: templates of the look of the plots (fonts, colors, line
//! widths, markers, tick labels, legend, grid) and of the export settings
//! (image size, slide format). Unlike a workspace, a preset holds no data,
//! no file, no plot and no text: it can be applied to any plot of any
//! workspace. Presets are `.ctp` files in the CurveTracer settings folder.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::app::{ExportSettings, ImageSize};
use crate::export::pptx::SlideFormat;
use crate::plot::config::{FontSpec, LegendPos, PlotConfig, PlotOptions, Rgba, TraceDefaults};
use crate::plot::render::ImageFormat;

pub const PRESET_EXT: &str = "ctp";

/// Look of an axis (no title, range or tick step: they depend on the data).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AxisStyle {
    pub grid: bool,
    pub auto_decimals: bool,
    pub decimals: usize,
}

impl Default for AxisStyle {
    fn default() -> Self {
        AxisStyle {
            grid: true,
            auto_decimals: true,
            decimals: 2,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LegendStyle {
    pub show: bool,
    pub position: LegendPos,
    pub font: FontSpec,
    pub background: bool,
    pub border: bool,
}

impl Default for LegendStyle {
    fn default() -> Self {
        let l = crate::plot::config::LegendConfig::default();
        LegendStyle {
            show: l.show,
            position: l.position,
            font: l.font,
            background: l.background,
            border: l.border,
        }
    }
}

/// Look of the plots.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlotStyle {
    pub show_title: bool,
    pub title_font: FontSpec,
    pub label_font: FontSpec,
    pub tick_font: FontSpec,
    pub x: AxisStyle,
    pub y: AxisStyle,
    pub legend: LegendStyle,
    pub background: Rgba,
    pub grid_color: Rgba,
    /// Palette, line width and style, markers, temperature styles.
    pub traces: TraceDefaults,
    pub options: PlotOptions,
}

impl Default for PlotStyle {
    fn default() -> Self {
        PlotStyle::of(&PlotConfig::default())
    }
}

impl PlotStyle {
    /// Style of a plot.
    pub fn of(cfg: &PlotConfig) -> PlotStyle {
        let l = &cfg.layout;
        let axis = |a: &crate::plot::config::AxisLayout| AxisStyle {
            grid: a.grid,
            auto_decimals: a.auto_decimals,
            decimals: a.decimals,
        };
        PlotStyle {
            show_title: l.show_title,
            title_font: l.title_font.clone(),
            label_font: l.label_font.clone(),
            tick_font: l.tick_font.clone(),
            x: axis(&l.x),
            y: axis(&l.y),
            legend: LegendStyle {
                show: l.legend.show,
                position: l.legend.position,
                font: l.legend.font.clone(),
                background: l.legend.background,
                border: l.legend.border,
            },
            background: l.background,
            grid_color: l.grid_color,
            traces: cfg.traces.clone(),
            options: cfg.options.clone(),
        }
    }

    /// Gives a plot this style; its data, texts, ranges and per-curve
    /// settings are kept.
    pub fn apply(&self, cfg: &mut PlotConfig) {
        let l = &mut cfg.layout;
        l.show_title = self.show_title;
        l.title_font = self.title_font.clone();
        l.label_font = self.label_font.clone();
        l.tick_font = self.tick_font.clone();
        for (a, s) in [(&mut l.x, &self.x), (&mut l.y, &self.y)] {
            a.grid = s.grid;
            a.auto_decimals = s.auto_decimals;
            a.decimals = s.decimals;
        }
        l.legend.show = self.legend.show;
        l.legend.position = self.legend.position;
        l.legend.font = self.legend.font.clone();
        l.legend.background = self.legend.background;
        l.legend.border = self.legend.border;
        l.background = self.background;
        l.grid_color = self.grid_color;
        cfg.traces = self.traces.clone();
        cfg.options = self.options.clone();
    }
}

/// Export settings of a preset (not the file name nor the folder).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExportStyle {
    pub size: ImageSize,
    pub ask_size: bool,
    pub format: SlideFormat,
    pub image_format: ImageFormat,
    pub pptx_svg: bool,
    pub use_view: bool,
}

impl Default for ExportStyle {
    fn default() -> Self {
        ExportStyle::of(&ExportSettings::default())
    }
}

impl ExportStyle {
    pub fn of(e: &ExportSettings) -> ExportStyle {
        ExportStyle {
            size: e.size,
            ask_size: e.ask_size,
            format: e.format,
            image_format: e.image_format,
            pptx_svg: e.pptx_svg,
            use_view: e.use_view,
        }
    }

    pub fn apply(&self, e: &mut ExportSettings) {
        e.size = self.size;
        e.ask_size = self.ask_size;
        e.format = self.format;
        e.image_format = self.image_format;
        e.pptx_svg = self.pptx_svg;
        e.use_view = self.use_view;
    }
}

/// A preset file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preset {
    /// "CurveTracer preset": tells a preset from a workspace (empty when missing).
    #[serde(default)]
    pub kind: String,
    pub version: u32,
    pub name: String,
    pub description: String,
    /// Look of the plots (`None` = not part of the preset).
    pub style: Option<PlotStyle>,
    /// Export settings (`None` = not part of the preset).
    pub export: Option<ExportStyle>,
}

const KIND: &str = "CurveTracer preset";

impl Default for Preset {
    fn default() -> Self {
        Preset {
            kind: KIND.to_owned(),
            version: 1,
            name: String::new(),
            description: String::new(),
            style: None,
            export: None,
        }
    }
}

impl Preset {
    /// What the preset contains ("plot style + export").
    pub fn contents(&self) -> String {
        match (self.style.is_some(), self.export.is_some()) {
            (true, true) => "plot style + export".to_owned(),
            (true, false) => "plot style".to_owned(),
            (false, true) => "export".to_owned(),
            (false, false) => "empty".to_owned(),
        }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    pub fn from_json(text: &str) -> Result<Preset, String> {
        let p: Preset = serde_json::from_str(text).map_err(|e| format!("Invalid preset file: {e}"))?;
        if p.kind != KIND {
            return Err("This file is not a CurveTracer preset (a workspace? use Workspace → Open)".to_owned());
        }
        Ok(p)
    }

    pub fn load(path: &Path) -> Result<Preset, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
        let mut p = Preset::from_json(&text)?;
        if p.name.trim().is_empty() {
            p.name = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
        }
        Ok(p)
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(d) = path.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        std::fs::write(path, self.to_json()).map_err(|e| format!("Cannot write {}: {e}", path.display()))
    }
}

// =============================================================================
// Library of presets (settings folder)
// =============================================================================

/// Folder of the presets: <settings>/CurveTracer/presets.
pub fn dir() -> Option<PathBuf> {
    crate::recent::config_dir().map(|d| d.join("presets"))
}

/// File of a preset of the library.
pub fn path_for(name: &str) -> Option<PathBuf> {
    let file: String = name
        .trim()
        .chars()
        .map(|c| {
            if "<>:\"/\\|?*".contains(c) || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let file = file.trim().trim_end_matches('.').to_owned();
    if file.is_empty() {
        return None;
    }
    dir().map(|d| d.join(format!("{file}.{PRESET_EXT}")))
}

/// Presets of the library, by name.
#[derive(Default)]
pub struct Library {
    pub presets: Vec<(PathBuf, Preset)>,
    /// Name of the preset given to the new plots and workspaces.
    pub default: Option<String>,
}

fn default_file() -> Option<PathBuf> {
    dir().map(|d| d.join("default.txt"))
}

impl Library {
    pub fn load() -> Library {
        let mut presets = Vec::new();
        if let Some(d) = dir()
            && let Ok(rd) = std::fs::read_dir(&d)
        {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().is_some_and(|x| x.eq_ignore_ascii_case(PRESET_EXT))
                    && let Ok(preset) = Preset::load(&p)
                {
                    presets.push((p, preset));
                }
            }
        }
        presets.sort_by_key(|(_, p)| p.name.to_lowercase());
        let default = default_file()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .map(|s| s.trim().to_owned())
            .filter(|n| presets.iter().any(|(_, p)| p.name == *n));
        Library { presets, default }
    }

    pub fn get(&self, name: &str) -> Option<&Preset> {
        self.presets.iter().find(|(_, p)| p.name == name).map(|(_, p)| p)
    }

    pub fn default_preset(&self) -> Option<&Preset> {
        self.default.as_deref().and_then(|n| self.get(n))
    }

    /// Saves (or replaces) a preset in the library.
    pub fn save(&mut self, preset: Preset) -> Result<PathBuf, String> {
        let path = path_for(&preset.name).ok_or("Give the preset a name")?;
        preset.save(&path)?;
        self.presets.retain(|(_, p)| p.name != preset.name);
        self.presets.push((path.clone(), preset));
        self.presets.sort_by_key(|(_, p)| p.name.to_lowercase());
        Ok(path)
    }

    pub fn delete(&mut self, name: &str) {
        if let Some(i) = self.presets.iter().position(|(_, p)| p.name == name) {
            let (path, _) = self.presets.remove(i);
            let _ = std::fs::remove_file(path);
        }
        if self.default.as_deref() == Some(name) {
            self.set_default(None);
        }
    }

    pub fn set_default(&mut self, name: Option<String>) {
        self.default = name;
        if let Some(f) = default_file() {
            match &self.default {
                Some(n) => {
                    if let Some(d) = f.parent() {
                        let _ = std::fs::create_dir_all(d);
                    }
                    let _ = std::fs::write(f, n);
                }
                None => {
                    let _ = std::fs::remove_file(f);
                }
            }
        }
    }

    /// Copies a preset file into the library.
    pub fn import(&mut self, path: &Path) -> Result<String, String> {
        let p = Preset::load(path)?;
        let name = p.name.clone();
        self.save(p)?;
        Ok(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn style_keeps_data_and_texts() {
        let mut a = PlotConfig::default();
        a.layout.tick_font.size = 9.0;
        a.layout.legend.font.family = "Arial".into();
        a.traces.width = 3.5;
        a.layout.title = "Title of A".into();
        let style = PlotStyle::of(&a);

        let mut b = PlotConfig {
            x: "Freq".into(),
            ys: vec!["S21db".into()],
            ..Default::default()
        };
        b.layout.title = "Title of B".into();
        b.layout.x.label = "Frequency".into();
        b.layout.x.auto_range = false;
        b.layout.x.min = 30.0;
        style.apply(&mut b);
        assert_eq!(b.layout.tick_font.size, 9.0);
        assert_eq!(b.layout.legend.font.family, "Arial");
        assert_eq!(b.traces.width, 3.5);
        // data, texts and ranges are not part of a preset
        assert_eq!(b.x, "Freq");
        assert_eq!(b.layout.title, "Title of B");
        assert_eq!(b.layout.x.label, "Frequency");
        assert_eq!(b.layout.x.min, 30.0);
        assert!(!b.layout.x.auto_range);
    }

    #[test]
    fn file_round_trip_and_kind() {
        let p = Preset {
            name: "Report".into(),
            style: Some(PlotStyle::default()),
            export: Some(ExportStyle::default()),
            ..Default::default()
        };
        let back = Preset::from_json(&p.to_json()).unwrap();
        assert_eq!(back, p);
        assert_eq!(back.contents(), "plot style + export");
        // a workspace is not a preset
        assert!(Preset::from_json(r#"{"app":"CurveTracer","sources":[]}"#).is_err());
        // no data in a preset file
        let json = p.to_json();
        assert!(!json.contains("\"source\"") && !json.contains("\"ys\""));
    }
}
