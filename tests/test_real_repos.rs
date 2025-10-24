use std::fs;
use tempfile::tempdir;
use tms::configs::{Config, SearchDirectory};
use tms::repos::find_repos;

fn create_mock_git_repo(path: &std::path::Path) {
    // Create a mock .git directory structure that looks like a real git repo
    let git_dir = path.join(".git");
    fs::create_dir_all(&git_dir).expect("Failed to create .git directory");
    
    // Create essential git files that make it look like a real repo
    fs::write(git_dir.join("HEAD"), "ref: refs/heads/main\n").expect("Failed to create HEAD");
    
    let refs_heads = git_dir.join("refs").join("heads");
    fs::create_dir_all(&refs_heads).expect("Failed to create refs/heads");
    fs::write(refs_heads.join("main"), "0000000000000000000000000000000000000000\n")
        .expect("Failed to create main ref");
    
    let objects_dir = git_dir.join("objects");
    fs::create_dir_all(&objects_dir).expect("Failed to create objects dir");
    
    // Create config file
    fs::write(git_dir.join("config"), "[core]\n\trepositoryformatversion = 0\n")
        .expect("Failed to create config");
}

#[test]
fn test_async_repo_scanning_real() {
    // Create a temporary directory structure with multiple mock git repos
    let temp = tempdir().expect("Failed to create temp dir");
    let base_path = temp.path();

    // Create multiple test repositories
    let repo_paths = vec![
        base_path.join("repo1"),
        base_path.join("repo2"),
        base_path.join("nested/repo3"),
    ];

    for repo_path in &repo_paths {
        fs::create_dir_all(repo_path).expect("Failed to create repo dir");
        create_mock_git_repo(repo_path);
    }

    // Create config pointing to our test directory
    let config = Config {
        search_dirs: Some(vec![SearchDirectory::new(base_path.to_path_buf(), 3)]),
        ..Default::default()
    };

    // Run the async repo finder
    let result = find_repos(&config);
    assert!(
        result.is_ok(),
        "find_repos should succeed: {:?}",
        result.err()
    );

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

#[test]
fn test_repo_scanning_with_worktrees() {
    // Test scanning repositories with worktree-like structures
    let temp = tempdir().expect("Failed to create temp dir");
    let base_path = temp.path();

    // Create main repo
    let main_repo = base_path.join("main_repo");
    fs::create_dir_all(&main_repo).expect("Failed to create main repo");
    create_mock_git_repo(&main_repo);

    // Create what looks like a worktree (git directory with gitfile)
    let worktree_path = base_path.join("worktree");
    fs::create_dir_all(&worktree_path).expect("Failed to create worktree");
    
    // Create a gitfile pointing to the main repo (simulates git worktree)
    fs::write(
        worktree_path.join(".git"),
        format!("gitdir: {}", main_repo.join(".git").display())
    ).expect("Failed to create gitfile");

    let config = Config {
        search_dirs: Some(vec![SearchDirectory::new(base_path.to_path_buf(), 5)]),
        ..Default::default()
    };

    let result = find_repos(&config);
    assert!(result.is_ok(), "Repository scanning should succeed");
    
    let repos = result.unwrap();
    
    // Should find at least the main repository
    assert!(repos.len() >= 1, "Should find at least 1 repository");
}

#[test]
fn test_repo_scanning_mixed_content() {
    // Test scanning directories with both repos and regular directories
    let temp = tempdir().expect("Failed to create temp dir");
    let base_path = temp.path();

    // Create mix of repositories and regular directories
    let test_dirs = vec![
        ("project1", true),   // Git repo
        ("project2", false),  // Regular directory
        ("project3", true),   // Git repo
        ("docs", false),      // Regular directory
        ("scripts", false),   // Regular directory
        ("project4", true),   // Git repo
    ];

    let mut expected_repos = 0;
    for (name, is_repo) in &test_dirs {
        let dir_path = base_path.join(name);
        fs::create_dir_all(&dir_path).expect("Failed to create directory");
        
        if *is_repo {
            create_mock_git_repo(&dir_path);
            expected_repos += 1;
        } else {
            // Create some files to make it look like a real directory
            fs::write(dir_path.join("README.md"), "# Documentation").ok();
            fs::write(dir_path.join("notes.txt"), "Some notes").ok();
        }
    }

    let config = Config {
        search_dirs: Some(vec![SearchDirectory::new(base_path.to_path_buf(), 3)]),
        ..Default::default()
    };

    let result = find_repos(&config);
    assert!(result.is_ok(), "Repository scanning should succeed");
    
    let repos = result.unwrap();
    let found_repos = repos.len();
    
    assert_eq!(found_repos, expected_repos, "Should find exactly {} repositories, found {}", expected_repos, found_repos);
}
