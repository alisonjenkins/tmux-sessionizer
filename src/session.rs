use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use error_stack::ResultExt;
use tokio::sync::mpsc;

use crate::{
    configs::{Config, SessionSortOrderConfig},
    dirty_paths::DirtyUtf8Path,
    error::TmsError,
    repos::{find_repos, find_repos_streaming, find_submodules, RepoProvider},
    tmux::Tmux,
    Result,
};

pub struct Session {
    pub name: String,
    pub session_type: SessionType,
}

pub enum SessionType {
    Git(Box<RepoProvider>),
    Bookmark(PathBuf),
}

impl Session {
    pub fn new(name: String, session_type: SessionType) -> Self {
        Session { name, session_type }
    }

    pub fn path(&self) -> &Path {
        match &self.session_type {
            SessionType::Git(repo) if repo.is_bare() => repo.path(),
            SessionType::Git(repo) => repo.path().parent().unwrap(),
            SessionType::Bookmark(path) => path,
        }
    }

    pub fn switch_to(&self, tmux: &Tmux, config: &Config) -> Result<()> {
        match &self.session_type {
            SessionType::Git(repo) => self.switch_to_repo_session(repo, tmux, config),
            SessionType::Bookmark(path) => self.switch_to_bookmark_session(tmux, path, config),
        }
    }

    fn switch_to_repo_session(
        &self,
        repo: &RepoProvider,
        tmux: &Tmux,
        config: &Config,
    ) -> Result<()> {
        let path = if repo.is_bare() {
            repo.path().to_path_buf().to_string()?
        } else {
            repo.work_dir()
                .expect("bare repositories should all have parent directories")
                .canonicalize()
                .change_context(TmsError::IoError)?
                .to_string()?
        };
        let session_name = self.name.replace('.', "_");

        if !tmux.session_exists(&session_name) {
            tmux.new_session(Some(&session_name), Some(&path));
            tmux.set_up_tmux_env(repo, &session_name, config)?;
            tmux.run_session_create_script(self.path(), &session_name, config)?;
        }

        tmux.switch_to_session(&session_name);

        Ok(())
    }

    fn switch_to_bookmark_session(&self, tmux: &Tmux, path: &Path, config: &Config) -> Result<()> {
        let session_name = self.name.replace('.', "_");

        if !tmux.session_exists(&session_name) {
            tmux.new_session(Some(&session_name), path.to_str());
            tmux.run_session_create_script(path, &session_name, config)?;
        }

        tmux.switch_to_session(&session_name);

        Ok(())
    }
}

pub trait SessionContainer {
    fn find_session(&self, name: &str) -> Option<&Session>;
    fn insert_session(&mut self, name: String, repo: Session);
    fn list(&self) -> Vec<String>;
    fn list_sorted(&self, config: &Config) -> Vec<String>;
}

impl SessionContainer for BTreeMap<String, Session> {
    fn find_session(&self, name: &str) -> Option<&Session> {
        self.get(name)
    }

    fn insert_session(&mut self, name: String, session: Session) {
        self.insert(name, session);
    }

    fn list(&self) -> Vec<String> {
        // BTreeMap keys are already sorted, so we can just collect them
        self.keys().map(|s| s.to_owned()).collect()
    }

    fn list_sorted(&self, config: &Config) -> Vec<String> {
        match config.session_sort_order.as_ref().unwrap_or(&SessionSortOrderConfig::Alphabetical) {
            SessionSortOrderConfig::Alphabetical => self.list(),
            SessionSortOrderConfig::LastAttached => {
                // For repository sessions, we don't have tmux last_attached info here,
                // so fall back to alphabetical for now. This is used mainly for tmux sessions.
                self.list()
            }
            SessionSortOrderConfig::Frecency => {
                let mut sessions: Vec<_> = self.keys().map(|s| s.to_owned()).collect();
                sessions.sort_by(|a, b| {
                    let score_a = config.get_session_frecency_score(a);
                    let score_b = config.get_session_frecency_score(b);
                    score_b.partial_cmp(&score_a).unwrap_or(std::cmp::Ordering::Equal)
                });
                sessions
            }
        }
    }
}

pub fn create_sessions(config: &Config) -> Result<impl SessionContainer> {
    let mut sessions = find_repos(config)?;
    sessions = append_bookmarks(config, sessions)?;

    let sessions = generate_session_container(sessions, config)?;

    Ok(sessions)
}

/// Create a streaming session channel that yields sessions as repositories are found
/// Returns a tuple of (display_names_receiver, session_container)
/// The session_container will be populated as sessions are found
/// If frecency sorting is enabled, this will collect all sessions first, sort them, then stream them
pub async fn create_sessions_streaming(config: &Config) -> Result<(mpsc::UnboundedReceiver<String>, std::sync::Arc<std::sync::Mutex<BTreeMap<String, Session>>>)> {
    let (tx, rx) = mpsc::unbounded_channel();
    let (session_tx, session_rx) = mpsc::unbounded_channel();
    
    // Create a shared session container to collect sessions as they're found
    let sessions_map = std::sync::Arc::new(std::sync::Mutex::new(BTreeMap::<String, Session>::new()));
    let sessions_map_clone = sessions_map.clone();
    
    let config_clone = config.clone();
    
    // Start background repository scanning
    tokio::spawn(async move {
        if let Err(e) = find_repos_streaming(&config_clone, session_tx).await {
            // Only log streaming errors when explicitly requested (defaults to suppressed)
            if std::env::var("TMS_TRACE").unwrap_or_default() == "1" 
                || std::env::var("TMS_DEBUG").unwrap_or_default() == "1" 
                || std::env::var("TMS_NON_INTERACTIVE").unwrap_or_default() == "1" {
                eprintln!("[TRACE] Error in streaming repo scan: {}", e);
            }
        }
    });

    // Check if we need frecency sorting
    let use_frecency = matches!(config.session_sort_order.as_ref(), Some(SessionSortOrderConfig::Frecency));
    
    if use_frecency {
        // For frecency sorting, collect all sessions first, then sort and stream them
        let config_clone = config.clone();
        let sessions_map_clone2 = sessions_map_clone.clone();
        tokio::spawn(async move {
            let mut all_sessions = Vec::new();
            
            // First, send bookmarks
            let bookmarks = config_clone.bookmark_paths();
            for bookmark_path in bookmarks {
                let bookmark_name = bookmark_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("unknown")
                    .to_string();
                    
                let visible_name = if config_clone.display_full_path == Some(true) {
                    bookmark_path.display().to_string()
                } else {
                    bookmark_name.clone()
                };
                
                let bookmark_session = Session::new(bookmark_name, SessionType::Bookmark(bookmark_path));
                all_sessions.push((visible_name, bookmark_session));
            }
            
            // Collect streaming sessions
            let mut session_rx = session_rx;
            while let Some(session) = session_rx.recv().await {
                let visible_name = if config_clone.display_full_path == Some(true) {
                    session.path().display().to_string()
                } else {
                    session.name.clone()
                };
                
                all_sessions.push((visible_name, session));
            }
            
            // Sort by frecency score
            all_sessions.sort_by(|(name_a, _), (name_b, _)| {
                let score_a = config_clone.get_session_frecency_score(name_a);
                let score_b = config_clone.get_session_frecency_score(name_b);
                score_b.partial_cmp(&score_a).unwrap_or(std::cmp::Ordering::Equal)
            });
            
            // Now stream the sorted sessions
            for (visible_name, session) in all_sessions {
                if let Ok(mut map) = sessions_map_clone2.lock() {
                    map.insert(visible_name.clone(), session);
                }
                
                if tx.send(visible_name).is_err() {
                    break; // Receiver was dropped
                }
            }
        });
    } else {
        // For non-frecency sorting, use original streaming approach
        // Process bookmarks first (they're instantly available)
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
            
            // Store the bookmark session in our map
            let bookmark_session = Session::new(bookmark_name, SessionType::Bookmark(bookmark_path));
            if let Ok(mut map) = sessions_map_clone.lock() {
                map.insert(visible_name.clone(), bookmark_session);
            }
            
            if tx.send(visible_name).is_err() {
                break; // Receiver was dropped
            }
        }
        
        let config_clone = config.clone();
        let sessions_map_clone2 = sessions_map_clone.clone();
        // Process streaming repository sessions
        tokio::spawn(async move {
            let mut session_rx = session_rx;
            while let Some(session) = session_rx.recv().await {
                let visible_name = if config_clone.display_full_path == Some(true) {
                    session.path().display().to_string()
                } else {
                    session.name.clone()
                };
                
                // Store the session in our map
                if let Ok(mut map) = sessions_map_clone2.lock() {
                    map.insert(visible_name.clone(), session);
                }
                
                if tx.send(visible_name).is_err() {
                    break; // Receiver was dropped
                }
            }
        });
    }
    
    Ok((rx, sessions_map))
}

fn generate_session_container(
    mut sessions: BTreeMap<String, Vec<Session>>,
    config: &Config,
) -> Result<impl SessionContainer> {
    let mut ret = BTreeMap::new();

    for list in sessions.values_mut() {
        if list.len() == 1 {
            let session = list.pop().unwrap();
            insert_session(&mut ret, session, config)?;
        } else {
            let deduplicated = deduplicate_sessions(list);

            for session in deduplicated {
                insert_session(&mut ret, session, config)?;
            }
        }
    }

    Ok(ret)
}

fn insert_session(
    sessions: &mut impl SessionContainer,
    session: Session,
    config: &Config,
) -> Result<()> {
    let visible_name = if config.display_full_path == Some(true) {
        session.path().display().to_string()
    } else {
        session.name.clone()
    };
    if let SessionType::Git(repo) = &session.session_type {
        if config.search_submodules == Some(true) {
            if let Ok(Some(submodules)) = repo.submodules() {
                find_submodules(submodules, &visible_name, sessions, config)?;
            }
        }
    }
    sessions.insert_session(visible_name, session);
    Ok(())
}

fn deduplicate_sessions(duplicate_sessions: &mut Vec<Session>) -> Vec<Session> {
    let mut depth = 1;
    let mut deduplicated = Vec::new();
    while let Some(current_session) = duplicate_sessions.pop() {
        let mut equal = true;
        let current_path = current_session.path();
        let mut current_depth = 1;

        while equal {
            equal = false;
            if let Some(current_str) = current_path.iter().rev().nth(current_depth) {
                for session in &mut *duplicate_sessions {
                    if let Some(str) = session.path().iter().rev().nth(current_depth) {
                        if str == current_str {
                            current_depth += 1;
                            equal = true;
                            break;
                        }
                    }
                }
            }
        }

        deduplicated.push(current_session);
        depth = depth.max(current_depth);
    }

    for session in &mut deduplicated {
        session.name = {
            let mut count = depth + 1;
            let mut iterator = session.path().iter().rev();
            let mut str = String::new();

            while count > 0 {
                if let Some(dir) = iterator.next() {
                    if str.is_empty() {
                        str = dir.to_string_lossy().to_string();
                    } else {
                        str = format!("{}/{}", dir.to_string_lossy(), str);
                    }
                    count -= 1;
                } else {
                    count = 0;
                }
            }

            str
        };
    }

    deduplicated
}

fn append_bookmarks(
    config: &Config,
    mut sessions: BTreeMap<String, Vec<Session>>,
) -> Result<BTreeMap<String, Vec<Session>>> {
    let bookmarks = config.bookmark_paths();

    for path in bookmarks {
        let session_name = path
            .file_name()
            .expect("The file name doesn't end in `..`")
            .to_string()?;
        let session = Session::new(session_name, SessionType::Bookmark(path));
        if let Some(list) = sessions.get_mut(&session.name) {
            list.push(session);
        } else {
            sessions.insert(session.name.clone(), vec![session]);
        }
    }

    Ok(sessions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn verify_session_name_deduplication() {
        let mut test_sessions = vec![
            Session::new(
                "test".into(),
                SessionType::Bookmark("/search/path/to/proj1/test".into()),
            ),
            Session::new(
                "test".into(),
                SessionType::Bookmark("/search/path/to/proj2/test".into()),
            ),
            Session::new(
                "test".into(),
                SessionType::Bookmark("/other/path/to/projects/proj2/test".into()),
            ),
        ];

        let deduplicated = deduplicate_sessions(&mut test_sessions);

        assert_eq!(deduplicated[0].name, "projects/proj2/test");
        assert_eq!(deduplicated[1].name, "to/proj2/test");
        assert_eq!(deduplicated[2].name, "to/proj1/test");
    }

    #[test]
    fn verify_btreemap_maintains_alphabetical_order() {
        let mut sessions: BTreeMap<String, Session> = BTreeMap::new();

        // Insert sessions in non-alphabetical order
        sessions.insert(
            "zebra".to_string(),
            Session::new("zebra".into(), SessionType::Bookmark("/path/zebra".into())),
        );
        sessions.insert(
            "apple".to_string(),
            Session::new("apple".into(), SessionType::Bookmark("/path/apple".into())),
        );
        sessions.insert(
            "middle".to_string(),
            Session::new(
                "middle".into(),
                SessionType::Bookmark("/path/middle".into()),
            ),
        );
        sessions.insert(
            "banana".to_string(),
            Session::new(
                "banana".into(),
                SessionType::Bookmark("/path/banana".into()),
            ),
        );

        // Verify list() returns them in alphabetical order
        let list = sessions.list();
        assert_eq!(list, vec!["apple", "banana", "middle", "zebra"]);

        // Verify keys are iterated in sorted order
        let keys: Vec<_> = sessions.keys().cloned().collect();
        assert_eq!(keys, vec!["apple", "banana", "middle", "zebra"]);
    }
}
