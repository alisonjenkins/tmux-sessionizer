use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use error_stack::ResultExt;
use serde_derive::Deserialize;
use tokio::process::Command as AsyncCommand;

use crate::{
    configs::{GitHubCloneMethod, GitHubProfile, GitHubRepo, GitHubRepoCache},
    error::TmsError,
    state::StateManager,
    Result,
};

const CACHE_DURATION_SECONDS: u64 = 3600; // 1 hour

#[derive(Debug, Deserialize)]
struct GitHubApiRepo {
    name: String,
    full_name: String,
    clone_url: String,
    ssh_url: String,
    description: Option<String>,
    updated_at: String,
}

pub struct GitHubClient {
    state_manager: StateManager,
}

impl GitHubClient {
    pub fn new() -> Result<Self> {
        let state_manager = StateManager::new()?;

        Ok(GitHubClient { state_manager })
    }

    pub async fn get_repositories(&self, profile: &GitHubProfile, force_refresh: bool) -> Result<Vec<GitHubRepo>> {
        let cache_file = self.state_manager.get_cache_file_path(&profile.name);
        
        // Try to load from cache first if not forcing refresh
        if !force_refresh {
            if let Ok(cached_repos) = self.load_cached_repos(&cache_file).await {
                return Ok(cached_repos);
            }
        }

        // Get fresh token
        let token = self.get_access_token(&profile.credentials_command).await?;
        
        // Fetch repositories from GitHub API
        let repos = self.fetch_repositories(&token).await?;
        
        // Cache the results
        self.cache_repositories(&cache_file, &profile.name, &repos).await?;
        
        Ok(repos)
    }

    async fn load_cached_repos(&self, cache_file: &Path) -> Result<Vec<GitHubRepo>> {
        let cache_content = tokio::fs::read_to_string(cache_file)
            .await
            .change_context(TmsError::IoError)?;
            
        let cache: GitHubRepoCache = serde_json::from_str(&cache_content)
            .change_context(TmsError::IoError)?;
            
        // Check if cache is still valid
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
            
        if now - cache.cached_at > CACHE_DURATION_SECONDS {
            return Err(TmsError::IoError.into()); // Cache expired
        }
        
        Ok(cache.repositories)
    }

    async fn get_access_token(&self, credentials_command: &str) -> Result<String> {
        let output = AsyncCommand::new("sh")
            .arg("-c")
            .arg(credentials_command)
            .output()
            .await
            .change_context(TmsError::GitError)?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            eprintln!("Warning: Failed to get GitHub token: {}", stderr);
            return Err(TmsError::GitError.into());
        }

        let token = String::from_utf8(output.stdout)
            .change_context(TmsError::GitError)?
            .trim()
            .to_string();
            
        if token.is_empty() {
            return Err(TmsError::GitError.into());
        }
        
        Ok(token)
    }

    async fn fetch_repositories(&self, token: &str) -> Result<Vec<GitHubRepo>> {
        let client = reqwest::Client::new();
        let mut repos = Vec::new();
        let mut page = 1;
        let per_page = 100;

        loop {
            let url = format!(
                "https://api.github.com/user/repos?page={}&per_page={}&sort=updated",
                page, per_page
            );

            let response = client
                .get(&url)
                .header("Authorization", format!("token {}", token))
                .header("User-Agent", "tmux-sessionizer")
                .send()
                .await
                .change_context(TmsError::GitError)?;

            if !response.status().is_success() {
                let status = response.status();
                let error_text = response.text().await.unwrap_or_default();
                eprintln!("GitHub API error {}: {}", status, error_text);
                return Err(TmsError::GitError.into());
            }

            let page_repos: Vec<GitHubApiRepo> = response
                .json()
                .await
                .change_context(TmsError::GitError)?;

            if page_repos.is_empty() {
                break;
            }

            repos.extend(page_repos.into_iter().map(|repo| GitHubRepo {
                name: repo.name,
                full_name: repo.full_name,
                clone_url_ssh: repo.ssh_url,
                clone_url_https: repo.clone_url,
                description: repo.description,
                updated_at: repo.updated_at,
            }));

            page += 1;

            // Limit to reasonable number of pages to avoid infinite loops
            if page > 50 {
                break;
            }
        }

        Ok(repos)
    }

    async fn cache_repositories(&self, cache_file: &Path, profile_name: &str, repos: &[GitHubRepo]) -> Result<()> {
        let cache = GitHubRepoCache {
            profile_name: profile_name.to_string(),
            repositories: repos.to_vec(),
            cached_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        };

        let cache_content = serde_json::to_string_pretty(&cache)
            .change_context(TmsError::IoError)?;

        tokio::fs::write(cache_file, cache_content)
            .await
            .change_context(TmsError::IoError)?;

        Ok(())
    }

    pub async fn clone_repository(
        &self,
        repo: &GitHubRepo,
        profile: &GitHubProfile,
        target_path: &Path,
    ) -> Result<PathBuf> {
        let clone_url = match profile.clone_method.as_ref().unwrap_or(&GitHubCloneMethod::SSH) {
            GitHubCloneMethod::SSH => &repo.clone_url_ssh,
            GitHubCloneMethod::HTTPS => &repo.clone_url_https,
        };

        let repo_path = target_path.join(&repo.name);

        // Check if repository already exists
        if repo_path.exists() {
            return Ok(repo_path);
        }

        // Ensure target directory exists
        std::fs::create_dir_all(target_path)
            .change_context(TmsError::IoError)?;

        let output = AsyncCommand::new("git")
            .args(&["clone", clone_url, &repo.name])
            .current_dir(target_path)
            .output()
            .await
            .change_context(TmsError::GitError)?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            eprintln!("Error cloning repository {}: {}", repo.full_name, stderr);
            return Err(TmsError::GitError.into());
        }

        Ok(repo_path)
    }
}

pub fn expand_clone_root_path(path: &str) -> Result<PathBuf> {
    let expanded = shellexpand::full(path)
        .change_context(TmsError::IoError)?;
    
    Ok(PathBuf::from(expanded.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_expand_clone_root_path() {
        // Test basic expansion
        let result = expand_clone_root_path("/tmp/test").unwrap();
        assert_eq!(result, PathBuf::from("/tmp/test"));
        
        // Test that it handles shell expansion (though we can't test ~ without actual home dir)
        let result = expand_clone_root_path("./test").unwrap();
        assert!(result.to_string_lossy().ends_with("/test"));
    }
}