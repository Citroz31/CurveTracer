//! Font registry shared by the GUI (egui) and the PNG renderer (ab_glyph).
//!
//! Two built-in families are always available ("Proportional", "Monospace");
//! common system fonts (Arial, Calibri, Times New Roman, …) are added when
//! found, and any .ttf/.otf file can be loaded by the user.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ab_glyph::FontArc;

pub const PROPORTIONAL: &str = "Proportional";
pub const MONOSPACE: &str = "Monospace";

#[derive(Clone)]
pub struct FontFace {
    pub name: String,
    pub data: Arc<Vec<u8>>,
    pub font: FontArc,
    pub path: Option<PathBuf>,
    pub builtin: bool,
    /// Loaded by the user (File → Load a font file / workspace), not a system font.
    pub user: bool,
}

#[derive(Clone)]
pub struct FontBook {
    pub faces: Vec<FontFace>,
}

/// (family name, file names) looked up in the system font folders.
const SYSTEM_FONTS: &[(&str, &[&str])] = &[
    ("Arial", &["arial.ttf", "Arial.ttf", "LiberationSans-Regular.ttf"]),
    (
        "Arial Bold",
        &["arialbd.ttf", "Arial Bold.ttf", "LiberationSans-Bold.ttf"],
    ),
    ("Calibri", &["calibri.ttf", "Carlito-Regular.ttf"]),
    ("Calibri Bold", &["calibrib.ttf", "Carlito-Bold.ttf"]),
    ("Segoe UI", &["segoeui.ttf"]),
    ("Verdana", &["verdana.ttf", "Verdana.ttf"]),
    ("Tahoma", &["tahoma.ttf", "Tahoma.ttf"]),
    (
        "Times New Roman",
        &["times.ttf", "Times New Roman.ttf", "LiberationSerif-Regular.ttf"],
    ),
    (
        "Courier New",
        &["cour.ttf", "Courier New.ttf", "LiberationMono-Regular.ttf"],
    ),
    ("Consolas", &["consola.ttf"]),
    ("DejaVu Sans", &["DejaVuSans.ttf"]),
    ("DejaVu Sans Bold", &["DejaVuSans-Bold.ttf"]),
];

fn font_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(windir) = std::env::var("WINDIR") {
        dirs.push(Path::new(&windir).join("Fonts"));
    }
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        dirs.push(Path::new(&local).join("Microsoft").join("Windows").join("Fonts"));
    }
    for d in [
        "C:\\Windows\\Fonts",
        "/Library/Fonts",
        "/System/Library/Fonts/Supplemental",
        "/System/Library/Fonts",
        "/usr/share/fonts/truetype/msttcorefonts",
        "/usr/share/fonts/truetype/liberation",
        "/usr/share/fonts/truetype/liberation2",
        "/usr/share/fonts/truetype/crosextra",
        "/usr/share/fonts/truetype/dejavu",
        "/usr/share/fonts/dejavu",
        "/usr/share/fonts/liberation",
        "/usr/share/fonts/TTF",
    ] {
        dirs.push(PathBuf::from(d));
    }
    if let Ok(home) = std::env::var("HOME") {
        dirs.push(Path::new(&home).join("Library").join("Fonts"));
        dirs.push(Path::new(&home).join(".local").join("share").join("fonts"));
    }
    dirs
}

impl FontBook {
    /// `builtins`: bytes of the proportional and monospace default fonts.
    pub fn new(proportional: &'static [u8], monospace: &'static [u8]) -> FontBook {
        let mut faces = Vec::new();
        for (name, bytes) in [(PROPORTIONAL, proportional), (MONOSPACE, monospace)] {
            if let Ok(font) = FontArc::try_from_slice(bytes) {
                faces.push(FontFace {
                    name: name.to_owned(),
                    data: Arc::new(bytes.to_vec()),
                    font,
                    path: None,
                    builtin: true,
                    user: false,
                });
            }
        }
        FontBook { faces }
    }

    /// Adds the common system fonts that can be found.
    pub fn scan_system(&mut self) {
        let dirs = font_dirs();
        for (name, files) in SYSTEM_FONTS {
            'files: for file in *files {
                for d in &dirs {
                    let p = d.join(file);
                    if p.is_file() && self.add_file_named(&p, name, false).is_ok() {
                        break 'files;
                    }
                }
            }
        }
    }

    /// Loads a font file; returns the family name.
    pub fn add_file(&mut self, path: &Path) -> Result<String, String> {
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Font".to_owned());
        self.add_file_named(path, &stem, true)
    }

    fn add_file_named(&mut self, path: &Path, name: &str, user: bool) -> Result<String, String> {
        if let Some(f) = self.faces.iter_mut().find(|f| f.name == name) {
            f.user |= user;
            return Ok(f.name.clone());
        }
        let bytes = std::fs::read(path).map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
        let font =
            FontArc::try_from_vec(bytes.clone()).map_err(|_| format!("{} is not a valid font", path.display()))?;
        self.faces.push(FontFace {
            name: name.to_owned(),
            data: Arc::new(bytes),
            font,
            path: Some(path.to_path_buf()),
            builtin: false,
            user,
        });
        Ok(name.to_owned())
    }

    pub fn names(&self) -> Vec<String> {
        self.faces.iter().map(|f| f.name.clone()).collect()
    }

    pub fn contains(&self, name: &str) -> bool {
        self.faces.iter().any(|f| f.name == name)
    }

    /// Face for a family name, falling back to the proportional font.
    pub fn get(&self, name: &str) -> &FontFace {
        self.faces
            .iter()
            .find(|f| f.name == name)
            .unwrap_or_else(|| &self.faces[0])
    }
}
