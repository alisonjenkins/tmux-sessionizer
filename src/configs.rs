use clap::ValueEnum;
use error_stack::ResultExt;
use serde_derive::{Deserialize, Serialize};
use std::{collections::HashMap, env, fmt::Display, fs::canonicalize, io::Write, path::PathBuf, time::{SystemTime, UNIX_EPOCH}};

use ratatui::style::{Color, Style, Stylize};

use crate::{error::Suggestion, keymap::Keymap, picker::InputPosition};

type Result<T> = error_stack::Result<T, ConfigError>;

#[derive(Debug)]
pub enum ConfigError {
    NoDefaultSearchPath,
    NoValidSearchPath,
    LoadError,
    TomlError,
    FileWriteError,
    IoError,
}

impl std::error::Error for ConfigError {}

impl Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoDefaultSearchPath => write!(f, "No default search path was found"),
            Self::NoValidSearchPath => write!(f, "No valid search path was found"),
            Self::TomlError => write!(f, "Could not serialize config to TOML"),
            Self::FileWriteError => write!(f, "Could not write to config file"),
            Self::LoadError => write!(f, "Could not load configuration"),
            Self::IoError => write!(f, "IO error"),
        }
    }
}

#[derive(Default, Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct Config {
    pub default_session: Option<String>,
    pub display_full_path: Option<bool>,
    pub search_submodules: Option<bool>,
    pub recursive_submodules: Option<bool>,
    pub switch_filter_unknown: Option<bool>,
    pub session_sort_order: Option<SessionSortOrderConfig>,
    pub excluded_dirs: Option<Vec<String>>,
    pub search_paths: Option<Vec<String>>, // old format, deprecated
    pub search_dirs: Option<Vec<SearchDirectory>>,
    pub sessions: Option<Vec<Session>>,
    pub picker_colors: Option<PickerColorConfig>,
    pub input_position: Option<InputPosition>,
    pub shortcuts: Option<Keymap>,
    pub bookmarks: Option<Vec<String>>,
    pub session_configs: Option<HashMap<String, SessionConfig>>,
    pub marks: Option<HashMap<String, String>>,
    pub clone_repo_switch: Option<CloneRepoSwitchConfig>,
    pub vcs_providers: Option<Vec<VcsProviders>>,
    pub session_frecency: Option<HashMap<String, SessionFrecencyData>>,
    pub github_profiles: Option<Vec<GitHubProfile>>,
    pub picker_switch_mode_key: Option<String>, // default: "tab"
    pub picker_refresh_key: Option<String>, // default: "f5"
    pub github_cache_duration_hours: Option<u64>, // default: 24*30 (1 month)
}

pub const DEFAULT_VCS_PROVIDERS: &[VcsProviders] = &[VcsProviders::Git];

#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum VcsProviders {
    Git,
    #[serde(alias = "jj")]
    Jujutsu,
}

#[derive(Default, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfigExport {
    pub default_session: Option<String>,
    pub display_full_path: bool,
    pub search_submodules: bool,
    pub recursive_submodules: bool,
    pub switch_filter_unknown: bool,
    pub session_sort_order: SessionSortOrderConfig,
    pub excluded_dirs: Vec<String>,
    pub search_dirs: Vec<SearchDirectory>,
    pub sessions: Vec<Session>,
    pub picker_colors: PickerColorConfig,
    pub shortcuts: Keymap,
    pub bookmarks: Vec<String>,
    pub session_configs: HashMap<String, SessionConfig>,
    pub marks: HashMap<String, String>,
    pub clone_repo_switch: CloneRepoSwitchConfig,
    pub vcs_providers: Vec<VcsProviders>,
    pub session_frecency: HashMap<String, SessionFrecencyData>,
    pub github_profiles: Vec<GitHubProfile>,
    pub picker_switch_mode_key: String,
    pub picker_refresh_key: String,
    pub github_cache_duration_hours: u64,
}

impl From<Config> for ConfigExport {
    fn from(value: Config) -> Self {
        Self {
            default_session: value.default_session,
            display_full_path: value.display_full_path.unwrap_or_default(),
            search_submodules: value.search_submodules.unwrap_or_default(),
            recursive_submodules: value.recursive_submodules.unwrap_or_default(),
            switch_filter_unknown: value.switch_filter_unknown.unwrap_or_default(),
            session_sort_order: value.session_sort_order.unwrap_or_default(),
            excluded_dirs: value.excluded_dirs.unwrap_or_default(),
            search_dirs: value.search_dirs.unwrap_or_default(),
            sessions: value.sessions.unwrap_or_default(),
            picker_colors: PickerColorConfig::with_defaults(
                value.picker_colors.unwrap_or_default(),
            ),
            shortcuts: value
                .shortcuts
                .as_ref()
                .map(Keymap::with_defaults)
                .unwrap_or_default(),
            bookmarks: value.bookmarks.unwrap_or_default(),
            session_configs: value.session_configs.unwrap_or_default(),
            marks: value.marks.unwrap_or_default(),
            clone_repo_switch: value.clone_repo_switch.unwrap_or_default(),
            vcs_providers: value.vcs_providers.unwrap_or(DEFAULT_VCS_PROVIDERS.into()),
            session_frecency: value.session_frecency.unwrap_or_default(),
            github_profiles: value.github_profiles.unwrap_or_default(),
            picker_switch_mode_key: value.picker_switch_mode_key.unwrap_or_else(|| "tab".to_string()),
            picker_refresh_key: value.picker_refresh_key.unwrap_or_else(|| "f5".to_string()),
            github_cache_duration_hours: value.github_cache_duration_hours.unwrap_or(24 * 30), // 1 month
        }
    }
}

impl Config {
    pub(crate) fn new() -> Result<Self> {
        let config_builder = match env::var("TMS_CONFIG_FILE") {
            Ok(path) => {
                config::Config::builder().add_source(config::File::with_name(&path).required(false))
            }
            Err(e) => match e {
                env::VarError::NotPresent => {
                    let mut builder = config::Config::builder();
                    let mut config_found = false; // Stores whether a valid config file was found
                    if let Some(home_path) = dirs::home_dir() {
                        config_found = true;
                        let path = home_path.as_path().join(".config/tms/config.toml");
                        builder = builder.add_source(config::File::from(path).required(false));
                    }
                    if let Some(config_path) = dirs::config_dir() {
                        config_found = true;
                        let path = config_path.as_path().join("tms/config.toml");
                        builder = builder.add_source(config::File::from(path).required(false));
                    }
                    if !config_found {
                        return Err(ConfigError::LoadError)
                            .attach_printable("Could not find a valid location for config file (both home and config dirs cannot be found)")
                            .attach(Suggestion("Try specifying a config file with the TMS_CONFIG_FILE environment variable."));
                    }
                    builder
                }
                env::VarError::NotUnicode(_) => {
                    return Err(ConfigError::LoadError).attach_printable(
                        "Invalid non-unicode value for TMS_CONFIG_FILE env variable",
                    );
                }
            },
        };
        let config = config_builder
            .build()
            .change_context(ConfigError::LoadError)
            .attach_printable("Could not parse configuration")?;
        config
            .try_deserialize()
            .change_context(ConfigError::LoadError)
            .attach_printable("Could not deserialize configuration")
    }

    pub fn save(&self) -> Result<()> {
        let toml_pretty = toml::to_string_pretty(self)
            .change_context(ConfigError::TomlError)?
            .into_bytes();
        // The TMS_CONFIG_FILE envvar should be set, either by the user or when the config is
        // loaded. However, there is a possibility it becomes unset between loading and saving
        // the config. In this case, it will fall back to the platform-specific config folder, and
        // if that can't be found then it's good old ~/.config
        let path = match env::var("TMS_CONFIG_FILE") {
            Ok(path) => PathBuf::from(path),
            Err(_) => {
                if let Some(config_path) = dirs::config_dir() {
                    config_path.as_path().join("tms/config.toml")
                } else if let Some(home_path) = dirs::home_dir() {
                    home_path.as_path().join(".config/tms/config.toml")
                } else {
                    return Err(ConfigError::LoadError)
                        .attach_printable("Could not find a valid location to write config file (both home and config dirs cannot be found)")
                        .attach(Suggestion("Try specifying a config file with the TMS_CONFIG_FILE environment variable."));
                }
            }
        };
        let parent = path
            .parent()
            .ok_or(ConfigError::FileWriteError)
            .attach_printable(format!(
                "Unable to determine parent directory of specified tms config file: {}",
                path.to_str()
                    .unwrap_or("(path could not be converted to string)")
            ))?;
        std::fs::create_dir_all(parent)
            .change_context(ConfigError::FileWriteError)
            .attach_printable("Unable to create tms config folder")?;
        let mut file = std::fs::File::create(path).change_context(ConfigError::FileWriteError)?;
        file.write_all(&toml_pretty)
            .change_context(ConfigError::FileWriteError)?;
        Ok(())
    }

    pub fn search_dirs(&self) -> Result<Vec<SearchDirectory>> {
        if self.search_dirs.as_ref().is_none_or(Vec::is_empty)
            && self.search_paths.as_ref().is_none_or(Vec::is_empty)
        {
            return Err(ConfigError::NoDefaultSearchPath)
            .attach_printable(
                "You must configure at least one default search path with the `config` subcommand. E.g `tms config` ",
            );
        }

        let mut search_dirs = if let Some(search_dirs) = self.search_dirs.as_ref() {
            search_dirs
                .iter()
                .filter_map(|search_dir| {
                    let expanded_path = shellexpand::full(&search_dir.path.to_string_lossy())
                        .ok()?
                        .to_string();

                    let path = canonicalize(expanded_path).ok()?;

                    Some(SearchDirectory::new(path, search_dir.depth))
                })
                .collect()
        } else {
            Vec::new()
        };

        // merge old search paths with new search directories
        if let Some(search_paths) = self.search_paths.as_ref() {
            if !search_paths.is_empty() {
                search_dirs.extend(search_paths.iter().filter_map(|path| {
                    let expanded_path = shellexpand::full(&path).ok()?.to_string();
                    let path = canonicalize(expanded_path).ok()?;

                    Some(SearchDirectory::new(path, 10))
                }));
            }
        }

        if search_dirs.is_empty() {
            return Err(ConfigError::NoValidSearchPath)
            .attach_printable(
                "You must configure at least one valid search path with the `config` subcommand. E.g `tms config` "
            );
        }

        // Deduplicate search directories by path to prevent scanning the same directory multiple times
        // Keep the entry with the maximum depth for each unique path
        let mut seen_paths = std::collections::HashMap::new();
        for dir in search_dirs {
            match seen_paths.entry(dir.path.clone()) {
                std::collections::hash_map::Entry::Vacant(entry) => {
                    entry.insert(dir);
                }
                std::collections::hash_map::Entry::Occupied(mut entry) => {
                    // Keep the directory with the greater depth
                    if dir.depth > entry.get().depth {
                        entry.insert(dir);
                    }
                }
            }
        }
        let search_dirs: Vec<_> = seen_paths.into_values().collect();

        Ok(search_dirs)
    }

    pub fn add_bookmark(&mut self, path: String) {
        let bookmarks = &mut self.bookmarks;
        match bookmarks {
            Some(ref mut bookmarks) => {
                if !bookmarks.contains(&path) {
                    bookmarks.push(path);
                }
            }
            None => {
                self.bookmarks = Some(vec![path]);
            }
        }
    }

    pub fn delete_bookmark(&mut self, path: String) {
        if let Some(ref mut bookmarks) = self.bookmarks {
            if let Some(idx) = bookmarks.iter().position(|bookmark| *bookmark == path) {
                bookmarks.remove(idx);
            }
        }
    }

    pub fn bookmark_paths(&self) -> Vec<PathBuf> {
        if let Some(bookmarks) = &self.bookmarks {
            bookmarks
                .iter()
                .filter_map(|b| {
                    if let Ok(expanded) = shellexpand::full(b) {
                        PathBuf::from(expanded.to_string()).canonicalize().ok()
                    } else {
                        None
                    }
                })
                .collect()
        } else {
            Vec::new()
        }
    }

    pub fn add_mark(&mut self, path: String, index: usize) {
        let marks = &mut self.marks;
        match marks {
            Some(ref mut marks) => {
                marks.insert(index.to_string(), path);
            }
            None => {
                self.marks = Some(HashMap::from([(index.to_string(), path)]));
            }
        }
    }

    pub fn delete_mark(&mut self, index: usize) {
        if let Some(ref mut marks) = self.marks {
            marks.remove(&index.to_string());
        }
    }

    pub fn clear_marks(&mut self) {
        self.marks = None;
    }

    pub fn update_session_frecency(&mut self, session_name: &str) {
        let session_frecency = self.session_frecency.get_or_insert_with(HashMap::new);
        
        match session_frecency.get_mut(session_name) {
            Some(data) => data.update_access(),
            None => {
                session_frecency.insert(session_name.to_string(), SessionFrecencyData::new());
            }
        }
    }

    pub fn get_session_frecency_score(&self, session_name: &str) -> f64 {
        self.session_frecency
            .as_ref()
            .and_then(|frecency| frecency.get(session_name))
            .map(|data| data.frecency_score())
            .unwrap_or(0.0)
    }

    pub fn get_github_profiles(&self) -> Vec<GitHubProfile> {
        self.github_profiles.clone().unwrap_or_default()
    }



    pub fn get_picker_switch_mode_key(&self) -> String {
        self.picker_switch_mode_key.clone().unwrap_or_else(|| "tab".to_string())
    }

    pub fn get_picker_refresh_key(&self) -> String {
        self.picker_refresh_key.clone().unwrap_or_else(|| "f5".to_string())
    }

    pub fn get_github_cache_duration_hours(&self) -> u64 {
        self.github_cache_duration_hours.unwrap_or(24 * 30) // 1 month
    }
}

#[derive(Default, Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct SearchDirectory {
    pub path: PathBuf,
    pub depth: usize,
}

impl SearchDirectory {
    pub fn new(path: PathBuf, depth: usize) -> Self {
        SearchDirectory { path, depth }
    }
}

#[derive(Default, Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct Session {
    pub name: Option<String>,
    pub path: Option<String>,
    pub windows: Option<Vec<Window>>,
}

#[derive(Default, Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct Window {
    pub name: Option<String>,
    pub path: Option<String>,
    pub panes: Option<Vec<Pane>>,
    pub command: Option<String>,
}

#[derive(Default, Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct Pane {}

#[derive(Clone, Default, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PickerColorConfig {
    pub highlight_color: Option<Color>,
    pub highlight_text_color: Option<Color>,
    pub border_color: Option<Color>,
    pub info_color: Option<Color>,
    pub prompt_color: Option<Color>,
}

const HIGHLIGHT_COLOR_DEFAULT: Color = Color::LightBlue;
const HIGHLIGHT_TEXT_COLOR_DEFAULT: Color = Color::Black;
const BORDER_COLOR_DEFAULT: Color = Color::DarkGray;
const INFO_COLOR_DEFAULT: Color = Color::LightYellow;
const PROMPT_COLOR_DEFAULT: Color = Color::LightGreen;

impl PickerColorConfig {
    pub fn default_colors() -> Self {
        PickerColorConfig {
            highlight_color: Some(HIGHLIGHT_COLOR_DEFAULT),
            highlight_text_color: Some(HIGHLIGHT_TEXT_COLOR_DEFAULT),
            border_color: Some(BORDER_COLOR_DEFAULT),
            info_color: Some(INFO_COLOR_DEFAULT),
            prompt_color: Some(PROMPT_COLOR_DEFAULT),
        }
    }

    pub fn with_defaults(self) -> Self {
        PickerColorConfig {
            highlight_color: self.highlight_color.or(Some(HIGHLIGHT_COLOR_DEFAULT)),
            highlight_text_color: self
                .highlight_text_color
                .or(Some(HIGHLIGHT_TEXT_COLOR_DEFAULT)),
            border_color: self.border_color.or(Some(BORDER_COLOR_DEFAULT)),
            info_color: self.info_color.or(Some(INFO_COLOR_DEFAULT)),
            prompt_color: self.prompt_color.or(Some(PROMPT_COLOR_DEFAULT)),
        }
    }

    pub fn highlight_style(&self) -> Style {
        let mut style = Style::default()
            .bg(HIGHLIGHT_COLOR_DEFAULT)
            .fg(HIGHLIGHT_TEXT_COLOR_DEFAULT)
            .bold();

        if let Some(color) = self.highlight_color {
            style = style.bg(color);
        }

        if let Some(color) = self.highlight_text_color {
            style = style.fg(color);
        }

        style
    }

    pub fn border_color(&self) -> Color {
        if let Some(color) = self.border_color {
            color
        } else {
            BORDER_COLOR_DEFAULT
        }
    }

    pub fn info_color(&self) -> Color {
        if let Some(color) = self.info_color {
            color
        } else {
            INFO_COLOR_DEFAULT
        }
    }

    pub fn prompt_color(&self) -> Color {
        if let Some(color) = self.prompt_color {
            color
        } else {
            PROMPT_COLOR_DEFAULT
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Default, Clone, PartialEq, Eq)]
pub enum SessionSortOrderConfig {
    #[default]
    Alphabetical,
    LastAttached,
    Frecency,
}

impl ValueEnum for SessionSortOrderConfig {
    fn value_variants<'a>() -> &'a [Self] {
        &[Self::Alphabetical, Self::LastAttached, Self::Frecency]
    }

    fn to_possible_value(&self) -> Option<clap::builder::PossibleValue> {
        match self {
            SessionSortOrderConfig::Alphabetical => {
                Some(clap::builder::PossibleValue::new("Alphabetical"))
            }
            SessionSortOrderConfig::LastAttached => {
                Some(clap::builder::PossibleValue::new("LastAttached"))
            }
            SessionSortOrderConfig::Frecency => {
                Some(clap::builder::PossibleValue::new("Frecency"))
            }
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize, Copy, Clone, PartialEq, Eq)]
pub enum CloneRepoSwitchConfig {
    #[default]
    Always,
    Never,
    Foreground,
}

impl ValueEnum for CloneRepoSwitchConfig {
    fn value_variants<'a>() -> &'a [Self] {
        &[Self::Always, Self::Never, Self::Foreground]
    }

    fn to_possible_value(&self) -> Option<clap::builder::PossibleValue> {
        match self {
            CloneRepoSwitchConfig::Always => Some(clap::builder::PossibleValue::new("Always")),
            CloneRepoSwitchConfig::Never => Some(clap::builder::PossibleValue::new("Never")),
            CloneRepoSwitchConfig::Foreground => {
                Some(clap::builder::PossibleValue::new("Foreground"))
            }
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct SessionFrecencyData {
    pub access_count: u32,
    pub last_accessed: u64, // Unix timestamp
    pub first_accessed: u64, // Unix timestamp
}

impl SessionFrecencyData {
    pub fn new() -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        
        Self {
            access_count: 1,
            last_accessed: now,
            first_accessed: now,
        }
    }

    pub fn update_access(&mut self) {
        self.access_count += 1;
        self.last_accessed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
    }

    /// Calculate frecency score - higher scores mean more frequent and recent access
    /// This uses a simple algorithm: frequency * recency_factor
    /// where recency_factor decays based on time since last access
    pub fn frecency_score(&self) -> f64 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let time_since_last_access = (now - self.last_accessed) as f64;
        
        // Recency decay - more recent access gets higher score
        // Using an exponential decay with a half-life of about 1 week (604800 seconds)
        let recency_factor = if time_since_last_access > 0.0 {
            (-time_since_last_access / 604800.0).exp()
        } else {
            1.0
        };

        // Combine frequency and recency
        (self.access_count as f64) * recency_factor
    }
}

impl Default for SessionFrecencyData {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_frecency_data_creation() {
        let data = SessionFrecencyData::new();
        assert_eq!(data.access_count, 1);
        assert!(data.last_accessed > 0);
        assert_eq!(data.first_accessed, data.last_accessed);
    }

    #[test]
    fn test_frecency_data_update() {
        let mut data = SessionFrecencyData::new();
        let initial_count = data.access_count;
        let initial_last_accessed = data.last_accessed;
        
        // Small sleep to ensure timestamp changes
        thread::sleep(Duration::from_millis(1));
        
        data.update_access();
        
        assert_eq!(data.access_count, initial_count + 1);
        assert!(data.last_accessed >= initial_last_accessed);
    }

    #[test]
    fn test_frecency_score_calculation() {
        let mut data = SessionFrecencyData::new();
        let initial_score = data.frecency_score();
        
        // More frequent access should increase score
        data.update_access();
        data.update_access();
        let higher_score = data.frecency_score();
        
        assert!(higher_score > initial_score, 
                "Higher frequency should result in higher score: {} vs {}", 
                higher_score, initial_score);
    }

    #[test]
    fn test_config_frecency_methods() {
        let mut config = Config::default();
        
        // Test updating session frecency
        config.update_session_frecency("test_session");
        
        let score = config.get_session_frecency_score("test_session");
        assert!(score > 0.0, "Session should have positive frecency score after access");
        
        // Test multiple updates increase score
        config.update_session_frecency("test_session");
        let higher_score = config.get_session_frecency_score("test_session");
        
        assert!(higher_score > score, 
                "Multiple accesses should increase frecency score: {} vs {}", 
                higher_score, score);
        
        // Test unknown session has zero score
        let unknown_score = config.get_session_frecency_score("unknown_session");
        assert_eq!(unknown_score, 0.0, "Unknown session should have zero frecency score");
    }

    #[test]
    fn test_search_dirs_deduplication() {
        use tempfile::TempDir;
        
        // Create temporary directories for testing
        let temp_dir = TempDir::new().unwrap();
        let test_path = temp_dir.path().join("test");
        let other_path = temp_dir.path().join("other");
        
        std::fs::create_dir_all(&test_path).unwrap();
        std::fs::create_dir_all(&other_path).unwrap();
        
        // Create a config with duplicate search directories
        let mut config = Config::default();
        config.search_dirs = Some(vec![
            SearchDirectory::new(test_path.clone(), 5),
            SearchDirectory::new(test_path.clone(), 10), // Same path, different depth
            SearchDirectory::new(other_path.clone(), 5),
            SearchDirectory::new(test_path.clone(), 3),  // Same path again, lower depth
        ]);
        
        let search_dirs = config.search_dirs().unwrap();
        
        // Should have only 2 unique paths
        assert_eq!(search_dirs.len(), 2, "Should deduplicate paths and have only 2 unique directories");
        
        // Find the test entry
        let test_dir = search_dirs.iter().find(|d| d.path == test_path).unwrap();
        // Should keep the highest depth (10)
        assert_eq!(test_dir.depth, 10, "Should keep the directory entry with the highest depth");
        
        // Find the other entry
        let other_dir = search_dirs.iter().find(|d| d.path == other_path).unwrap();
        assert_eq!(other_dir.depth, 5, "Other directory should have original depth");
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct SessionConfig {
    pub create_script: Option<PathBuf>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct GitHubProfile {
    pub name: String,
    pub credentials_command: String,
    pub clone_root_path: String,
    pub clone_method: Option<GitHubCloneMethod>, // defaults to SSH
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub enum GitHubCloneMethod {
    SSH,
    HTTPS,
}

impl Default for GitHubCloneMethod {
    fn default() -> Self {
        GitHubCloneMethod::SSH
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct GitHubRepoCache {
    pub profile_name: String,
    pub repositories: Vec<GitHubRepo>,
    pub cached_at: u64, // Unix timestamp
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct GitHubRepo {
    pub name: String,
    pub full_name: String,
    pub clone_url_ssh: String,
    pub clone_url_https: String,
    pub description: Option<String>,
    pub updated_at: String,
}
