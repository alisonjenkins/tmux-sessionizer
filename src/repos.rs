use aho_corasick::{AhoCorasickBuilder, MatchKind};
use error_stack::{report, Report, ResultExt};
use gix::{Repository, Submodule};
use jj_lib::{
    config::StackedConfig,
    git_backend::GitBackend,
    local_working_copy::{LocalWorkingCopy, LocalWorkingCopyFactory},
    repo::StoreFactories,
    settings::UserSettings,
    workspace::{WorkingCopyFactories, Workspace},
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::{self, Stdio},
    sync::{Arc, Mutex, atomic::{AtomicU64, AtomicUsize, Ordering}},
    time::{Duration, Instant},
};

use crate::{
    configs::{Config, SearchDirectory, VcsProviders, DEFAULT_VCS_PROVIDERS},
    dirty_paths::DirtyUtf8Path,
    session::{Session, SessionContainer, SessionType},
    Result, TmsError,
};

pub trait Worktree {
    fn name(&self) -> String;

    fn path(&self) -> Result<PathBuf>;

    fn is_prunable(&self) -> bool;
}

impl Worktree for gix::worktree::Proxy<'_> {
    fn name(&self) -> String {
        self.id().to_string()
    }

    fn path(&self) -> Result<PathBuf> {
        self.base().change_context(TmsError::GitError)
    }

    fn is_prunable(&self) -> bool {
        !self.base().is_ok_and(|path| path.exists())
    }
}

impl Worktree for Workspace {
    fn name(&self) -> String {
        self.working_copy().workspace_name().as_str().to_string()
    }

    fn path(&self) -> Result<PathBuf> {
        Ok(self.workspace_root().to_path_buf())
    }

    fn is_prunable(&self) -> bool {
        false
    }
}

pub enum RepoProvider {
    Git(Box<Repository>),
    Jujutsu(Workspace),
}

impl From<gix::Repository> for RepoProvider {
    fn from(repo: gix::Repository) -> Self {
        Self::Git(Box::new(repo))
    }
}

impl RepoProvider {
    pub fn open(path: &Path, config: &Config) -> Result<Self> {
        fn open_git(path: &Path) -> Result<RepoProvider> {
            gix::open(path)
                .map(|repo| RepoProvider::Git(Box::new(repo)))
                .change_context(TmsError::GitError)
        }

        fn open_jj(path: &Path) -> Result<RepoProvider> {
            let user_settings = UserSettings::from_config(StackedConfig::with_defaults())
                .change_context(TmsError::GitError)?;
            let mut store_factories = StoreFactories::default();
            store_factories.add_backend(
                GitBackend::name(),
                Box::new(|settings, store_path| {
                    Ok(Box::new(GitBackend::load(settings, store_path)?))
                }),
            );
            let mut working_copy_factories = WorkingCopyFactories::new();
            working_copy_factories.insert(
                LocalWorkingCopy::name().to_owned(),
                Box::new(LocalWorkingCopyFactory {}),
            );

            Workspace::load(
                &user_settings,
                path,
                &store_factories,
                &working_copy_factories,
            )
            .map(RepoProvider::Jujutsu)
            .change_context(TmsError::GitError)
        }

        let vcs_provider_config = config
            .vcs_providers
            .as_ref()
            .map(|providers| providers.iter())
            .unwrap_or(DEFAULT_VCS_PROVIDERS.iter());

        let results = vcs_provider_config
            .filter_map(|provider| match provider {
                VcsProviders::Git => open_git(path).ok(),
                VcsProviders::Jujutsu => open_jj(path).ok(),
            })
            .take(1);
        results
            .into_iter()
            .next()
            .ok_or(TmsError::GitError)
            .change_context(TmsError::GitError)
    }

    pub fn is_worktree(&self) -> bool {
        match self {
            RepoProvider::Git(repo) => !repo.main_repo().is_ok_and(|r| r == **repo),
            RepoProvider::Jujutsu(repo) => {
                let repo_path = repo.repo_path();
                let workspace_repo_path = repo.workspace_root().join(".jj/repo");
                repo_path != workspace_repo_path
            }
        }
    }

    pub fn path(&self) -> &Path {
        match self {
            RepoProvider::Git(repo) => repo.path(),
            RepoProvider::Jujutsu(repo) => repo.workspace_root(),
        }
    }

    pub fn main_repo(&self) -> Option<PathBuf> {
        match self {
            RepoProvider::Git(repo) => repo.main_repo().map(|repo| repo.path().to_path_buf()).ok(),
            RepoProvider::Jujutsu(repo) => Some(repo.repo_path().to_path_buf()),
        }
    }

    pub fn work_dir(&self) -> Option<&Path> {
        match self {
            RepoProvider::Git(repo) => repo.workdir(),
            RepoProvider::Jujutsu(repo) => Some(repo.workspace_root()),
        }
    }

    pub fn head_name(&self) -> Result<String> {
        match self {
            RepoProvider::Git(repo) => Ok(repo
                .head_name()
                .change_context(TmsError::GitError)?
                .ok_or(TmsError::GitError)?
                .shorten()
                .to_string()),
            RepoProvider::Jujutsu(_) => Err(TmsError::GitError.into()),
        }
    }
    pub fn submodules(&'_ self) -> Result<Option<impl Iterator<Item = Submodule<'_>>>> {
        match self {
            RepoProvider::Git(repo) => repo.submodules().change_context(TmsError::GitError),
            RepoProvider::Jujutsu(_) => Ok(None),
        }
    }

    pub fn is_bare(&self) -> bool {
        match self {
            RepoProvider::Git(repo) => repo.is_bare(),
            RepoProvider::Jujutsu(workspace) => {
                let loader = workspace.repo_loader();
                let store = loader.store();
                let Ok(repo) = loader.load_at_head() else {
                    return false;
                };
                // currently checked out commit, get from current (default) workspace
                let Some(commit_id) = repo.view().wc_commit_ids().get(workspace.workspace_name())
                else {
                    return false;
                };
                let Ok(commit) = store.get_commit(commit_id) else {
                    return false;
                };
                // if parent is root commit then it's the only possible parent
                let Some(Ok(parent)) = commit.parents().next() else {
                    return false;
                };

                // root commit is direct parent of current commit => repo is effectively bare
                // current commit should be empty
                parent.change_id() == store.root_commit().change_id()
                    && commit.is_empty(&*repo).unwrap_or_default()
            }
        }
    }

    pub fn add_worktree(&self, path: &Path) -> Result<Option<(String, PathBuf)>> {
        match self {
            RepoProvider::Git(_) => {
                let Ok(head) = self.head_name() else {
                    return Ok(None);
                };
                // Add the default branch as a tree (usually either main or master)
                process::Command::new("git")
                    .current_dir(path)
                    .args(["worktree", "add", &head])
                    .stderr(Stdio::inherit())
                    .output()
                    .change_context(TmsError::GitError)?;
                Ok(Some((head.clone(), path.to_path_buf().join(&head))))
            }
            RepoProvider::Jujutsu(_) => {
                process::Command::new("jj")
                    .current_dir(path)
                    .args(["workspace", "add", "-r", "trunk()", "trunk"])
                    .stderr(Stdio::inherit())
                    .output()
                    .change_context(TmsError::GitError)?;
                Ok(Some(("trunk".into(), path.to_path_buf().join("trunk"))))
            }
        }
    }

    pub fn worktrees(&'_ self, config: &Config) -> Result<Vec<Box<dyn Worktree + '_>>> {
        match self {
            RepoProvider::Git(repo) => Ok(repo
                .worktrees()
                .change_context(TmsError::GitError)?
                .into_iter()
                .map(|i| Box::new(i) as Box<dyn Worktree>)
                .collect()),

            RepoProvider::Jujutsu(workspace) => {
                let repos: Arc<Mutex<Vec<RepoProvider>>> = Arc::new(Mutex::new(Vec::new()));

                search_dirs(config, |_, repo| {
                    if !repo.is_worktree() {
                        return Ok(());
                    }
                    let Some(path) = repo.main_repo() else {
                        return Ok(());
                    };
                    if workspace.repo_path() == path {
                        repos
                            .lock()
                            .map_err(|_| TmsError::IoError)
                            .change_context(TmsError::IoError)?
                            .push(repo);
                    }
                    Ok(())
                })?;

                let mut repos = Arc::try_unwrap(repos)
                    .map_err(|_| TmsError::IoError)
                    .change_context(TmsError::IoError)?
                    .into_inner()
                    .map_err(|_| TmsError::IoError)
                    .change_context(TmsError::IoError)?;

                if self.is_bare() {
                    if let Ok(read_dir) = std::fs::read_dir(self.path()) {
                        let mut sub = read_dir
                            .filter_map(|entry| entry.ok())
                            .map(|dir| dir.path())
                            .filter(|path| path.is_dir())
                            .filter_map(|path| RepoProvider::open(&path, config).ok())
                            .filter(|repo| matches!(repo, RepoProvider::Jujutsu(_)))
                            .filter(|repo| {
                                repo.main_repo()
                                    .is_some_and(|main| main == self.path().join(".jj/repo"))
                            })
                            .collect::<Vec<_>>();
                        repos.append(&mut sub);
                    }
                }

                let repos = repos
                    .into_iter()
                    .filter_map(|repo| match repo {
                        RepoProvider::Jujutsu(r) => Some(r),
                        _ => None,
                    })
                    .map(|i| Box::new(i) as Box<dyn Worktree>)
                    .collect();
                Ok(repos)
            }
        }
    }
}

pub fn find_repos(config: &Config) -> Result<BTreeMap<String, Vec<Session>>> {
    let start_time = Instant::now();
    eprintln!("[TRACE] Starting repository search...");
    
    let repos: Arc<Mutex<BTreeMap<String, Vec<Session>>>> = Arc::new(Mutex::new(BTreeMap::new()));

    search_dirs(config, |file, repo| {
        if repo.is_worktree() {
            return Ok(());
        }

        let session_name = file
            .path
            .file_name()
            .ok_or_else(|| {
                Report::new(TmsError::GitError).attach_printable("Not a valid repository name")
            })?
            .to_string()?;

        let session = Session::new(session_name, SessionType::Git(Box::new(repo)));
        let mut repos = repos
            .lock()
            .map_err(|_| TmsError::IoError)
            .change_context(TmsError::IoError)?;
        if let Some(list) = repos.get_mut(&session.name) {
            list.push(session);
        } else {
            repos.insert(session.name.clone(), vec![session]);
        }
        Ok(())
    })?;

    let repos = Arc::try_unwrap(repos)
        .map_err(|_| TmsError::IoError)
        .change_context(TmsError::IoError)?
        .into_inner()
        .map_err(|_| TmsError::IoError)
        .change_context(TmsError::IoError)?;
        
    let total_time = start_time.elapsed();
    let repo_count = repos.values().map(|v| v.len()).sum::<usize>();
    eprintln!("[TRACE] Repository search completed: found {} repos in {:.2}ms", repo_count, total_time.as_millis());
    
    Ok(repos)
}

fn search_dirs<F>(config: &Config, f: F) -> Result<()>
where
    F: Fn(SearchDirectory, RepoProvider) -> Result<()>,
{
    let start_time = Instant::now();
    let directories = config.search_dirs().change_context(TmsError::ConfigError)?;
    eprintln!("[TRACE] Starting search in {} directories", directories.len());
    for (i, dir) in directories.iter().enumerate() {
        eprintln!("[TRACE] Search dir {}: {} (depth: {})", i+1, dir.path.display(), dir.depth);
    }
    
    let to_search: Arc<Mutex<Vec<SearchDirectory>>> = Arc::new(Mutex::new(directories));

    let excluder = if let Some(excluded_dirs) = &config.excluded_dirs {
        eprintln!("[TRACE] Exclusion patterns: {} patterns configured", excluded_dirs.len());
        Some(Arc::new(
            AhoCorasickBuilder::new()
                .match_kind(MatchKind::LeftmostFirst)
                .build(excluded_dirs)
                .change_context(TmsError::IoError)?,
        ))
    } else {
        eprintln!("[TRACE] No exclusion patterns configured");
        None
    };

    // Performance counters
    let dirs_scanned = Arc::new(AtomicUsize::new(0));
    let dirs_excluded = Arc::new(AtomicUsize::new(0)); 
    let likely_repos_found = Arc::new(AtomicUsize::new(0));
    let repos_opened = Arc::new(AtomicUsize::new(0));
    let repo_open_failures = Arc::new(AtomicUsize::new(0));
    let total_repo_open_time = Arc::new(AtomicU64::new(0));

    let cpu_count = num_cpus::get();
    eprintln!("[TRACE] System has {} CPU cores", cpu_count);
    let worker_threads = cpu_count.max(4);
    eprintln!("[TRACE] Using {} worker threads", worker_threads);

    // Smart optimization: common paths that usually contain many build artifacts or deps but few repos
    let common_skip_patterns = Arc::new([
        "node_modules",
        "target", 
        "build",
        "dist",
        ".gradle", 
        ".m2",
        ".cargo",
        ".npm",
        ".cache",
        "__pycache__",
        "venv",
        ".venv",
        "env",
        ".env",
        "vendor",
        ".terraform",
        "site-packages",
        ".pytest_cache",
        ".mypy_cache",
        "coverage",
        ".coverage",
        ".nyc_output",
        ".next",
        ".nuxt",
        "Pods",
        "DerivedData",
        ".ccls-cache",
        ".clangd",
    ]);

    // Use optimized tokio runtime for async directory scanning
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(worker_threads) // Ensure minimum 4 threads
        .thread_keep_alive(Duration::from_secs(60)) // Keep threads alive longer
        .enable_all()
        .build()
        .change_context(TmsError::IoError)?;

    runtime.block_on(async {
        let mut tasks = Vec::new();
        let mut last_report = Instant::now();
        let mut total_iterations = 0u64;

        loop {
            total_iterations += 1;
            
            // Report progress every 5 seconds
            if last_report.elapsed() > Duration::from_secs(5) {
                let scanned = dirs_scanned.load(Ordering::Relaxed);
                let excluded = dirs_excluded.load(Ordering::Relaxed);
                let likely = likely_repos_found.load(Ordering::Relaxed);
                let opened = repos_opened.load(Ordering::Relaxed);
                let failures = repo_open_failures.load(Ordering::Relaxed);
                let elapsed = start_time.elapsed();
                
                eprintln!("[TRACE] Progress after {:.2}s: dirs_scanned={}, dirs_excluded={}, likely_repos={}, repos_opened={}, failures={}, active_tasks={}", 
                    elapsed.as_secs_f64(), scanned, excluded, likely, opened, failures, tasks.len());
                last_report = Instant::now();
            }

            // Try to get the next directory to process
            let file = {
                let mut search_queue = to_search.lock().map_err(|_| TmsError::IoError).change_context(TmsError::IoError)?;
                search_queue.pop()
            };

            match file {
                Some(file) => {
                    dirs_scanned.fetch_add(1, Ordering::Relaxed);
                    
                    // We have a directory to process
                    if let Some(ref excluder) = excluder {
                        if excluder.is_match(&file.path.to_string()?) {
                            dirs_excluded.fetch_add(1, Ordering::Relaxed);
                            continue;
                        }
                    }

                    // Early termination: if we've found a lot of repos and scanned many dirs, consider stopping
                    let current_repos = repos_opened.load(Ordering::Relaxed);
                    let current_dirs = dirs_scanned.load(Ordering::Relaxed);
                    
                    // Adaptive limits: more aggressive limits for very large search spaces
                    let should_terminate = if current_dirs > 500_000 {
                        current_repos > 300 // More conservative limit for huge directories
                    } else if current_dirs > 100_000 {
                        current_repos > 800 // Reasonable limit for large directories
                    } else {
                        current_repos > 2000 // Original generous limit
                    };
                    
                    if should_terminate {
                        eprintln!("[TRACE] Early termination: found {} repos after scanning {} directories", current_repos, current_dirs);
                        eprintln!("[TRACE] This prevents excessive scanning in very large directory structures.");
                        break;
                    }

                    let to_search_clone = Arc::clone(&to_search);
                    let excluder_clone = excluder.clone();
                    let f_ref = &f;
                    let _dirs_scanned_clone = Arc::clone(&dirs_scanned);
                    let dirs_excluded_clone = Arc::clone(&dirs_excluded);
                    let likely_repos_found_clone = Arc::clone(&likely_repos_found);
                    let repos_opened_clone = Arc::clone(&repos_opened);
                    let repo_open_failures_clone = Arc::clone(&repo_open_failures);
                    let total_repo_open_time_clone = Arc::clone(&total_repo_open_time);

                    let common_skip_patterns_clone = Arc::clone(&common_skip_patterns);

                    // Fast pre-check: only try to open paths that likely contain repositories
                    let likely_repo = file.path.join(".git").exists() || file.path.join(".jj").exists();
                    
                    if likely_repo {
                        likely_repos_found_clone.fetch_add(1, Ordering::Relaxed);
                        
                        // Check if it's a repo (blocking operation)
                        let repo_open_start = Instant::now();
                        match RepoProvider::open(&file.path, config) {
                            Ok(repo) => {
                                let repo_open_time = repo_open_start.elapsed();
                                total_repo_open_time_clone.fetch_add(repo_open_time.as_nanos() as u64, Ordering::Relaxed);
                                repos_opened_clone.fetch_add(1, Ordering::Relaxed);
                                f_ref(file.clone(), repo)?;
                            }
                            Err(_) => {
                                let repo_open_time = repo_open_start.elapsed();
                                total_repo_open_time_clone.fetch_add(repo_open_time.as_nanos() as u64, Ordering::Relaxed);
                                repo_open_failures_clone.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }
                    
                    // Continue directory traversal regardless
                    if file.path.is_dir() && file.depth > 0 {
                        // Scan directory asynchronously
                        let task = tokio::spawn(async move {
                            match tokio::fs::read_dir(&file.path).await {
                                Err(ref e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                                    if let Ok(path_str) = file.path.to_string() {
                                        eprintln!(
                                            "[TRACE] Warning: insufficient permissions to read '{}'. Skipping directory...",
                                            path_str
                                        );
                                    }
                                    Ok(())
                                }
                                Err(e) => {
                                    eprintln!("[TRACE] Error reading directory {:?}: {}", file.path, e);
                                    Err(report!(e)
                                        .change_context(TmsError::IoError)
                                        .attach_printable(format!("Could not read directory {:?}", file.path)))
                                }
                                Ok(mut read_dir) => {
                                    let mut subdirs = Vec::with_capacity(32); // Pre-allocate for performance
                                    while let Ok(Some(dir_entry)) = read_dir.next_entry().await {
                                        let path = dir_entry.path();
                                        if path.is_dir() {
                                            // Smart skip patterns - check common build/dependency directories
                                            if let Some(dir_name) = path.file_name().and_then(|n| n.to_str()) {
                                                if common_skip_patterns_clone.iter().any(|&pattern| dir_name == pattern) {
                                                    continue; // Skip common build directories
                                                }
                                            }
                                            
                                            // Check exclusion patterns after smart filtering
                                            if let Some(ref excluder) = excluder_clone {
                                                if let Ok(path_str) = path.to_string() {
                                                    if excluder.is_match(&path_str) {
                                                        dirs_excluded_clone.fetch_add(1, Ordering::Relaxed);
                                                        continue;
                                                    }
                                                }
                                            }
                                            subdirs.push(SearchDirectory::new(path, file.depth - 1));
                                        }
                                    }

                                    if !subdirs.is_empty() {
                                        if let Ok(mut search_queue) = to_search_clone.lock() {
                                            search_queue.extend(subdirs);
                                        }
                                    }
                                    Ok(())
                                }
                            }
                        });

                        tasks.push(task);

                        // Limit concurrent tasks with higher limits for better performance
                        if tasks.len() >= 200 {
                            while tasks.len() > 100 {
                                if let Some(task) = tasks.pop() {
                                    task.await.change_context(TmsError::IoError)??;
                                }
                            }
                        }
                    }
                }
                None => {
                    // Queue is empty, check if we have pending tasks
                    if tasks.is_empty() {
                        // No more work to do
                        break;
                    }

                    // Wait for at least one task to complete, which might add more directories
                    if let Some(task) = tasks.pop() {
                        task.await.change_context(TmsError::IoError)??;
                    }

                    // Continue the loop to check if new items were added to the queue
                }
            }
        }

        // Wait for all remaining tasks
        for task in tasks {
            task.await.change_context(TmsError::IoError)??;
        }

        // Final statistics
        let final_scanned = dirs_scanned.load(Ordering::Relaxed);
        let final_excluded = dirs_excluded.load(Ordering::Relaxed);
        let final_likely = likely_repos_found.load(Ordering::Relaxed);
        let final_opened = repos_opened.load(Ordering::Relaxed);
        let final_failures = repo_open_failures.load(Ordering::Relaxed);
        let total_repo_open_ns = total_repo_open_time.load(Ordering::Relaxed);
        let total_elapsed = start_time.elapsed();
        
        eprintln!("[TRACE] Search completed in {:.2}ms:", total_elapsed.as_millis());
        eprintln!("[TRACE]   - Directories scanned: {}", final_scanned);
        eprintln!("[TRACE]   - Directories excluded: {}", final_excluded);
        eprintln!("[TRACE]   - Likely repos found: {}", final_likely);
        eprintln!("[TRACE]   - Repos successfully opened: {}", final_opened);
        eprintln!("[TRACE]   - Repository open failures: {}", final_failures);
        eprintln!("[TRACE]   - Total iterations: {}", total_iterations);
        if final_opened > 0 {
            let avg_repo_open_ms = (total_repo_open_ns as f64 / 1_000_000.0) / final_opened as f64;
            eprintln!("[TRACE]   - Average repo open time: {:.2}ms", avg_repo_open_ms);
        }
        eprintln!("[TRACE]   - Directories per second: {:.0}", final_scanned as f64 / total_elapsed.as_secs_f64());
        if final_likely > 0 {
            eprintln!("[TRACE]   - Repository detection accuracy: {:.1}%", (final_opened as f64 / final_likely as f64) * 100.0);
        }

        Ok(())
    })
}

pub fn find_submodules<'a>(
    submodules: impl Iterator<Item = Submodule<'a>>,
    parent_name: &String,
    repos: &mut impl SessionContainer,
    config: &Config,
) -> Result<()> {
    for submodule in submodules {
        let repo = match submodule.open() {
            Ok(Some(repo)) => repo,
            _ => continue,
        };
        let path = match repo.workdir() {
            Some(path) => path,
            _ => continue,
        };
        let submodule_file_name = path
            .file_name()
            .ok_or_else(|| {
                Report::new(TmsError::GitError).attach_printable("Not a valid submodule name")
            })?
            .to_string()?;
        let session_name = format!("{}>{}", parent_name, submodule_file_name);
        let name = if let Some(true) = config.display_full_path {
            path.display().to_string()
        } else {
            session_name.clone()
        };

        if config.recursive_submodules == Some(true) {
            if let Ok(Some(submodules)) = repo.submodules() {
                find_submodules(submodules, &name, repos, config)?;
            }
        }
        let session = Session::new(session_name, SessionType::Git(Box::new(repo.into())));
        repos.insert_session(name, session);
    }
    Ok(())
}
