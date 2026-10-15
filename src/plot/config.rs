//! Serializable description of a plot (what is saved in a workspace).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::data::hash::hash_one;

pub type Rgba = [u8; 4];

/// Unit modifier applied to a column (e.g. Hz → GHz).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Modifier {
    #[default]
    None,
    Giga,
    Mega,
    Kilo,
    Milli,
    Micro,
    Nano,
    Custom,
}

impl Modifier {
    pub const ALL: [Modifier; 8] = [
        Modifier::None,
        Modifier::Giga,
        Modifier::Mega,
        Modifier::Kilo,
        Modifier::Milli,
        Modifier::Micro,
        Modifier::Nano,
        Modifier::Custom,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            Modifier::None => "None",
            Modifier::Giga => "÷ 1e9  (Hz → GHz)",
            Modifier::Mega => "÷ 1e6  (Hz → MHz)",
            Modifier::Kilo => "÷ 1e3  (k)",
            Modifier::Milli => "× 1e3  (A → mA)",
            Modifier::Micro => "× 1e6  (A → µA)",
            Modifier::Nano => "× 1e9  (s → ns)",
            Modifier::Custom => "Custom factor",
        }
    }

    pub fn short(&self) -> &'static str {
        match self {
            Modifier::None => "",
            Modifier::Giga => "÷1e9",
            Modifier::Mega => "÷1e6",
            Modifier::Kilo => "÷1e3",
            Modifier::Milli => "×1e3",
            Modifier::Micro => "×1e6",
            Modifier::Nano => "×1e9",
            Modifier::Custom => "×k",
        }
    }

    fn prefix(&self) -> Option<&'static str> {
        match self {
            Modifier::Giga => Some("G"),
            Modifier::Mega => Some("M"),
            Modifier::Kilo => Some("k"),
            Modifier::Milli => Some("m"),
            Modifier::Micro => Some("µ"),
            Modifier::Nano => Some("n"),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Scale {
    pub modifier: Modifier,
    pub custom: f64,
}

impl Default for Scale {
    fn default() -> Self {
        Scale {
            modifier: Modifier::None,
            custom: 1.0,
        }
    }
}

impl Scale {
    /// Applies the modifier to a value (divisions keep e.g. 35e9 Hz → 35 GHz exact).
    #[inline]
    pub fn apply(&self, v: f64) -> f64 {
        match self.modifier {
            Modifier::None => v,
            Modifier::Giga => v / 1e9,
            Modifier::Mega => v / 1e6,
            Modifier::Kilo => v / 1e3,
            Modifier::Milli => v * 1e3,
            Modifier::Micro => v * 1e6,
            Modifier::Nano => v * 1e9,
            Modifier::Custom => v * self.custom,
        }
    }

    /// Axis label for a column with this modifier:
    /// `Freq` + GHz → `Freq (GHz)`, `VDD Supply Current [A]` + milli → `… [mA]`.
    pub fn apply_to_label(&self, name: &str) -> String {
        if let Some(p) = self.modifier.prefix() {
            for (open, close) in [('[', ']'), ('(', ')')] {
                if name.ends_with(close)
                    && let Some(i) = name.rfind(open)
                {
                    let unit = &name[i + 1..name.len() - 1];
                    if !unit.is_empty() && unit.len() <= 4 {
                        return format!("{}{open}{p}{unit}{close}", &name[..i]);
                    }
                }
            }
            let lower = name.to_ascii_lowercase();
            if lower.contains("freq") {
                return format!("{name} ({p}Hz)");
            }
            return format!("{name} ({})", self.modifier.short());
        }
        if self.modifier == Modifier::Custom && self.custom != 1.0 {
            return format!("{name} (×{})", self.custom);
        }
        name.to_owned()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FilterMode {
    /// Keep the rows whose value is in the selected list.
    #[default]
    Values,
    /// Keep the rows whose numeric value is in `[min, max]`.
    Range,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FilterConfig {
    pub column: String,
    pub enabled: bool,
    pub mode: FilterMode,
    /// Selected (kept) values for `FilterMode::Values`.
    pub values: Vec<String>,
    pub min: Option<f64>,
    pub max: Option<f64>,
}

impl Default for FilterConfig {
    fn default() -> Self {
        FilterConfig {
            column: String::new(),
            enabled: true,
            mode: FilterMode::Values,
            values: Vec::new(),
            min: None,
            max: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AggFunc {
    #[default]
    Mean,
    Median,
    Min,
    Max,
    Sum,
    Rms,
    Std,
    Range,
    Count,
}

impl AggFunc {
    pub const ALL: [AggFunc; 9] = [
        AggFunc::Mean,
        AggFunc::Median,
        AggFunc::Min,
        AggFunc::Max,
        AggFunc::Sum,
        AggFunc::Rms,
        AggFunc::Std,
        AggFunc::Range,
        AggFunc::Count,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            AggFunc::Mean => "Mean (average)",
            AggFunc::Median => "Median",
            AggFunc::Min => "Min",
            AggFunc::Max => "Max",
            AggFunc::Sum => "Sum",
            AggFunc::Rms => "RMS",
            AggFunc::Std => "Standard deviation",
            AggFunc::Range => "Range (max − min)",
            AggFunc::Count => "Count",
        }
    }

    pub fn short(&self) -> &'static str {
        match self {
            AggFunc::Mean => "mean",
            AggFunc::Median => "median",
            AggFunc::Min => "min",
            AggFunc::Max => "max",
            AggFunc::Sum => "sum",
            AggFunc::Rms => "rms",
            AggFunc::Std => "std",
            AggFunc::Range => "range",
            AggFunc::Count => "count",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProcessConfig {
    pub enabled: bool,
    pub func: AggFunc,
    /// Series columns collapsed by the aggregation. Empty = only merge the
    /// points sharing the same X inside each trace.
    pub collapse: Vec<String>,
    /// Keep the original traces next to the aggregated ones.
    pub keep_original: bool,
    /// Smoothing of the curves (applied after the aggregation, if any).
    pub smooth: super::smooth::SmoothConfig,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FontSpec {
    /// "Proportional", "Monospace" or the name of a loaded font.
    pub family: String,
    pub size: f32,
    pub color: Rgba,
}

impl FontSpec {
    pub fn new(size: f32) -> Self {
        FontSpec {
            family: "Proportional".to_owned(),
            size,
            color: [30, 30, 30, 255],
        }
    }
}

impl Default for FontSpec {
    fn default() -> Self {
        FontSpec::new(14.0)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AxisLayout {
    /// Axis title (empty = automatic, from the column name).
    pub label: String,
    pub auto_range: bool,
    pub min: f64,
    pub max: f64,
    pub auto_ticks: bool,
    pub step: f64,
    pub auto_decimals: bool,
    pub decimals: usize,
    pub grid: bool,
}

impl Default for AxisLayout {
    fn default() -> Self {
        AxisLayout {
            label: String::new(),
            auto_range: true,
            min: 0.0,
            max: 1.0,
            auto_ticks: true,
            step: 1.0,
            auto_decimals: true,
            decimals: 2,
            grid: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum LegendPos {
    #[default]
    InsideTopRight,
    InsideTopLeft,
    InsideBottomRight,
    InsideBottomLeft,
    OutsideRight,
    OutsideLeft,
    OutsideTop,
    OutsideBottom,
    /// Free position set with the mouse, normalized in the figure area.
    Free {
        x: f32,
        y: f32,
    },
}

impl LegendPos {
    pub const PRESETS: [LegendPos; 8] = [
        LegendPos::InsideTopRight,
        LegendPos::InsideTopLeft,
        LegendPos::InsideBottomRight,
        LegendPos::InsideBottomLeft,
        LegendPos::OutsideRight,
        LegendPos::OutsideLeft,
        LegendPos::OutsideTop,
        LegendPos::OutsideBottom,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            LegendPos::InsideTopRight => "Inside, top right",
            LegendPos::InsideTopLeft => "Inside, top left",
            LegendPos::InsideBottomRight => "Inside, bottom right",
            LegendPos::InsideBottomLeft => "Inside, bottom left",
            LegendPos::OutsideRight => "Outside, right",
            LegendPos::OutsideLeft => "Outside, left",
            LegendPos::OutsideTop => "Outside, top",
            LegendPos::OutsideBottom => "Outside, bottom",
            LegendPos::Free { .. } => "Free (dragged)",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LegendConfig {
    pub show: bool,
    pub position: LegendPos,
    pub font: FontSpec,
    pub title: String,
    pub background: bool,
    pub border: bool,
}

impl Default for LegendConfig {
    fn default() -> Self {
        LegendConfig {
            show: true,
            position: LegendPos::InsideTopRight,
            font: FontSpec::new(13.0),
            title: String::new(),
            background: true,
            border: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LayoutConfig {
    pub show_title: bool,
    pub title: String,
    pub title_font: FontSpec,
    pub label_font: FontSpec,
    pub tick_font: FontSpec,
    pub x: AxisLayout,
    pub y: AxisLayout,
    pub legend: LegendConfig,
    pub background: Rgba,
    pub grid_color: Rgba,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        LayoutConfig {
            show_title: false,
            title: String::new(),
            title_font: FontSpec::new(18.0),
            label_font: FontSpec::new(15.0),
            tick_font: FontSpec::new(13.0),
            x: AxisLayout::default(),
            y: AxisLayout::default(),
            legend: LegendConfig::default(),
            background: [255, 255, 255, 255],
            grid_color: [120, 120, 120, 255],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlotOptions {
    /// Sort the points of every trace by X.
    pub sort_x: bool,
    /// When not sorting: start a new line segment when X goes backwards.
    pub break_on_reversal: bool,
    pub hover_readout: bool,
    pub crosshair: bool,
    /// Min/max decimation of huge traces (lossless visually).
    pub decimate: bool,
}

impl Default for PlotOptions {
    fn default() -> Self {
        PlotOptions {
            sort_x: false,
            break_on_reversal: true,
            hover_readout: true,
            crosshair: true,
            decimate: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LineKind {
    #[default]
    Solid,
    Dashed,
    Dotted,
    /// User dash pattern (dash-dot by default).
    Custom,
    None,
}

impl LineKind {
    pub const ALL: [LineKind; 5] = [
        LineKind::Solid,
        LineKind::Dashed,
        LineKind::Dotted,
        LineKind::Custom,
        LineKind::None,
    ];
    pub fn label(&self) -> &'static str {
        match self {
            LineKind::Solid => "Solid",
            LineKind::Dashed => "Dashed",
            LineKind::Dotted => "Dotted",
            LineKind::Custom => "Custom (dash-dot)",
            LineKind::None => "No line",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MarkerKind {
    #[default]
    None,
    Circle,
    Square,
    Diamond,
    Up,
    Down,
    Cross,
    Plus,
}

impl MarkerKind {
    pub const ALL: [MarkerKind; 8] = [
        MarkerKind::None,
        MarkerKind::Circle,
        MarkerKind::Square,
        MarkerKind::Diamond,
        MarkerKind::Up,
        MarkerKind::Down,
        MarkerKind::Cross,
        MarkerKind::Plus,
    ];
    pub fn label(&self) -> &'static str {
        match self {
            MarkerKind::None => "None",
            MarkerKind::Circle => "● Circle",
            MarkerKind::Square => "■ Square",
            MarkerKind::Diamond => "◆ Diamond",
            MarkerKind::Up => "▲ Triangle up",
            MarkerKind::Down => "▼ Triangle down",
            MarkerKind::Cross => "✖ Cross",
            MarkerKind::Plus => "+ Plus",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Palette {
    #[default]
    Classic,
    Bold,
    Dark,
    Pastel,
    Viridis,
    Plasma,
    Gray,
}

impl Palette {
    pub const ALL: [Palette; 7] = [
        Palette::Classic,
        Palette::Bold,
        Palette::Dark,
        Palette::Pastel,
        Palette::Viridis,
        Palette::Plasma,
        Palette::Gray,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            Palette::Classic => "Classic (tab10)",
            Palette::Bold => "Bold",
            Palette::Dark => "Dark",
            Palette::Pastel => "Pastel",
            Palette::Viridis => "Viridis (gradient)",
            Palette::Plasma => "Plasma (gradient)",
            Palette::Gray => "Grayscale (gradient)",
        }
    }

    /// Color of trace `i` among `n` traces.
    pub fn color(&self, i: usize, n: usize) -> Rgba {
        const CLASSIC: [Rgba; 10] = [
            [31, 119, 180, 255],
            [255, 127, 14, 255],
            [44, 160, 44, 255],
            [214, 39, 40, 255],
            [148, 103, 189, 255],
            [140, 86, 75, 255],
            [227, 119, 194, 255],
            [127, 127, 127, 255],
            [188, 189, 34, 255],
            [23, 190, 207, 255],
        ];
        const BOLD: [Rgba; 9] = [
            [228, 26, 28, 255],
            [55, 126, 184, 255],
            [77, 175, 74, 255],
            [152, 78, 163, 255],
            [255, 127, 0, 255],
            [166, 86, 40, 255],
            [247, 129, 191, 255],
            [153, 153, 153, 255],
            [0, 0, 0, 255],
        ];
        const DARK: [Rgba; 8] = [
            [27, 158, 119, 255],
            [217, 95, 2, 255],
            [117, 112, 179, 255],
            [231, 41, 138, 255],
            [102, 166, 30, 255],
            [230, 171, 2, 255],
            [166, 118, 29, 255],
            [102, 102, 102, 255],
        ];
        const PASTEL: [Rgba; 8] = [
            [102, 194, 165, 255],
            [252, 141, 98, 255],
            [141, 160, 203, 255],
            [231, 138, 195, 255],
            [166, 216, 84, 255],
            [255, 217, 47, 255],
            [229, 196, 148, 255],
            [179, 179, 179, 255],
        ];
        const VIRIDIS: [Rgba; 6] = [
            [68, 1, 84, 255],
            [65, 68, 135, 255],
            [42, 120, 142, 255],
            [34, 168, 132, 255],
            [122, 209, 81, 255],
            [253, 231, 37, 255],
        ];
        const PLASMA: [Rgba; 6] = [
            [13, 8, 135, 255],
            [106, 0, 168, 255],
            [177, 42, 144, 255],
            [225, 100, 98, 255],
            [252, 166, 54, 255],
            [240, 249, 33, 255],
        ];
        let t = if n <= 1 { 0.0 } else { i as f32 / (n - 1) as f32 };
        match self {
            Palette::Classic => CLASSIC[i % CLASSIC.len()],
            Palette::Bold => BOLD[i % BOLD.len()],
            Palette::Dark => DARK[i % DARK.len()],
            Palette::Pastel => PASTEL[i % PASTEL.len()],
            Palette::Viridis => gradient(&VIRIDIS, t * 0.92),
            Palette::Plasma => gradient(&PLASMA, t * 0.9),
            Palette::Gray => {
                let v = (t * 170.0) as u8;
                [v, v, v, 255]
            }
        }
    }
}

fn gradient(stops: &[Rgba], t: f32) -> Rgba {
    let t = t.clamp(0.0, 1.0) * (stops.len() - 1) as f32;
    let i = (t.floor() as usize).min(stops.len() - 2);
    let f = t - i as f32;
    let a = stops[i];
    let b = stops[i + 1];
    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * f).round() as u8;
    [mix(a[0], b[0]), mix(a[1], b[1]), mix(a[2], b[2]), 255]
}

/// Long dash, gap, short dash, gap (× line width).
pub const DEFAULT_CUSTOM_DASH: [f32; 4] = [6.0, 2.5, 1.5, 2.5];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TraceDefaults {
    pub width: f32,
    pub line: LineKind,
    pub marker: MarkerKind,
    pub marker_size: f32,
    pub palette: Palette,
    /// Temperature as the series: -40 °C blue custom dashes, 25 °C green
    /// solid, 85 °C red dashed.
    pub temperature_colors: bool,
    /// Dash pattern of the "Custom" line style: dash, gap, dash, gap…
    /// (in multiples of the line width).
    pub custom_dash: Vec<f32>,
}

impl Default for TraceDefaults {
    fn default() -> Self {
        TraceDefaults {
            width: 2.0,
            line: LineKind::Solid,
            marker: MarkerKind::None,
            marker_size: 3.5,
            palette: Palette::Classic,
            temperature_colors: true,
            custom_dash: DEFAULT_CUSTOM_DASH.to_vec(),
        }
    }
}

/// Per-trace customization, keyed by the trace key.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TraceOverride {
    pub color: Option<Rgba>,
    pub width: Option<f32>,
    pub line: Option<LineKind>,
    pub marker: Option<MarkerKind>,
    pub visible: bool,
    /// Custom legend label (empty = automatic).
    pub label: String,
}

impl Default for TraceOverride {
    fn default() -> Self {
        TraceOverride {
            color: None,
            width: None,
            line: None,
            marker: None,
            visible: true,
            label: String::new(),
        }
    }
}

impl TraceOverride {
    pub fn is_default(&self) -> bool {
        *self == TraceOverride::default()
    }
}

/// A data cursor placed by clicking on a curve.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DataMarker {
    pub trace: String,
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlotConfig {
    pub id: u64,
    pub name: String,
    /// Key (path) of the data source.
    pub source: String,
    pub x: String,
    pub x_scale: Scale,
    pub ys: Vec<String>,
    pub y_scale: Scale,
    pub filters: Vec<FilterConfig>,
    pub series: Vec<String>,
    pub process: ProcessConfig,
    pub layout: LayoutConfig,
    pub options: PlotOptions,
    pub traces: TraceDefaults,
    pub overrides: BTreeMap<String, TraceOverride>,
    /// Names shown in the legend instead of the column names:
    /// ("Set_TempC", "Temperature").
    pub legend_names: Vec<(String, String)>,
    pub markers: Vec<DataMarker>,
    /// Title of the slide in the PowerPoint export (empty = plot title/name).
    pub slide_title: String,
    pub export: bool,
}

impl Default for PlotConfig {
    fn default() -> Self {
        PlotConfig {
            id: 0,
            name: "Plot".to_owned(),
            source: String::new(),
            x: String::new(),
            x_scale: Scale::default(),
            ys: Vec::new(),
            y_scale: Scale::default(),
            filters: Vec::new(),
            series: Vec::new(),
            process: ProcessConfig::default(),
            layout: LayoutConfig::default(),
            options: PlotOptions::default(),
            traces: TraceDefaults::default(),
            overrides: BTreeMap::new(),
            legend_names: Vec::new(),
            markers: Vec::new(),
            slide_title: String::new(),
            export: true,
        }
    }
}

impl PlotConfig {
    /// Hash of everything that changes the computed traces.
    pub fn data_key(&self) -> u64 {
        use std::hash::Hasher;
        // called every frame for every plot: hash the (possibly thousands of)
        // filter values directly instead of formatting the whole configuration
        let mut h = crate::data::hash::FxHasher::default();
        let small = format!(
            "{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{}|{}",
            self.source,
            self.x,
            self.x_scale,
            self.ys,
            self.y_scale,
            self.series,
            self.process,
            self.options.sort_x,
            self.options.break_on_reversal
        );
        h.write(small.as_bytes());
        for f in &self.filters {
            h.write(f.column.as_bytes());
            h.write_u8(f.enabled as u8);
            h.write_u8(f.mode as u8);
            h.write_u64(f.min.map_or(u64::MAX, f64::to_bits));
            h.write_u64(f.max.map_or(u64::MAX - 1, f64::to_bits));
            h.write_usize(f.values.len());
            for v in &f.values {
                h.write(v.as_bytes());
                h.write_u8(0xFF);
            }
        }
        h.finish()
    }

    /// Hash of the axis ranges (a change resets the view).
    pub fn view_key(&self) -> u64 {
        let x = &self.layout.x;
        let y = &self.layout.y;
        hash_one(&format!(
            "{}{}{}{}{}{}",
            x.auto_range, x.min, x.max, y.auto_range, y.min, y.max
        ))
    }

    /// Title shown above the plot / on the slide.
    pub fn display_title(&self) -> String {
        if !self.slide_title.trim().is_empty() {
            self.slide_title.clone()
        } else if self.layout.show_title && !self.layout.title.trim().is_empty() {
            self.layout.title.clone()
        } else {
            self.name.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_round_trip() {
        let mut cfg = PlotConfig {
            id: 7,
            x: "Freq".into(),
            ys: vec!["S21db".into()],
            series: vec!["SN".into()],
            filters: vec![FilterConfig {
                column: "Set_TempC".into(),
                values: vec!["25.0".into()],
                ..Default::default()
            }],
            ..Default::default()
        };
        cfg.layout.legend.position = LegendPos::Free { x: 0.5, y: 0.25 };
        cfg.x_scale.modifier = Modifier::Giga;
        cfg.overrides.insert(
            "S21db|SN=1".into(),
            TraceOverride {
                color: Some([1, 2, 3, 255]),
                ..Default::default()
            },
        );
        cfg.markers.push(DataMarker {
            trace: "S21db|SN=1".into(),
            x: 1.0,
            y: 2.0,
        });
        let text = serde_json::to_string(&cfg).unwrap();
        let back: PlotConfig = serde_json::from_str(&text).unwrap();
        assert_eq!(back, cfg);
        // missing fields take their default values
        let partial: PlotConfig = serde_json::from_str(r#"{"x":"Freq","ys":["S21db"]}"#).unwrap();
        assert_eq!(partial.layout, LayoutConfig::default());
        assert!(partial.export);
    }

    #[test]
    fn labels() {
        let giga = Scale {
            modifier: Modifier::Giga,
            custom: 1.0,
        };
        assert_eq!(giga.apply_to_label("Freq"), "Freq (GHz)");
        let milli = Scale {
            modifier: Modifier::Milli,
            custom: 1.0,
        };
        assert_eq!(
            milli.apply_to_label("VDD Supply Current [A]"),
            "VDD Supply Current [mA]"
        );
        assert_eq!(Scale::default().apply_to_label("S21db"), "S21db");
    }
}
