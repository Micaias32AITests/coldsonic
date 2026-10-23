use std::path::PathBuf;

use anyhow::{Context, bail};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

use crate::library::LibraryConfig;

fn project_dirs() -> Option<ProjectDirs> {
    ProjectDirs::from("", "", "coldsonic")
}

/// `$XDG_CONFIG_HOME/coldsonic/libraries.toml` (falls back to
/// `~/.config/coldsonic`).
fn libraries_path() -> Option<PathBuf> {
    project_dirs().map(|dirs| dirs.config_dir().join("libraries.toml"))
}

fn settings_path() -> Option<PathBuf> {
    project_dirs().map(|dirs| dirs.config_dir().join("settings.toml"))
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PanelStartup {
    #[default]
    PersistState,
    StartClosed,
    StartOpened,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PanelScreenConfig {
    #[default]
    Queue,
    NowPlaying,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiSettings {
    #[serde(default)]
    pub panel_startup: PanelStartup,
    #[serde(default = "default_panel_open")]
    pub panel_open: bool,
    #[serde(default)]
    pub panel_screen: PanelScreenConfig,
    #[serde(default = "default_sidebar_ratio")]
    pub sidebar_ratio: f32,
    #[serde(default = "default_panel_ratio")]
    pub panel_ratio: f32,
}

fn default_panel_open() -> bool {
    true
}

fn default_sidebar_ratio() -> f32 {
    0.15
}

fn default_panel_ratio() -> f32 {
    0.20
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            panel_startup: PanelStartup::default(),
            panel_open: default_panel_open(),
            panel_screen: PanelScreenConfig::default(),
            sidebar_ratio: default_sidebar_ratio(),
            panel_ratio: default_panel_ratio(),
        }
    }
}

pub fn load_ui_settings() -> UiSettings {
    let Some(path) = settings_path() else {
        return UiSettings::default();
    };
    let Ok(raw) = std::fs::read_to_string(&path) else {
        tracing::debug!(path = %path.display(), "no settings.toml; using defaults");
        return UiSettings::default();
    };
    match toml::from_str(&raw) {
        Ok(settings) => settings,
        Err(err) => {
            tracing::warn!(path = %path.display(), error = %err, "failed to parse settings.toml; using defaults");
            UiSettings::default()
        }
    }
}

pub fn save_ui_settings(settings: &UiSettings) -> anyhow::Result<()> {
    let path =
        settings_path().ok_or_else(|| anyhow::anyhow!("no config directory on this system"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let raw = toml::to_string_pretty(settings)
        .with_context(|| format!("serializing {}", path.display()))?;
    std::fs::write(&path, raw).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}
