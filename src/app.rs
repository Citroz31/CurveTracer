//! Application state: data sources, plots, user functions, background jobs.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Instant;

use egui::{Color32, Context, FontData, FontDefinitions, FontFamily};
use serde::{Deserialize, Serialize};

use crate::data::deembed::{self, DeembedConfig, DeembedReport};
use crate::data::loader::{ImportOptions, load_path};
use crate::data::{ColumnKind, Dataset, UserFunction, with_functions};
use crate::export::plot_settings;
use crate::export::pptx::{Slide, SlideFormat, build_pptx};
use crate::fonts::FontBook;
use crate::plot::compute::{PlotData, traces_to_csv};
use crate::plot::config::PlotConfig;
use crate::plot::render::{ImageFormat, RenderOptions, render_image, render_png, render_svg};
use crate::plot::view::PlotState;

pub const WORKSPACE_EXT: &str = "ctw";

/// A plot to export: configuration, data, visible range, source name.
type ExportItem = (PlotConfig, Arc<PlotData>, Option<[f64; 4]>, String);

pub struct DataSource {
    /// Unique key: the absolute path of the file.
    pub key: String,
    pub path: PathBuf,
    pub options: ImportOptions,
    /// Dataset as loaded (no user function).
    pub base: Option<Arc<Dataset>>,
    /// De-embedding of the fixtures (input / output .s2p files).
    pub deembed: DeembedConfig,
    /// Base dataset after de-embedding (when active).
    pub deembedded: Option<Arc<Dataset>>,
    pub deembed_report: Option<Result<DeembedReport, String>>,
    /// Dataset with the user function columns.
    pub ds: Option<Arc<Dataset>>,
    pub func_errors: Vec<Option<String>>,
    pub loading: bool,
    /// De-embedding / user functions being recomputed in the background.
    pub rebuilding: bool,
    /// Number of the latest rebuild (older results are dropped).
    rebuild_ticket: u64,
    pub error: Option<String>,
    pub show_options: bool,
    pub show_deembed: bool,
}

/// Result of a background rebuild: base → de-embedding → user functions.
struct RebuildMsg {
    key: String,
    ticket: u64,
    deembedded: Option<Arc<Dataset>>,
    report: Option<Result<DeembedReport, String>>,
    ds: Dataset,
    func_errors: Vec<Option<String>>,
    /// Show the de-embedding result in the status bar.
    announce: bool,
}

/// De-embedding result: (de-embedded base, report).
type Deembedded = (Option<Arc<Dataset>>, Option<Result<DeembedReport, String>>);
/// (de-embedded base, report, dataset with the functions, function errors).
type Rebuilt = (
    Option<Arc<Dataset>>,
    Option<Result<DeembedReport, String>>,
    Dataset,
    Vec<Option<String>>,
);

/// De-embedding (when `redo`, else the previous result is kept) then user functions.
fn rebuild(
    base: &Arc<Dataset>,
    deembed_cfg: &DeembedConfig,
    redo: bool,
    previous: Deembedded,
    funcs: &[UserFunction],
) -> Rebuilt {
    let (deembedded, report) = if !redo {
        previous
    } else if !deembed_cfg.is_active() {
        (None, None)
    } else {
        match deembed::apply(base, deembed_cfg) {
            Ok((ds, report)) => (Some(Arc::new(ds)), Some(Ok(report))),
            Err(e) => (None, Some(Err(e))),
        }
    };
    let (ds, errors) = with_functions(deembedded.as_ref().unwrap_or(base), funcs);
    (deembedded, report, ds, errors)
}

impl DataSource {
    pub fn name(&self) -> String {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.key.clone())
    }
}

/// UI state of a plot (not saved).
#[derive(Default)]
pub struct PlotUiState {
    pub col_search: String,
    pub sort_by_unique: bool,
    pub open_filters: BTreeSet<usize>,
    pub filter_search: String,
    pub view_all_columns: bool,
    pub trace_search: String,
}

pub struct PlotEntry {
    pub cfg: PlotConfig,
    pub state: PlotState,
    pub ui: PlotUiState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Files,
    Data,
    Functions,
    Plots,
    Export,
    DataExport,
    Help,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SettingsTab {
    #[default]
    Data,
    Layout,
    Options,
    Traces,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DataTab {
    #[default]
    Select,
    Process,
    View,
}

/// Size of the exported images (PNG, SVG, PowerPoint).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ImageSize {
    /// Width and height in inches.
    pub w_in: f32,
    pub h_in: f32,
    pub dpi: u32,
}

impl ImageSize {
    /// Standard format of the reports: 6 × 4.2 in.
    pub const STANDARD: ImageSize = ImageSize {
        w_in: 6.0,
        h_in: 4.2,
        dpi: 300,
    };
    pub const RATIO: f32 = 6.0 / 4.2;

    /// The width / height ratio is 6 : 4.2.
    pub fn standard_ratio(&self) -> bool {
        self.h_in > 0.0 && (self.w_in / self.h_in - Self::RATIO).abs() < 0.002
    }

    pub fn pixels(&self) -> (u32, u32) {
        let o = RenderOptions::physical(self.w_in, self.h_in, self.dpi as f32, 1000.0);
        (o.width, o.height)
    }

    /// Width / height (6 / 4.2 = 1.43 by default).
    pub fn ratio(&self) -> f32 {
        (self.w_in / self.h_in.max(0.1)).clamp(0.25, 4.0)
    }
}

impl Default for ImageSize {
    fn default() -> Self {
        ImageSize::STANDARD
    }
}

/// An export of images waiting for the choice of their format.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ImageExport {
    Pptx,
    Images,
    Plot(usize, ImageFormat),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExportSettings {
    pub format: SlideFormat,
    /// Size of the images (standard 6 × 4.2 in).
    pub size: ImageSize,
    /// Ask the format of the images at each export (it is always asked when
    /// the size is not in the 6 : 4.2 ratio).
    pub ask_size: bool,
    /// Export the current zoom of each plot instead of the full range.
    pub use_view: bool,
    pub file_name: String,
    pub last_dir: Option<PathBuf>,
    /// Format of "Download Images".
    pub image_format: ImageFormat,
    /// Embed the plots of the PowerPoint as vector (SVG) images, with a PNG fallback.
    pub pptx_svg: bool,
}

impl Default for ExportSettings {
    fn default() -> Self {
        ExportSettings {
            format: SlideFormat::PlotAndSettings,
            size: ImageSize::STANDARD,
            ask_size: false,
            use_view: true,
            file_name: "CurveTracer_export".to_owned(),
            last_dir: None,
            image_format: ImageFormat::Png,
            pptx_svg: true,
        }
    }
}

pub struct Status {
    pub text: String,
    pub error: bool,
    pub at: Instant,
}

struct LoadMsg {
    key: String,
    result: Result<Dataset, String>,
}

pub enum TaskMsg {
    Done(String),
    Failed(String),
}

/// An undoable change of the plots.
#[derive(Clone, Debug)]
pub enum UndoAction {
    /// The markers of a plot before the change.
    Markers {
        id: u64,
        markers: Vec<crate::plot::config::DataMarker>,
    },
    /// A plot was deleted from this position.
    Removed { index: usize, cfg: Box<PlotConfig> },
    /// A plot was added (new, duplicated).
    Added { id: u64 },
    /// A plot was moved from → to.
    Moved { from: usize, to: usize },
}

/// Change of workspace waiting for the "save the changes?" answer.
#[derive(Clone, Debug, PartialEq)]
pub enum Switch {
    Open(PathBuf),
    OpenDialog,
    /// Back to the start screen (to create or open another workspace).
    Close,
    /// Close the application window.
    Quit,
}

/// State of the start screen.
pub struct StartState {
    pub new_name: String,
    pub new_dir: PathBuf,
    pub error: Option<String>,
}

#[derive(Default)]
pub struct DataPageState {
    pub source: usize,
    pub search: String,
}

/// What is saved in a workspace file.
#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Workspace {
    pub app: String,
    pub version: u32,
    pub sources: Vec<WorkspaceSource>,
    pub functions: Vec<UserFunction>,
    pub plots: Vec<PlotConfig>,
    pub export: ExportSettings,
    pub fonts: Vec<PathBuf>,
    pub data_export: crate::export::data::DataExportConfig,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
pub struct WorkspaceSource {
    pub path: PathBuf,
    pub options: ImportOptions,
    pub deembed: DeembedConfig,
}

pub struct App {
    pub sources: Vec<DataSource>,
    pub plots: Vec<PlotEntry>,
    pub functions: Vec<UserFunction>,
    pub func_dirty: bool,
    pub page: Page,
    pub active: usize,
    pub settings_tab: SettingsTab,
    pub data_tab: DataTab,
    pub fonts: Arc<FontBook>,
    fonts_installed: usize,
    pub status: Option<Status>,
    load_tx: Sender<LoadMsg>,
    load_rx: Receiver<LoadMsg>,
    rebuild_tx: Sender<RebuildMsg>,
    rebuild_rx: Receiver<RebuildMsg>,
    /// To wake the UI up when a background job is done.
    repaint: Context,
    task_tx: Sender<TaskMsg>,
    task_rx: Receiver<TaskMsg>,
    pub busy: Option<String>,
    pub export: ExportSettings,
    /// "Image format" dialog: (export waiting, size being chosen, keep the 6 : 4.2 ratio).
    pub size_dialog: Option<(ImageExport, ImageSize, bool)>,
    /// Export Data page: settings (saved) and state (plan, running export).
    pub data_export: crate::export::data::DataExportConfig,
    pub dx: crate::ui::data_export::DataExportState,
    next_id: u64,
    pub workspace_path: Option<PathBuf>,
    /// Start screen (choose / create a workspace) shown instead of the pages.
    pub show_start: bool,
    pub start: StartState,
    pub recent: Vec<crate::recent::RecentWorkspace>,
    /// Workspace content when it was last opened / saved (change detection).
    saved_snapshot: Option<String>,
    /// Take the snapshot once the data sources are loaded.
    snapshot_pending: bool,
    pub pending_switch: Option<Switch>,
    /// The user confirmed closing the window.
    quitting: bool,
    /// (datasets key, memory used).
    mem_cache: std::cell::Cell<(u64, usize)>,
    /// Plots page: left-drag draws a zoom rectangle instead of panning.
    pub box_zoom: bool,
    /// Ctrl+Z / Ctrl+Y on the plots (markers, deleted / added / moved plots).
    undo: Vec<UndoAction>,
    redo: Vec<UndoAction>,
    /// Size of the plot area of the Plots page (thumbnails use its proportions).
    pub plot_area: egui::Vec2,
    /// (time, result) of the last change detection (it serializes the workspace).
    dirty_check: std::cell::Cell<(f64, bool)>,
    pub import_defaults: ImportOptions,
    pub data_page: DataPageState,
    pub dark: bool,
}

pub fn setup_style(ctx: &Context, dark: bool) {
    ctx.set_theme(if dark { egui::Theme::Dark } else { egui::Theme::Light });
    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(8.0, 5.0);
        style.spacing.button_padding = egui::vec2(8.0, 3.0);
        style.spacing.interact_size.y = 22.0;
    });
    ctx.style_mut_of(egui::Theme::Light, |style| {
        let v = &mut style.visuals;
        v.panel_fill = Color32::from_rgb(241, 243, 247);
        v.window_fill = Color32::from_rgb(250, 251, 253);
        v.faint_bg_color = Color32::from_rgb(232, 236, 242);
        v.extreme_bg_color = Color32::WHITE;
        v.selection.bg_fill = Color32::from_rgb(52, 92, 160);
        v.selection.stroke.color = Color32::WHITE;
        v.hyperlink_color = Color32::from_rgb(30, 90, 190);
    });
}

impl App {
    pub fn new(ctx: &Context) -> App {
        setup_style(ctx, false);
        let mut fonts = FontBook::new(epaint_default_fonts::UBUNTU_LIGHT, epaint_default_fonts::HACK_REGULAR);
        fonts.scan_system();
        let (load_tx, load_rx) = channel();
        let (rebuild_tx, rebuild_rx) = channel();
        let (task_tx, task_rx) = channel();
        let mut app = App {
            sources: Vec::new(),
            plots: Vec::new(),
            functions: Vec::new(),
            func_dirty: false,
            page: Page::Files,
            active: 0,
            settings_tab: SettingsTab::Data,
            data_tab: DataTab::Select,
            fonts: Arc::new(fonts),
            fonts_installed: 0,
            status: None,
            load_tx,
            load_rx,
            rebuild_tx,
            rebuild_rx,
            repaint: ctx.clone(),
            task_tx,
            task_rx,
            busy: None,
            export: ExportSettings::default(),
            size_dialog: None,
            data_export: Default::default(),
            dx: Default::default(),
            next_id: 1,
            workspace_path: None,
            show_start: true,
            start: StartState {
                new_name: String::new(),
                new_dir: crate::recent::default_workspace_dir(),
                error: None,
            },
            recent: crate::recent::load(),
            saved_snapshot: None,
            snapshot_pending: false,
            pending_switch: None,
            quitting: false,
            mem_cache: std::cell::Cell::new((u64::MAX, 0)),
            box_zoom: false,
            undo: Vec::new(),
            redo: Vec::new(),
            plot_area: egui::vec2(0.0, 0.0),
            dirty_check: std::cell::Cell::new((f64::NEG_INFINITY, false)),
            import_defaults: ImportOptions::default(),
            data_page: DataPageState::default(),
            dark: false,
        };
        app.install_fonts(ctx);
        // files given on the command line
        for arg in std::env::args().skip(1) {
            let p = PathBuf::from(arg);
            if p.extension().is_some_and(|e| e.eq_ignore_ascii_case(WORKSPACE_EXT)) {
                app.load_workspace(&p, ctx);
            } else if p.is_file() {
                app.add_source(p, None, ctx);
                app.show_start = false;
            }
        }
        // new workspaces go next to the last one used
        if let Some(dir) = app.recent.first().and_then(|r| r.path.parent()) {
            app.start.new_dir = dir.to_path_buf();
        }
        app
    }

    /// Registers the fonts of the font book in egui.
    pub fn install_fonts(&mut self, ctx: &Context) {
        let mut defs = FontDefinitions::default();
        // Hack has many symbols (arrows, boxes…) missing from the proportional font.
        if let Some(prop) = defs.families.get_mut(&FontFamily::Proportional) {
            prop.push("Hack".to_owned());
        }
        for f in self.fonts.faces.iter().filter(|f| !f.builtin) {
            defs.font_data
                .insert(f.name.clone(), Arc::new(FontData::from_owned(f.data.as_ref().clone())));
            defs.families.insert(
                FontFamily::Name(f.name.as_str().into()),
                vec![
                    f.name.clone(),
                    "Ubuntu-Light".to_owned(),
                    "NotoEmoji-Regular".to_owned(),
                    "emoji-icon-font".to_owned(),
                    "Hack".to_owned(),
                ],
            );
        }
        ctx.set_fonts(defs);
        self.fonts_installed = self.fonts.faces.len();
    }

    pub fn add_font_file(&mut self, path: &Path, ctx: &Context) {
        let mut book = (*self.fonts).clone();
        match book.add_file(path) {
            Ok(name) => {
                self.fonts = Arc::new(book);
                self.install_fonts(ctx);
                self.set_status(format!("Font '{name}' loaded"), false);
            }
            Err(e) => self.set_status(e, true),
        }
    }

    pub fn set_status(&mut self, text: impl Into<String>, error: bool) {
        self.status = Some(Status {
            text: text.into(),
            error,
            at: Instant::now(),
        });
    }

    /// Context to wake the user interface up from a background job.
    pub fn repaint_ctx(&self) -> Context {
        self.repaint.clone()
    }

    fn new_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    // ---- data sources -------------------------------------------------------

    pub fn source_key(path: &Path) -> String {
        std::fs::canonicalize(path)
            .unwrap_or_else(|_| path.to_path_buf())
            .display()
            .to_string()
    }

    /// Adds (or reloads) a data source and loads it in the background.
    pub fn add_source(&mut self, path: PathBuf, options: Option<ImportOptions>, ctx: &Context) {
        let key = Self::source_key(&path);
        let options = options.unwrap_or_else(|| self.import_defaults.clone());
        if let Some(s) = self.sources.iter_mut().find(|s| s.key == key) {
            s.options = options;
            s.loading = true;
            s.error = None;
        } else {
            self.sources.push(DataSource {
                key: key.clone(),
                path: path.clone(),
                options,
                base: None,
                deembed: DeembedConfig::default(),
                deembedded: None,
                deembed_report: None,
                ds: None,
                func_errors: Vec::new(),
                loading: true,
                rebuilding: false,
                rebuild_ticket: 0,
                error: None,
                show_options: false,
                show_deembed: false,
            });
        }
        let opts = self
            .sources
            .iter()
            .find(|s| s.key == key)
            .map(|s| s.options.clone())
            .unwrap_or_default();
        let tx = self.load_tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            // a panic in a loader must not leave the source "Loading…" forever
            let result = std::panic::catch_unwind(|| load_path(&path, &opts))
                .unwrap_or_else(|_| Err(format!("Cannot read {}: unexpected file content", path.display())));
            let _ = tx.send(LoadMsg { key, result });
            ctx.request_repaint();
        });
    }

    pub fn reload_source(&mut self, i: usize, ctx: &Context) {
        if let Some(s) = self.sources.get(i) {
            let (path, opts) = (s.path.clone(), s.options.clone());
            self.add_source(path, Some(opts), ctx);
        }
    }

    /// Applies the de-embedding settings of a data source.
    pub fn apply_deembed(&mut self, i: usize) {
        self.start_rebuild(i, true, true);
    }

    /// Recomputes the de-embedding (`redo`) and the user functions of a data
    /// source in the background; the current dataset stays usable meanwhile.
    fn start_rebuild(&mut self, i: usize, redo: bool, announce: bool) {
        let Some(s) = self.sources.get_mut(i) else {
            return;
        };
        let Some(base) = s.base.clone() else {
            return;
        };
        s.rebuild_ticket += 1;
        s.rebuilding = true;
        let ticket = s.rebuild_ticket;
        let key = s.key.clone();
        let cfg = s.deembed.clone();
        let previous = (s.deembedded.clone(), s.deembed_report.clone());
        let funcs = self.functions.clone();
        let tx = self.rebuild_tx.clone();
        let ctx = self.repaint.clone();
        std::thread::spawn(move || {
            let (deembedded, report, ds, func_errors) = rebuild(&base, &cfg, redo, previous, &funcs);
            let _ = tx.send(RebuildMsg {
                key,
                ticket,
                deembedded,
                report,
                ds,
                func_errors,
                announce,
            });
            ctx.request_repaint();
        });
    }

    pub fn remove_source(&mut self, i: usize) {
        if i < self.sources.len() {
            self.sources.remove(i);
            if i < self.data_page.source {
                self.data_page.source -= 1;
            }
        }
        if self.data_page.source >= self.sources.len() {
            self.data_page.source = self.sources.len().saturating_sub(1);
        }
    }

    pub fn open_files_dialog(&mut self, ctx: &Context) {
        let mut dlg = rfd::FileDialog::new()
            .set_title("Open measurement files")
            .add_filter(
                "Data files",
                &[
                    "csv", "txt", "tsv", "dat", "xlsx", "xlsm", "xlsb", "xls", "ods", "s1p", "s2p", "s3p", "s4p",
                    "s5p", "s6p", "s8p", "s12p", "s16p",
                ],
            )
            .add_filter(
                "Touchstone (.snp)",
                &["s1p", "s2p", "s3p", "s4p", "s5p", "s6p", "s8p", "s12p", "s16p"],
            )
            .add_filter("All files", &["*"]);
        if let Some(d) = &self.export.last_dir {
            dlg = dlg.set_directory(d);
        }
        if let Some(paths) = dlg.pick_files() {
            for p in paths {
                if let Some(parent) = p.parent() {
                    self.export.last_dir = Some(parent.to_path_buf());
                }
                self.add_source(p, None, ctx);
            }
        }
    }

    pub fn dataset(&self, key: &str) -> Option<&Arc<Dataset>> {
        self.sources.iter().find(|s| s.key == key).and_then(|s| s.ds.as_ref())
    }

    pub fn source_name(&self, key: &str) -> String {
        self.sources
            .iter()
            .find(|s| s.key == key)
            .map(|s| s.name())
            .unwrap_or_else(|| {
                Path::new(key)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            })
    }

    /// Source name, with the de-embedding used (plot settings of the exports).
    pub fn source_description(&self, key: &str) -> String {
        let name = self.source_name(key);
        match self
            .sources
            .iter()
            .find(|s| s.key == key)
            .and_then(|s| s.deembed_report.as_ref())
        {
            Some(Ok(r)) => {
                let files: Vec<String> = [true, false]
                    .into_iter()
                    .filter_map(|input| self.fixture_name(key, input))
                    .collect();
                format!(
                    "{name} – de-embedded ({}, {})",
                    if r.vector { "vector" } else { "scalar" },
                    files.join(" / ")
                )
            }
            _ => name,
        }
    }

    fn fixture_name(&self, key: &str, input: bool) -> Option<String> {
        let s = self.sources.iter().find(|s| s.key == key)?;
        let fx = if input { &s.deembed.input } else { &s.deembed.output };
        let name = fx.path.as_ref()?.file_name()?.to_string_lossy().into_owned();
        Some(format!("{} {name}", if input { "in" } else { "out" }))
    }

    /// Recomputes the user function columns of every data source.
    pub fn apply_functions(&mut self) {
        for i in 0..self.sources.len() {
            self.start_rebuild(i, false, false);
        }
        self.func_dirty = false;
    }

    /// Handles background messages (loads, exports).
    pub fn poll(&mut self) {
        while let Ok(msg) = self.load_rx.try_recv() {
            let Some(s) = self.sources.iter_mut().find(|s| s.key == msg.key) else {
                continue;
            };
            s.loading = false;
            match msg.result {
                Ok(mut ds) => {
                    ds.generation = crate::data::next_generation();
                    let text = format!(
                        "Loaded {} – {} rows × {} columns in {:.0} ms",
                        ds.name,
                        ds.n_rows,
                        ds.columns.len(),
                        ds.info.load_ms
                    );
                    if let Some(sheet) = &ds.info.sheet {
                        s.options.sheet = Some(sheet.clone());
                    }
                    s.base = Some(Arc::new(ds));
                    s.error = None;
                    let key = msg.key.clone();
                    if let Some(i) = self.sources.iter().position(|x| x.key == key) {
                        self.start_rebuild(i, true, false);
                    }
                    self.status = Some(Status {
                        text,
                        error: false,
                        at: Instant::now(),
                    });
                }
                Err(e) => {
                    s.error = Some(e.clone());
                    self.status = Some(Status {
                        text: e,
                        error: true,
                        at: Instant::now(),
                    });
                }
            }
        }
        while let Ok(msg) = self.rebuild_rx.try_recv() {
            let Some(s) = self.sources.iter_mut().find(|s| s.key == msg.key) else {
                continue;
            };
            if msg.ticket != s.rebuild_ticket {
                continue; // settings changed meanwhile: a newer rebuild is running
            }
            s.rebuilding = false;
            s.deembedded = msg.deembedded;
            s.deembed_report = msg.report;
            s.ds = Some(Arc::new(msg.ds));
            s.func_errors = msg.func_errors;
            if msg.announce {
                let name = s.name();
                let (t, err) = match &s.deembed_report {
                    Some(Ok(r)) => (format!("{name}: {}", r.summary()), false),
                    Some(Err(e)) => (format!("{name}: {e}"), true),
                    None => (format!("{name}: de-embedding off"), false),
                };
                self.set_status(t, err);
            }
        }
        if self.snapshot_pending && self.sources.iter().all(|s| !s.loading && !s.rebuilding) {
            self.snapshot_pending = false;
            self.saved_snapshot = Some(self.snapshot());
        }
        crate::ui::data_export::poll(self);
        while let Ok(msg) = self.task_rx.try_recv() {
            self.busy = None;
            match msg {
                TaskMsg::Done(t) => self.set_status(t, false),
                TaskMsg::Failed(t) => self.set_status(t, true),
            }
        }
    }

    pub fn update_plots(&mut self, ctx: &Context) {
        let sources = &self.sources;
        for p in &mut self.plots {
            let ds = sources
                .iter()
                .find(|s| s.key == p.cfg.source)
                .and_then(|s| s.ds.as_ref());
            p.state.update(&p.cfg, ds, ctx);
        }
    }

    pub fn handle_dropped_files(&mut self, ctx: &Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        for p in dropped {
            let ext = p
                .extension()
                .map(|e| e.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default();
            if ext == WORKSPACE_EXT {
                self.request_switch(Switch::Open(p), ctx);
            } else if ext == "ttf" || ext == "otf" {
                self.add_font_file(&p, ctx);
            } else {
                self.add_source(p, None, ctx);
                self.show_start = false;
                if self.page != Page::Plots {
                    self.page = Page::Files;
                }
            }
        }
    }

    /// Memory used by the data (status bar): recomputed only when a dataset
    /// changes (it walks every dictionary).
    pub fn memory_bytes(&self) -> usize {
        let key = self
            .sources
            .iter()
            .filter_map(|s| s.ds.as_ref())
            .fold(self.sources.len() as u64, |k, d| {
                k.wrapping_mul(0x100_0000_01B3) ^ d.generation
            });
        let (k, v) = self.mem_cache.get();
        if k == key {
            return v;
        }
        let v = self
            .sources
            .iter()
            .filter_map(|s| s.ds.as_ref())
            .map(|d| d.memory_bytes())
            .sum();
        self.mem_cache.set((key, v));
        v
    }

    // ---- plots ----------------------------------------------------------------

    /// Creates a plot on a data source and makes it active.
    pub fn add_plot(&mut self, source_key: &str) -> usize {
        let id = self.new_id();
        let mut cfg = PlotConfig {
            id,
            name: format!("Plot {}", self.plots.len() + 1),
            source: source_key.to_owned(),
            ..Default::default()
        };
        if let Some(ds) = self.dataset(source_key) {
            // sensible defaults: a frequency column for X (in GHz)
            if let Some(c) = ds
                .columns
                .iter()
                .find(|c| c.name.to_ascii_lowercase().starts_with("freq") && c.has_numbers())
            {
                cfg.x = c.name.clone();
                if c.max.abs() >= 1e8 {
                    cfg.x_scale.modifier = crate::plot::config::Modifier::Giga;
                }
            } else if let Some(c) = ds
                .columns
                .iter()
                .find(|c| c.kind == ColumnKind::Numeric && c.n_unique > 1)
            {
                cfg.x = c.name.clone();
            }
            // Touchstone file: S21 (or S11) in dB
            if matches!(ds.info.kind, crate::data::dataset::FileKind::Touchstone(_)) {
                let y = ["S21db", "S11db"].into_iter().find(|n| ds.column(n).is_some());
                cfg.ys = y.map(|n| vec![n.to_owned()]).unwrap_or_default();
                cfg.name = format!("Plot {} – {}", self.plots.len() + 1, ds.name);
            }
        }
        // inherit the look of the previous plot
        if let Some(last) = self.plots.last() {
            cfg.layout = last.cfg.layout.clone();
            cfg.layout.title.clear();
            cfg.layout.x.label.clear();
            cfg.layout.y.label.clear();
            cfg.layout.x.auto_range = true;
            cfg.layout.y.auto_range = true;
            cfg.traces = last.cfg.traces.clone();
            cfg.options = last.cfg.options.clone();
        }
        self.plots.push(PlotEntry {
            cfg,
            state: PlotState::default(),
            ui: PlotUiState::default(),
        });
        self.active = self.plots.len() - 1;
        self.push_undo(UndoAction::Added { id });
        self.page = Page::Plots;
        self.settings_tab = SettingsTab::Data;
        self.data_tab = DataTab::Select;
        self.active
    }

    pub fn duplicate_plot(&mut self, i: usize) {
        let Some(p) = self.plots.get(i) else {
            return;
        };
        let mut cfg = p.cfg.clone();
        cfg.id = self.new_id();
        cfg.name = self.next_plot_name(&cfg.name);
        let id = cfg.id;
        self.plots.insert(
            i + 1,
            PlotEntry {
                cfg,
                state: PlotState::default(),
                ui: PlotUiState::default(),
            },
        );
        self.active = i + 1;
        self.push_undo(UndoAction::Added { id });
    }

    /// Name of a copy: the number of the name incremented ("Plot 3" →
    /// "Plot 4", "S21" → "S21 2"), the first one not used by another plot.
    fn next_plot_name(&self, name: &str) -> String {
        next_free_name(name, |n| self.plots.iter().any(|p| p.cfg.name == n))
    }

    /// Moves a plot one place up (`-1`) or down (`+1`).
    pub fn move_plot(&mut self, i: usize, dir: isize) {
        let j = i as isize + dir;
        if j >= 0 {
            self.move_plot_to(i, j as usize);
        }
    }

    /// Moves the plot at `from` so that it ends at position `to` (drag & drop).
    pub fn move_plot_to(&mut self, from: usize, to: usize) {
        if self.move_plot_raw(from, to) {
            self.push_undo(UndoAction::Moved { from, to });
        }
    }

    fn move_plot_raw(&mut self, from: usize, to: usize) -> bool {
        let n = self.plots.len();
        if from >= n || to >= n || from == to {
            return false;
        }
        let active_id = self.plots.get(self.active).map(|p| p.cfg.id);
        let p = self.plots.remove(from);
        self.plots.insert(to, p);
        // the shown plot stays shown
        if let Some(id) = active_id {
            self.active = self.plots.iter().position(|p| p.cfg.id == id).unwrap_or(0);
        }
        true
    }

    pub fn remove_plot(&mut self, i: usize) {
        if let Some(cfg) = self.remove_plot_raw(i) {
            self.push_undo(UndoAction::Removed {
                index: i,
                cfg: Box::new(cfg),
            });
        }
    }

    fn remove_plot_raw(&mut self, i: usize) -> Option<PlotConfig> {
        let removed = (i < self.plots.len()).then(|| self.plots.remove(i).cfg);
        if removed.is_some() && i < self.active {
            // the shown plot stays shown when a plot above it is deleted
            self.active -= 1;
        }
        if self.active >= self.plots.len() {
            self.active = self.plots.len().saturating_sub(1);
        }
        removed
    }

    // ---- undo / redo ------------------------------------------------------------

    pub fn push_undo(&mut self, a: UndoAction) {
        self.undo.push(a);
        if self.undo.len() > 200 {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self) {
        if let Some(a) = self.undo.pop()
            && let Some(inverse) = self.apply_undo(a)
        {
            self.redo.push(inverse);
        }
    }

    pub fn redo(&mut self) {
        if let Some(a) = self.redo.pop()
            && let Some(inverse) = self.apply_undo(a)
        {
            self.undo.push(inverse);
        }
    }

    /// Reverts an action; returns the action that re-does it.
    fn apply_undo(&mut self, a: UndoAction) -> Option<UndoAction> {
        let text;
        let inverse = match a {
            UndoAction::Markers { id, markers } => {
                let i = self.plots.iter().position(|p| p.cfg.id == id)?;
                let current = std::mem::replace(&mut self.plots[i].cfg.markers, markers);
                self.active = i;
                text = "markers";
                UndoAction::Markers { id, markers: current }
            }
            UndoAction::Removed { index, cfg } => {
                let id = cfg.id;
                let index = index.min(self.plots.len());
                self.plots.insert(
                    index,
                    PlotEntry {
                        cfg: *cfg,
                        state: PlotState::default(),
                        ui: PlotUiState::default(),
                    },
                );
                self.active = index;
                text = "plot restored";
                UndoAction::Added { id }
            }
            UndoAction::Added { id } => {
                let index = self.plots.iter().position(|p| p.cfg.id == id)?;
                let cfg = self.remove_plot_raw(index)?;
                text = "plot removed";
                UndoAction::Removed {
                    index,
                    cfg: Box::new(cfg),
                }
            }
            UndoAction::Moved { from, to } => {
                self.move_plot_raw(to, from).then_some(())?;
                self.active = from;
                text = "plot moved back";
                UndoAction::Moved { from: to, to: from }
            }
        };
        self.set_status(format!("Undo / redo: {text}"), false);
        Some(inverse)
    }

    pub fn plots_using(&self, key: &str) -> usize {
        self.plots.iter().filter(|p| p.cfg.source == key).count()
    }

    // ---- workspace ------------------------------------------------------------

    pub fn workspace(&self) -> Workspace {
        Workspace {
            app: "CurveTracer".to_owned(),
            version: 1,
            sources: self
                .sources
                .iter()
                .map(|s| WorkspaceSource {
                    path: s.path.clone(),
                    options: s.options.clone(),
                    deembed: s.deembed.clone(),
                })
                .collect(),
            functions: self.functions.clone(),
            plots: self.plots.iter().map(|p| p.cfg.clone()).collect(),
            export: self.export.clone(),
            fonts: self
                .fonts
                .faces
                .iter()
                .filter(|f| f.user)
                .filter_map(|f| f.path.clone())
                .collect(),
            data_export: self.data_export.clone(),
        }
    }

    pub fn save_workspace(&mut self, path: &Path) {
        let ws = self.workspace();
        match serde_json::to_string_pretty(&ws) {
            Ok(text) => match std::fs::write(path, text) {
                Ok(()) => {
                    self.workspace_path = Some(path.to_path_buf());
                    self.saved_snapshot = Some(self.snapshot());
                    crate::recent::touch(&mut self.recent, path);
                    self.set_status(format!("Workspace saved to {}", path.display()), false);
                }
                Err(e) => self.set_status(format!("Cannot write {}: {e}", path.display()), true),
            },
            Err(e) => self.set_status(format!("Cannot serialize the workspace: {e}"), true),
        }
    }

    pub fn save_workspace_dialog(&mut self, save_as: bool) {
        let path = if !save_as && self.workspace_path.is_some() {
            self.workspace_path.clone()
        } else {
            let mut dlg = rfd::FileDialog::new()
                .set_title("Save workspace")
                .add_filter("CurveTracer workspace", &[WORKSPACE_EXT])
                .set_file_name(format!("workspace.{WORKSPACE_EXT}"));
            if let Some(d) = &self.export.last_dir {
                dlg = dlg.set_directory(d);
            }
            dlg.save_file()
        };
        if let Some(mut p) = path {
            if p.extension().is_none() {
                p.set_extension(WORKSPACE_EXT);
            }
            self.save_workspace(&p);
        }
    }

    pub fn open_workspace_dialog(&mut self, ctx: &Context) {
        let mut dlg = rfd::FileDialog::new()
            .set_title("Open workspace")
            .add_filter("CurveTracer workspace", &[WORKSPACE_EXT]);
        if let Some(d) = &self.export.last_dir {
            dlg = dlg.set_directory(d);
        }
        if let Some(p) = dlg.pick_file() {
            if let Some(parent) = p.parent() {
                self.export.last_dir = Some(parent.to_path_buf());
            }
            self.load_workspace(&p, ctx);
        }
    }

    pub fn load_workspace(&mut self, path: &Path, ctx: &Context) {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => {
                self.set_status(format!("Cannot read {}: {e}", path.display()), true);
                return;
            }
        };
        let ws: Workspace = match serde_json::from_str(&text) {
            Ok(w) => w,
            Err(e) => {
                self.set_status(format!("Invalid workspace file: {e}"), true);
                return;
            }
        };
        self.reset_project();
        let mut book = (*self.fonts).clone();
        for f in &ws.fonts {
            let _ = book.add_file(f);
        }
        self.fonts = Arc::new(book);
        self.install_fonts(ctx);
        self.functions = ws.functions;
        self.export = ws.export;
        self.sources.clear();
        self.plots.clear();
        let mut key_map = Vec::new();
        for s in ws.sources {
            let old_key = s.path.display().to_string();
            let new_key = Self::source_key(&s.path);
            key_map.push((old_key, new_key.clone()));
            self.add_source(s.path, Some(s.options), ctx);
            if let Some(src) = self.sources.iter_mut().find(|x| x.key == new_key) {
                src.deembed = s.deembed;
            }
        }
        self.data_export = ws.data_export;
        if let Some((_, k)) = key_map
            .iter()
            .find(|(o, n)| *o == self.data_export.source || *n == self.data_export.source)
        {
            self.data_export.source = k.clone();
        }
        for mut cfg in ws.plots {
            if let Some((_, k)) = key_map.iter().find(|(o, n)| *o == cfg.source || *n == cfg.source) {
                cfg.source = k.clone();
            }
            self.next_id = self.next_id.max(cfg.id + 1);
            self.plots.push(PlotEntry {
                cfg,
                state: PlotState::default(),
                ui: PlotUiState::default(),
            });
        }
        // make the plot ids unique
        let mut seen = BTreeSet::new();
        for i in 0..self.plots.len() {
            if !seen.insert(self.plots[i].cfg.id) {
                let id = self.new_id();
                self.plots[i].cfg.id = id;
                seen.insert(id);
            }
        }
        self.active = 0;
        self.page = if self.plots.is_empty() {
            Page::Files
        } else {
            Page::Plots
        };
        self.workspace_path = Some(path.to_path_buf());
        self.snapshot_pending = true;
        self.show_start = false;
        crate::recent::touch(&mut self.recent, path);
        self.set_status(format!("Workspace {} opened", path.display()), false);
    }

    /// Workspace name (file name without extension).
    pub fn workspace_name(&self) -> Option<String> {
        self.workspace_path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|n| n.to_string_lossy().into_owned())
    }

    /// Workspace content, without what changes by itself (folders of the dialogs).
    fn snapshot(&self) -> String {
        let mut ws = self.workspace();
        ws.export.last_dir = None;
        if let Some((before, after)) = &self.dx.auto_defaults
            && *after == ws.data_export
        {
            ws.data_export = before.clone();
        }
        serde_json::to_string(&ws).unwrap_or_default()
    }

    /// The workspace has changes that are not saved.
    pub fn is_dirty(&self) -> bool {
        let empty = self.sources.is_empty() && self.plots.is_empty() && self.functions.is_empty();
        match &self.saved_snapshot {
            Some(s) => !self.snapshot_pending && *s != self.snapshot(),
            None => !empty && !self.snapshot_pending,
        }
    }

    /// `is_dirty`, recomputed at most twice a second (for the top bar).
    pub fn is_dirty_cached(&self, now: f64) -> bool {
        let (t, d) = self.dirty_check.get();
        if (now - t).abs() < 0.5 {
            return d;
        }
        let d = self.is_dirty();
        self.dirty_check.set((now, d));
        d
    }

    /// Empties the project (data sources, plots, functions).
    fn reset_project(&mut self) {
        self.undo.clear();
        self.redo.clear();
        // fonts loaded from files belong to the workspace that loaded them
        if self.fonts.faces.iter().any(|f| f.user) {
            let mut book = (*self.fonts).clone();
            book.faces.retain(|f| !f.user);
            self.fonts = Arc::new(book);
        }
        self.sources.clear();
        self.plots.clear();
        self.functions.clear();
        let last_dir = self.export.last_dir.take();
        self.export = ExportSettings {
            last_dir,
            ..Default::default()
        };
        self.data_export = Default::default();
        self.dx = Default::default();
        self.workspace_path = None;
        self.saved_snapshot = None;
        self.snapshot_pending = false;
        self.active = 0;
        self.page = Page::Files;
    }

    /// Changes of workspace: asks first when the current one has unsaved changes.
    pub fn request_switch(&mut self, sw: Switch, ctx: &Context) {
        if self.is_dirty() {
            self.pending_switch = Some(sw);
        } else {
            self.perform_switch(sw, ctx);
        }
    }

    pub fn perform_switch(&mut self, sw: Switch, ctx: &Context) {
        self.pending_switch = None;
        match sw {
            Switch::Open(p) => self.load_workspace(&p, ctx),
            Switch::OpenDialog => self.open_workspace_dialog(ctx),
            Switch::Close => {
                self.reset_project();
                self.recent = crate::recent::load();
                self.show_start = true;
            }
            Switch::Quit => {
                self.quitting = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    /// Creates an empty workspace file and opens it.
    pub fn create_workspace(&mut self, path: &Path) -> Result<(), String> {
        if path.exists() {
            return Err(format!("{} already exists", path.display()));
        }
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d).map_err(|e| format!("Cannot create {}: {e}", d.display()))?;
        }
        self.reset_project();
        self.save_workspace(path);
        if self.workspace_path.is_none() {
            return Err(self.status.as_ref().map(|s| s.text.clone()).unwrap_or_default());
        }
        self.show_start = false;
        self.set_status(format!("New workspace {} created", path.display()), false);
        Ok(())
    }

    // ---- exports --------------------------------------------------------------

    /// Plots selected for export, with their computed data.
    fn export_items(&mut self) -> Vec<ExportItem> {
        let sources = &self.sources;
        for p in &mut self.plots {
            let ds = sources
                .iter()
                .find(|s| s.key == p.cfg.source)
                .and_then(|s| s.ds.as_ref());
            p.state.wait_current(&p.cfg, ds);
        }
        let mut items = Vec::new();
        for p in self.plots.iter().filter(|p| p.cfg.export) {
            if let Some(d) = &p.state.data {
                let view = if self.export.use_view { p.state.view } else { None };
                items.push((
                    p.cfg.clone(),
                    Arc::clone(d),
                    view,
                    self.source_description(&p.cfg.source),
                ));
            }
        }
        items
    }

    fn render_options(&self, view: Option<[f64; 4]>) -> RenderOptions {
        let s = self.export.size;
        let mut o = RenderOptions::physical(s.w_in, s.h_in, s.dpi as f32, self.screen_plot_width());
        o.view = view;
        o
    }

    /// Width of the plots on the screen (points): the exported images look
    /// the same (texts, legend and lines keep their proportions).
    pub fn screen_plot_width(&self) -> f32 {
        if self.plot_area.x >= 200.0 {
            self.plot_area.x
        } else {
            860.0
        }
    }

    /// Exports images (PowerPoint, PNG / SVG): directly in the 6 : 4.2
    /// format, otherwise after asking the format.
    pub fn request_image_export(&mut self, what: ImageExport) {
        if self.export.ask_size || !self.export.size.standard_ratio() {
            let keep = self.export.size.standard_ratio();
            self.size_dialog = Some((what, self.export.size, keep));
        } else {
            self.run_image_export(what);
        }
    }

    pub fn run_image_export(&mut self, what: ImageExport) {
        match what {
            ImageExport::Pptx => self.export_pptx_dialog(),
            ImageExport::Images => self.export_images_dialog(),
            ImageExport::Plot(i, fmt) => self.save_plot_image_dialog(i, fmt),
        }
    }

    pub fn export_pptx_dialog(&mut self) {
        let mut dlg = rfd::FileDialog::new()
            .set_title("Export PowerPoint")
            .add_filter("PowerPoint", &["pptx"])
            .set_file_name(format!("{}.pptx", sanitize(&self.export.file_name)));
        if let Some(d) = &self.export.last_dir {
            dlg = dlg.set_directory(d);
        }
        let Some(mut path) = dlg.save_file() else {
            return;
        };
        if path.extension().is_none() {
            path.set_extension("pptx");
        }
        if let Some(parent) = path.parent() {
            self.export.last_dir = Some(parent.to_path_buf());
        }
        let items = self.export_items();
        if items.is_empty() {
            self.set_status("Nothing to export: select at least one computed plot", true);
            return;
        }
        let opts: Vec<RenderOptions> = items.iter().map(|it| self.render_options(it.2)).collect();
        let fonts = Arc::clone(&self.fonts);
        let format = self.export.format;
        let with_svg = self.export.pptx_svg;
        let title = self.export.file_name.clone();
        let tx = self.task_tx.clone();
        self.busy = Some(format!("Exporting {} slide(s)…", items.len()));
        std::thread::spawn(move || {
            let mut slides = Vec::new();
            for ((cfg, data, _, src), o) in items.iter().zip(&opts) {
                // the slide already shows the plot title: do not repeat it in the image
                let mut img_cfg = cfg.clone();
                if cfg.slide_title.trim().is_empty() {
                    img_cfg.layout.show_title = false;
                }
                match render_png(&img_cfg, data, &fonts, o) {
                    Ok(png) => slides.push(Slide {
                        title: cfg.display_title(),
                        png,
                        svg: with_svg.then(|| render_svg(&img_cfg, data, &fonts, o).into_bytes()),
                        img_w: o.width,
                        img_h: o.height,
                        size_in: o.dpi.map(|d| (o.width as f64 / d as f64, o.height as f64 / d as f64)),
                        settings: plot_settings(cfg, data, src),
                    }),
                    Err(e) => {
                        let _ = tx.send(TaskMsg::Failed(format!("{}: {e}", cfg.name)));
                        return;
                    }
                }
            }
            let bytes = build_pptx(&slides, &title, format);
            let msg = match std::fs::write(&path, bytes) {
                Ok(()) => TaskMsg::Done(format!(
                    "PowerPoint exported to {} ({} slides)",
                    path.display(),
                    slides.len()
                )),
                Err(e) => TaskMsg::Failed(format!("Cannot write {}: {e}", path.display())),
            };
            let _ = tx.send(msg);
        });
    }

    pub fn export_images_dialog(&mut self) {
        let fmt = self.export.image_format;
        let mut dlg = rfd::FileDialog::new().set_title(format!(
            "Export the plots as {} images: choose a folder",
            fmt.ext().to_uppercase()
        ));
        if let Some(d) = &self.export.last_dir {
            dlg = dlg.set_directory(d);
        }
        let Some(dir) = dlg.pick_folder() else {
            return;
        };
        self.export.last_dir = Some(dir.clone());
        let items = self.export_items();
        let opts: Vec<RenderOptions> = items.iter().map(|it| self.render_options(it.2)).collect();
        let fonts = Arc::clone(&self.fonts);
        let prefix = sanitize(&self.export.file_name);
        let tx = self.task_tx.clone();
        self.busy = Some(format!("Exporting {} image(s)…", items.len()));
        std::thread::spawn(move || {
            let mut n = 0;
            for (k, ((cfg, data, _, _), o)) in items.iter().zip(&opts).enumerate() {
                let file = dir.join(format!(
                    "{prefix}_{:02}_{}.{}",
                    k + 1,
                    sanitize(&cfg.display_title()),
                    fmt.ext()
                ));
                match render_image(cfg, data, &fonts, o, fmt)
                    .and_then(|bytes| std::fs::write(&file, bytes).map_err(|e| e.to_string()))
                {
                    Ok(()) => n += 1,
                    Err(e) => {
                        let _ = tx.send(TaskMsg::Failed(format!("{}: {e}", file.display())));
                        return;
                    }
                }
            }
            let _ = tx.send(TaskMsg::Done(format!("{n} image(s) written to {}", dir.display())));
        });
    }

    pub fn export_csv_dialog(&mut self) {
        let mut dlg = rfd::FileDialog::new().set_title("Export the plot data (CSV): choose a folder");
        if let Some(d) = &self.export.last_dir {
            dlg = dlg.set_directory(d);
        }
        let Some(dir) = dlg.pick_folder() else {
            return;
        };
        self.export.last_dir = Some(dir.clone());
        let items = self.export_items();
        let prefix = sanitize(&self.export.file_name);
        let mut n = 0;
        for (k, (cfg, data, _, _)) in items.iter().enumerate() {
            let file = dir.join(format!("{prefix}_{:02}_{}.csv", k + 1, sanitize(&cfg.display_title())));
            let text = traces_to_csv(data, &data.x_label, &data.y_label);
            if let Err(e) = std::fs::write(&file, text) {
                self.set_status(format!("Cannot write {}: {e}", file.display()), true);
                return;
            }
            n += 1;
        }
        self.set_status(format!("{n} CSV file(s) written to {}", dir.display()), false);
    }

    /// Saves the traces of one plot as CSV.
    pub fn save_plot_csv_dialog(&mut self, i: usize) {
        let ds = self
            .plots
            .get(i)
            .and_then(|p| self.sources.iter().find(|s| s.key == p.cfg.source))
            .and_then(|s| s.ds.clone());
        let Some(p) = self.plots.get_mut(i) else {
            return;
        };
        p.state.wait_current(&p.cfg, ds.as_ref());
        let Some(data) = p.state.data.clone() else {
            return;
        };
        let name = sanitize(&p.cfg.display_title());
        let mut dlg = rfd::FileDialog::new()
            .set_title("Save the plot data")
            .add_filter("CSV", &["csv"])
            .set_file_name(format!("{name}.csv"));
        if let Some(d) = &self.export.last_dir {
            dlg = dlg.set_directory(d);
        }
        if let Some(path) = dlg.save_file() {
            let text = traces_to_csv(&data, &data.x_label, &data.y_label);
            match std::fs::write(&path, text) {
                Ok(()) => self.set_status(format!("Plot data saved to {}", path.display()), false),
                Err(e) => self.set_status(format!("Cannot write {}: {e}", path.display()), true),
            }
        }
    }

    /// Saves one plot as a PNG or SVG image.
    pub fn save_plot_image_dialog(&mut self, i: usize, fmt: ImageFormat) {
        let ds = self
            .plots
            .get(i)
            .and_then(|p| self.sources.iter().find(|s| s.key == p.cfg.source))
            .and_then(|s| s.ds.clone());
        let Some(p) = self.plots.get_mut(i) else {
            return;
        };
        p.state.wait_current(&p.cfg, ds.as_ref());
        let Some(data) = p.state.data.clone() else {
            return;
        };
        let cfg = p.cfg.clone();
        let view = p.state.view;
        let (title, filter) = match fmt {
            ImageFormat::Png => ("Save the plot as PNG", "PNG image"),
            ImageFormat::Svg => ("Save the plot as SVG", "SVG vector image"),
        };
        let mut dlg = rfd::FileDialog::new()
            .set_title(title)
            .add_filter(filter, &[fmt.ext()])
            .set_file_name(format!("{}.{}", sanitize(&cfg.display_title()), fmt.ext()));
        if let Some(d) = &self.export.last_dir {
            dlg = dlg.set_directory(d);
        }
        if let Some(mut path) = dlg.save_file() {
            if path.extension().is_none() {
                path.set_extension(fmt.ext());
            }
            if let Some(parent) = path.parent() {
                self.export.last_dir = Some(parent.to_path_buf());
            }
            let o = self.render_options(view);
            match render_image(&cfg, &data, &self.fonts, &o, fmt)
                .and_then(|bytes| std::fs::write(&path, bytes).map_err(|e| e.to_string()))
            {
                Ok(()) => self.set_status(format!("Image saved to {}", path.display()), false),
                Err(e) => self.set_status(e, true),
            }
        }
    }

    /// Writes the (filtered) rows of a data source as CSV.
    pub fn save_dataset_csv_dialog(&mut self, i: usize) {
        let Some(ds) = self.sources.get(i).and_then(|s| s.ds.clone()) else {
            return;
        };
        let stem = Path::new(&ds.name)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "data".to_owned());
        let mut dlg = rfd::FileDialog::new()
            .set_title("Save the data")
            .add_filter("CSV", &["csv"])
            .set_file_name(format!("{stem}_export.csv"));
        if let Some(d) = &self.export.last_dir {
            dlg = dlg.set_directory(d);
        }
        if let Some(path) = dlg.save_file() {
            match write_dataset_csv(&ds, None, &path) {
                Ok(()) => self.set_status(format!("Data saved to {}", path.display()), false),
                Err(e) => self.set_status(e, true),
            }
        }
    }
}

/// Writes rows of a dataset (all, or a subset) as CSV.
pub fn write_dataset_csv(ds: &Dataset, rows: Option<&[u32]>, path: &Path) -> Result<(), String> {
    use std::io::Write as _;
    let f = std::fs::File::create(path).map_err(|e| format!("Cannot create {}: {e}", path.display()))?;
    let mut w = std::io::BufWriter::new(f);
    let q = |s: &str| -> String {
        if s.contains(',') || s.contains('"') || s.contains('\n') {
            format!("\"{}\"", s.replace('"', "\"\""))
        } else {
            s.to_owned()
        }
    };
    let header: Vec<String> = ds.columns.iter().map(|c| q(&c.name)).collect();
    writeln!(w, "{}", header.join(",")).map_err(|e| e.to_string())?;
    let mut line = String::new();
    let mut write_row = |r: usize, w: &mut std::io::BufWriter<std::fs::File>| -> Result<(), String> {
        line.clear();
        for (i, c) in ds.columns.iter().enumerate() {
            if i > 0 {
                line.push(',');
            }
            line.push_str(&q(&c.text(r)));
        }
        writeln!(w, "{line}").map_err(|e| e.to_string())
    };
    match rows {
        Some(rows) => {
            for &r in rows {
                write_row(r as usize, &mut w)?;
            }
        }
        None => {
            for r in 0..ds.n_rows {
                write_row(r, &mut w)?;
            }
        }
    }
    w.flush().map_err(|e| e.to_string())
}

/// `name` with its trailing number incremented until `used` says it is free.
pub fn next_free_name(name: &str, used: impl Fn(&str) -> bool) -> String {
    let name = name.trim();
    // former copies: "Plot 3 (copy)" → "Plot 3"
    let mut base = name;
    while let Some(b) = base.strip_suffix("(copy)") {
        base = b.trim_end();
    }
    let digits = base.len() - base.trim_end_matches(|c: char| c.is_ascii_digit()).len();
    // only a separate number counts: "Plot 3" → "Plot 4", but "S21" → "S21 2"
    let stem = &base[..base.len() - digits];
    let (stem, mut n) = match base[base.len() - digits..].parse::<u64>() {
        Ok(n) if digits > 0 && stem.ends_with(' ') => (stem.to_owned(), n + 1),
        _ => (format!("{base} "), 2),
    };
    loop {
        let candidate = format!("{stem}{n}");
        if !used(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

/// File-name friendly version of a text.
pub fn sanitize(s: &str) -> String {
    let t: String = s
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let t = t.trim_matches('_').to_owned();
    if t.is_empty() {
        "plot".to_owned()
    } else {
        t.chars().take(80).collect()
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // closing the window with unsaved changes: ask first
        if ctx.input(|i| i.viewport().close_requested()) && !self.quitting && self.is_dirty() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.pending_switch = Some(Switch::Quit);
        }
        self.poll();
        self.handle_dropped_files(&ctx);
        if self.fonts_installed != self.fonts.faces.len() {
            self.install_fonts(&ctx);
        }
        self.update_plots(&ctx);
        crate::ui::show(self, ui);
        if self.busy.is_some()
            || self.sources.iter().any(|s| s.loading || s.rebuilding)
            || self.plots.iter().any(|p| p.state.is_computing())
            || self.dx.is_busy()
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::next_free_name;

    #[test]
    fn copies_are_numbered() {
        let used = ["Plot 1", "Plot 2", "Plot 3", "S21", "S21 2"];
        let f = |n: &str| next_free_name(n, |c| used.contains(&c));
        assert_eq!(f("Plot 1"), "Plot 4");
        assert_eq!(f("Plot 3"), "Plot 4");
        assert_eq!(f("S21"), "S21 3");
        assert_eq!(f("Gain"), "Gain 2");
        assert_eq!(f("Plot 2 (copy)"), "Plot 4");
        assert_eq!(f("Run9"), "Run9 2");
        assert_eq!(f("12"), "12 2");
    }
}
