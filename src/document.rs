use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use resvg::usvg;

/// Parsed SVG kept in memory so resizes only re-rasterize.
pub struct Document {
    pub path: PathBuf,
    pub tree: usvg::Tree,
}

#[derive(Debug)]
pub enum LoadError {
    Io { path: PathBuf, source: io::Error },
    Parse { path: PathBuf, message: String },
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => {
                write!(f, "failed to read {}: {source}", path.display())
            }
            Self::Parse { path, message } => {
                write!(f, "failed to parse {}: {message}", path.display())
            }
        }
    }
}

impl std::error::Error for LoadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Parse { .. } => None,
        }
    }
}

pub fn load(path: &Path) -> Result<Document, LoadError> {
    let bytes = fs::read(path).map_err(|source| LoadError::Io {
        path: path.to_path_buf(),
        source,
    })?;

    let mut options = usvg::Options {
        resources_dir: path
            .canonicalize()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .or_else(|| path.parent().map(Path::to_path_buf)),
        ..usvg::Options::default()
    };

    // System font enumeration is the slowest part of SVG startup. Skip it
    // unless the file actually contains text.
    if likely_has_text(&bytes) {
        options.fontdb = system_fonts();
    }

    let tree = usvg::Tree::from_data(&bytes, &options).map_err(|err| LoadError::Parse {
        path: path.to_path_buf(),
        message: err.to_string(),
    })?;

    Ok(Document {
        path: path.to_path_buf(),
        tree,
    })
}

pub fn title_for(path: Option<&Path>, error: Option<&str>) -> String {
    if let Some(error) = error {
        return format!("savage — {error}");
    }
    match path.and_then(|p| p.file_name()).and_then(|n| n.to_str()) {
        Some(name) => format!("{name} — savage"),
        None => "savage".into(),
    }
}

fn system_fonts() -> Arc<usvg::fontdb::Database> {
    static FONTS: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    FONTS
        .get_or_init(|| {
            let mut db = usvg::fontdb::Database::new();
            db.load_system_fonts();
            Arc::new(db)
        })
        .clone()
}

pub fn likely_has_text(bytes: &[u8]) -> bool {
    contains_ignore_ascii_case(bytes, b"<text") || contains_ignore_ascii_case(bytes, b"<tspan")
}

fn contains_ignore_ascii_case(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_sample_svg() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/sample.svg");
        let doc = load(&path).expect("sample.svg should parse");
        assert!(doc.tree.size().width() > 0.0);
        assert!(doc.tree.size().height() > 0.0);
        assert!(!doc.tree.has_text_nodes());
    }

    #[test]
    fn text_heuristic() {
        assert!(likely_has_text(b"<svg><text x='0'>hi</text></svg>"));
        assert!(likely_has_text(b"<svg><TEXT>hi</TEXT></svg>"));
        assert!(!likely_has_text(b"<svg><circle r='1'/></svg>"));
    }
}
