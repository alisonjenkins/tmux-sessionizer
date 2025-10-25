use std::collections::BTreeMap;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use error_stack::ResultExt;

use crate::{
    configs::{Config, LocalRepoCache, LocalCachedSession, LocalSessionType},
    error::TmsError,
    perf_json,
    repos::RepoProvider,
    session::{Session, SessionType},
    state::StateManager,
    Result,
};

pub struct LocalCacheManager {
    state_manager: StateManager,
}

impl LocalCacheManager {
    pub fn new() -> Result<Self> {
        let state_manager = StateManager::new()?;
        Ok(LocalCacheManager { state_manager })
    }

    pub fn with_state_manager(state_manager: StateManager) -> Self {
        LocalCacheManager { state_manager }
    }

    /// Get local sessions, using cache if valid or scanning if needed
    pub async fn get_local_sessions(&self, config: &Config, force_refresh: bool) -> Result<BTreeMap<String, Session>> {
        let cache_file = self.state_manager.get_local_cache_file_path();
        
        // Try to load from cache first if not forcing refresh
        if !force_refresh {
            if let Ok(cached_sessions) = self.load_cached_sessions(&cache_file, config).await {
                if self.is_cache_config_valid(&cached_sessions, config) {
                    return Ok(self.convert_cached_to_sessions(cached_sessions));
                }
            }
        }

        // Cache is invalid or we're forcing refresh - scan fresh
        let sessions = self.scan_fresh_sessions(config).await?;
        
        // Cache the results
        self.cache_sessions(&cache_file, config, &sessions).await?;
        
        Ok(sessions)
    }

    async fn load_cached_sessions(&self, cache_file: &Path, config: &Config) -> Result<LocalRepoCache> {
        let cache: LocalRepoCache = perf_json::from_file(cache_file).await
            .change_context(TmsError::IoError)?;
            
        // Check if cache is still valid using configurable duration
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        
        let cache_duration_seconds = config.get_local_cache_duration_hours() * 3600;
        
        if now - cache.cached_at > cache_duration_seconds {
            return Err(TmsError::IoError.into()); // Cache expired
        }
        
        Ok(cache)
    }

    /// Check if the cache configuration matches current configuration
    fn is_cache_config_valid(&self, cached: &LocalRepoCache, current_config: &Config) -> bool {
        // Compare search directories
        if let Ok(current_search_dirs) = current_config.search_dirs() {
            if cached.search_dirs != current_search_dirs {
                return false;
            }
        }
        
        // Compare bookmarks
        let current_bookmarks = current_config.bookmarks.clone().unwrap_or_default();
        if cached.bookmarks != current_bookmarks {
            return false;
        }
        
        true
    }

    async fn scan_fresh_sessions(&self, config: &Config) -> Result<BTreeMap<String, Session>> {
        // Use existing repo finding logic
        let repos = crate::repos::find_repos(config).await?;
        let mut sessions = BTreeMap::new();

        // Convert repo results to sessions
        for (name, repo_list) in repos {
            if let Some(repo) = repo_list.into_iter().next() {
                sessions.insert(name, repo);
            }
        }

        // Add bookmarks
        let bookmarks = config.bookmark_paths();
        for bookmark_path in bookmarks {
            let bookmark_name = bookmark_path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("unknown")
                .to_string();
                
            let visible_name = if config.display_full_path == Some(true) {
                bookmark_path.display().to_string()
            } else {
                bookmark_name.clone()
            };
            
            let bookmark_session = Session::new(bookmark_name, SessionType::Bookmark(bookmark_path));
            sessions.insert(visible_name, bookmark_session);
        }

        Ok(sessions)
    }

    async fn cache_sessions(&self, cache_file: &Path, config: &Config, sessions: &BTreeMap<String, Session>) -> Result<()> {
        let search_dirs = config.search_dirs().change_context(TmsError::ConfigError)?;
        let bookmarks = config.bookmarks.clone().unwrap_or_default();
        
        let cached_sessions: Vec<LocalCachedSession> = sessions.iter()
            .map(|(name, session)| {
                let session_type = match &session.session_type {
                    SessionType::Git(repo) => {
                        match repo.as_ref() {
                            RepoProvider::Git(_) => LocalSessionType::Git,
                            RepoProvider::Jujutsu(_) => LocalSessionType::Jujutsu,
                        }
                    }
                    SessionType::Bookmark(_) => LocalSessionType::Bookmark,
                    SessionType::GitHub { .. } => LocalSessionType::Git, // Shouldn't happen in local cache
                };
                
                LocalCachedSession {
                    name: name.clone(),
                    path: session.path().display().to_string(),
                    session_type,
                }
            })
            .collect();

        let cache = LocalRepoCache {
            search_dirs,
            sessions: cached_sessions,
            bookmarks,
            cached_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        };

        perf_json::to_file(cache_file, &cache).await
            .change_context(TmsError::IoError)?;

        Ok(())
    }

    fn convert_cached_to_sessions(&self, cached: LocalRepoCache) -> BTreeMap<String, Session> {
        let mut sessions = BTreeMap::new();
        
        for cached_session in cached.sessions {
            let session_type = match cached_session.session_type {
                LocalSessionType::Bookmark => {
                    SessionType::Bookmark(cached_session.path.into())
                }
                LocalSessionType::Git | LocalSessionType::Jujutsu => {
                    // For cached git/jj repos, we need to re-open them to get the RepoProvider
                    // This is a lightweight operation compared to directory scanning
                    let path = Path::new(&cached_session.path);
                    match crate::repos::RepoProvider::open(path, &Default::default()) {
                        Ok(repo) => SessionType::Git(Box::new(repo)),
                        Err(_) => {
                            // If we can't open the repo, skip it (might have been deleted)
                            continue;
                        }
                    }
                }
            };
            
            let session = Session::new(
                cached_session.name.clone(),
                session_type,
            );
            sessions.insert(cached_session.name, session);
        }
        
        sessions
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::configs::SearchDirectory;
    use tempfile::TempDir;
    use std::fs;

    #[tokio::test]
    async fn test_local_cache_manager_creation() {
        let temp_dir = TempDir::new().unwrap();
        let state_path = temp_dir.path().join("state");
        let cache_path = temp_dir.path().join("cache");
        
        let state_manager = StateManager::with_dirs(state_path, cache_path).unwrap();
        let cache_manager = LocalCacheManager::with_state_manager(state_manager);
        
        // Just ensure it can be created and has the expected path structure
        assert!(cache_manager.state_manager.get_local_cache_file_path().to_string_lossy().contains("local"));
    }

    #[tokio::test]
    async fn test_cache_config_validation() {
        let temp_dir = TempDir::new().unwrap();
        let test_path = temp_dir.path().join("test");
        fs::create_dir_all(&test_path).unwrap();
        
        let mut config1 = Config::default();
        config1.search_dirs = Some(vec![SearchDirectory::new(test_path.clone(), 5)]);
        config1.bookmarks = Some(vec!["bookmark1".to_string()]);
        
        let mut config2 = Config::default();
        config2.search_dirs = Some(vec![SearchDirectory::new(test_path, 5)]);
        config2.bookmarks = Some(vec!["bookmark2".to_string()]); // Different bookmark
        
        // Create cache manager with temporary directories
        let state_path = temp_dir.path().join("state");
        let cache_path = temp_dir.path().join("cache");
        let state_manager = StateManager::with_dirs(state_path, cache_path).unwrap();
        let cache_manager = LocalCacheManager::with_state_manager(state_manager);
        
        let cache = LocalRepoCache {
            search_dirs: config1.search_dirs().unwrap(),
            sessions: vec![],
            bookmarks: config1.bookmarks.clone().unwrap(),
            cached_at: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
        };
        
        // Should be valid for config1
        assert!(cache_manager.is_cache_config_valid(&cache, &config1));
        
        // Should be invalid for config2 (different bookmarks)
        assert!(!cache_manager.is_cache_config_valid(&cache, &config2));
    }
}