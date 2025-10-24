use std::fs;
use tempfile::tempdir;
use tms::configs::{Config, SearchDirectory};
use tms::repos::find_repos;

#[test]
fn test_async_repo_scanning_real() {
    // Check if git is available
    let git_check = std::process::Command::new("git")
        .arg("--version")
        .output();
    
    if git_check.is_err() {
        eprintln!("Skipping test: git command not available");
        return;
    }

    // Create a temporary directory structure with multiple git repos
    let temp = tempdir().expect("Failed to create temp dir");
    let base_path = temp.path();

    // Configure git (ignore errors if already configured)
    let _ = std::process::Command::new("git")
        .args(["config", "--global", "user.email", "test@test.com"])
        .output();
    let _ = std::process::Command::new("git")
        .args(["config", "--global", "user.name", "Test User"])
        .output();
    let _ = std::process::Command::new("git")
        .args(["config", "--global", "init.defaultBranch", "main"])
        .output();

    // Create multiple test repositories
    let repo_paths = vec![
        base_path.join("repo1"),
        base_path.join("repo2"),
        base_path.join("nested/repo3"),
    ];

    for repo_path in &repo_paths {
        fs::create_dir_all(repo_path).expect("Failed to create repo dir");
        let output = std::process::Command::new("git")
            .arg("init")
            .current_dir(repo_path)
            .output();
        
        match output {
            Ok(out) => {
                if !out.status.success() {
                    eprintln!("Git init failed at {:?}: {}", repo_path, String::from_utf8_lossy(&out.stderr));
                    eprintln!("Skipping test due to git init failure");
                    return;
                }
            }
            Err(e) => {
                eprintln!("Failed to run git init: {}", e);
                eprintln!("Skipping test");
                return;
            }
        }
    }

    // Create config pointing to our test directory
    let config = Config {
        search_dirs: Some(vec![SearchDirectory::new(base_path.to_path_buf(), 3)]),
        ..Default::default()
    };

    // Run the async repo finder
    let result = find_repos(&config);
    assert!(result.is_ok(), "find_repos should succeed: {:?}", result.err());

    let repos = result.unwrap();
    
    // We should find all 3 repositories
    assert_eq!(
        repos.len(),
        3,
        "Should find exactly 3 repositories, found {} repos: {:?}",
        repos.len(),
        repos.keys().collect::<Vec<_>>()
    );
}
