use std::path::PathBuf;

use error_stack::ResultExt;
use serde_derive::{Deserialize, Serialize};

use crate::{error::TmsError, Result};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AppState {
    pub active_profile: Option<String>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            active_profile: Some("local".to_string()),
        }
    }
}

pub struct StateManager {
    state_dir: PathBuf,
    cache_dir: PathBuf,
}

impl StateManager {
    pub fn new() -> Result<Self> {
        let state_dir = get_xdg_state_home()?.join("tms");
        let cache_dir = get_xdg_cache_home()?.join("tms");
        Self::with_dirs(state_dir, cache_dir)
    }

    pub fn with_dirs(state_dir: PathBuf, cache_dir: PathBuf) -> Result<Self> {
        // Create directories if they don't exist
        std::fs::create_dir_all(&state_dir)
            .change_context(TmsError::IoError)?;
        std::fs::create_dir_all(&cache_dir.join("github"))
            .change_context(TmsError::IoError)?;

        Ok(StateManager {
            state_dir,
            cache_dir,
        })
    }

    pub fn load_state(&self) -> Result<AppState> {
        let state_file = self.state_dir.join("state.json");
        
        if !state_file.exists() {
            return Ok(AppState::default());
        }

        let content = std::fs::read_to_string(&state_file)
            .change_context(TmsError::IoError)?;
            
        let state: AppState = serde_json::from_str(&content)
            .change_context(TmsError::IoError)?;
            
        Ok(state)
    }

    pub fn save_state(&self, state: &AppState) -> Result<()> {
        let state_file = self.state_dir.join("state.json");
        
        let content = serde_json::to_string_pretty(state)
            .change_context(TmsError::IoError)?;
            
        std::fs::write(state_file, content)
            .change_context(TmsError::IoError)?;
            
        Ok(())
    }

    pub fn get_active_profile(&self) -> Result<Option<String>> {
        let state = self.load_state()?;
        Ok(state.active_profile)
    }

    pub fn set_active_profile(&self, profile_name: Option<String>) -> Result<()> {
        let mut state = self.load_state()?;
        state.active_profile = profile_name;
        self.save_state(&state)?;
        Ok(())
    }

    pub fn get_github_cache_dir(&self) -> PathBuf {
        self.cache_dir.join("github")
    }

    pub fn get_cache_file_path(&self, profile_name: &str) -> PathBuf {
        self.get_github_cache_dir().join(format!("{}.json", profile_name))
    }
}

fn get_xdg_state_home() -> Result<PathBuf> {
    if let Ok(path) = std::env::var("XDG_STATE_HOME") {
        Ok(PathBuf::from(path))
    } else if let Some(home) = dirs::home_dir() {
        Ok(home.join(".local/state"))
    } else {
        Err(TmsError::IoError.into())
    }
}

fn get_xdg_cache_home() -> Result<PathBuf> {
    if let Ok(path) = std::env::var("XDG_CACHE_HOME") {
        Ok(PathBuf::from(path))
    } else if let Some(home) = dirs::home_dir() {
        Ok(home.join(".cache"))
    } else {
        Err(TmsError::IoError.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use std::env;

    #[test]
    fn test_state_manager_creation() {
        let temp_dir = TempDir::new().unwrap();
        
        let state_path = temp_dir.path().join("state");
        let cache_path = temp_dir.path().join("cache");
        
        let state_manager = StateManager::with_dirs(state_path.clone(), cache_path.clone()).unwrap();
        
        // Check that directories were created
        assert!(state_manager.state_dir.exists());
        assert!(state_manager.cache_dir.join("github").exists());
        assert_eq!(state_manager.state_dir, state_path);
        assert_eq!(state_manager.cache_dir, cache_path);
    }

    #[test]
    fn test_state_persistence() {
        let temp_dir = TempDir::new().unwrap();
        
        let state_path = temp_dir.path().join("state");
        let cache_path = temp_dir.path().join("cache");
        
        let state_manager = StateManager::with_dirs(state_path.clone(), cache_path.clone()).unwrap();
        
        // Test default state
        let initial_state = state_manager.load_state().unwrap();
        assert_eq!(initial_state.active_profile, Some("local".to_string()));
        
        // Test setting and getting active profile
        state_manager.set_active_profile(Some("work".to_string())).unwrap();
        let active_profile = state_manager.get_active_profile().unwrap();
        assert_eq!(active_profile, Some("work".to_string()));
        
        // Test persistence across manager instances
        let new_state_manager = StateManager::with_dirs(state_path, cache_path).unwrap();
        let persisted_profile = new_state_manager.get_active_profile().unwrap();
        assert_eq!(persisted_profile, Some("work".to_string()));
    }

    #[test]
    fn test_xdg_fallbacks() {
        // Remove XDG variables to test fallback
        let original_state = env::var("XDG_STATE_HOME").ok();
        let original_cache = env::var("XDG_CACHE_HOME").ok();
        
        env::remove_var("XDG_STATE_HOME");
        env::remove_var("XDG_CACHE_HOME");
        
        let state_home = get_xdg_state_home().unwrap();
        let cache_home = get_xdg_cache_home().unwrap();
        
        // Should use ~/.local/state and ~/.cache as fallbacks
        assert!(state_home.to_string_lossy().ends_with("/.local/state"));
        assert!(cache_home.to_string_lossy().ends_with("/.cache"));
        
        // Restore original values
        if let Some(val) = original_state {
            env::set_var("XDG_STATE_HOME", val);
        }
        if let Some(val) = original_cache {
            env::set_var("XDG_CACHE_HOME", val);
        }
    }
}