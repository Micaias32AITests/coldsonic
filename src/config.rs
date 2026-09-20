use std::path::PathBuf;

use directories::ProjectDirs;

#[derive(Debug, Clone, serde::Deserialize)]
pub struct Credentials {
    pub url: String,
    pub username: String,
    pub password: String,
}

fn project_dirs() -> Option<ProjectDirs> {
    ProjectDirs::from("", "", "coldsonic")
}

impl Credentials {
    /// `$XDG_CONFIG_HOME/coldsonic/credentials.toml` (falls back to `~/.config/coldsonic`).
    pub fn load() -> Option<Self> {
        let path = project_dirs()?.config_dir().join("credentials.toml");
        toml::from_str(&std::fs::read_to_string(path).ok()?).ok()
    }
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