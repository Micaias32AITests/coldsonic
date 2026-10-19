use std::path::PathBuf;

use anyhow::{Context, bail};
use directories::ProjectDirs;

use crate::library::LibraryConfig;

fn project_dirs() -> Option<ProjectDirs> {
    ProjectDirs::from("", "", "coldsonic")
}

/// `$XDG_CONFIG_HOME/coldsonic/libraries.toml` (falls back to
/// `~/.config/coldsonic`).
fn libraries_path() -> Option<PathBuf> {
    project_dirs().map(|dirs| dirs.config_dir().join("libraries.toml"))
}

/// Reads the `[[library]]` entries. File IO only — the config types live in
/// [`crate::library`]. Missing file → empty list (first boot); a malformed
/// entry errors naming it, so hand-edited configs get a loud, specific error.
pub fn load_libraries() -> anyhow::Result<Vec<LibraryConfig>> {
    let Some(path) = libraries_path() else {
        return Ok(Vec::new());
    };
    let Ok(raw) = std::fs::read_to_string(&path) else {
        tracing::debug!(path = %path.display(), "no libraries.toml; starting empty");
        return Ok(Vec::new());
    };
    let value: toml::Value =
        toml::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;
    let Some(entries) = value.get("library").and_then(toml::Value::as_array) else {
        return Ok(Vec::new());
    };
    let mut libraries = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        let name = entry
            .get("name")
            .and_then(toml::Value::as_str)
            .unwrap_or("<unnamed>");
        if entry.get("id").and_then(toml::Value::as_str).is_none() {
            bail!("library entry #{} ('{name}') is missing `id`", index + 1);
        }
        let config: LibraryConfig = entry
            .clone()
            .try_into()
            .with_context(|| format!("library entry #{} ('{name}')", index + 1))?;
        libraries.push(config);
    }
    Ok(libraries)
}

#[derive(serde::Serialize)]
struct LibrariesFile<'a> {
    library: &'a [LibraryConfig],
}

/// Writes the full list back to `libraries.toml` (used by the create form).
pub fn save_libraries(libraries: &[LibraryConfig]) -> anyhow::Result<()> {
    let path =
        libraries_path().ok_or_else(|| anyhow::anyhow!("no config directory on this system"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let raw = toml::to_string_pretty(&LibrariesFile { library: libraries })
        .with_context(|| format!("serializing {}", path.display()))?;
    std::fs::write(&path, raw).with_context(|| format!("writing {}", path.display()))?;
    tracing::info!(path = %path.display(), count = libraries.len(), "libraries saved");
    Ok(())
}

pub fn config_dir() -> Option<PathBuf> {
    project_dirs().map(|dirs| dirs.config_dir().to_path_buf())
}

pub fn cache_dir() -> Option<PathBuf> {
    project_dirs().map(|dirs| dirs.cache_dir().to_path_buf())
}

pub fn data_dir() -> Option<PathBuf> {
    project_dirs().map(|dirs| dirs.data_dir().to_path_buf())
}
