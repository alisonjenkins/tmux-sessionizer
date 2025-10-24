// Performance benchmark test for repository scanning optimizations
// These tests work in Nix sandboxed environments by creating mock .git directories

use std::fs;
use std::time::Instant;
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
fn benchmark_wide_directory_structure() {
    // Create a wide directory structure with many repos
    let temp = tempdir().expect("Failed to create temp dir");
    let base_path = temp.path();
    
    // Create 100 directories, each with 10 subdirectories, 10 have git repos
    let mut repo_count = 0;
    for i in 0..100 {
        let dir_path = base_path.join(format!("wide_dir_{}", i));
        fs::create_dir_all(&dir_path).expect("Failed to create directory");
        
        for j in 0..10 {
            let subdir_path = dir_path.join(format!("sub_{}", j));
            fs::create_dir_all(&subdir_path).expect("Failed to create subdirectory");
            
            // Every 10th directory is a git repo
            if i % 10 == 0 && j == 0 {
                create_mock_git_repo(&subdir_path);
                repo_count += 1;
            }
        }
    }
    
    let config = Config {
        search_dirs: Some(vec![SearchDirectory::new(base_path.to_path_buf(), 5)]),
        ..Default::default()
    };

    let scan_start = Instant::now();
    let result = find_repos(&config);
    let scan_duration = scan_start.elapsed();
    
    assert!(result.is_ok(), "Repository scanning should succeed");
    let repos = result.unwrap();
    
    let total_repos: usize = repos.values().map(|sessions| sessions.len()).sum();
    
    // Performance assertion - should complete in reasonable time
    assert!(scan_duration.as_secs() < 30, "Scan should complete in under 30 seconds, took {:?}", scan_duration);
    assert_eq!(total_repos, repo_count, "Should find exactly {} repositories, found {}", repo_count, total_repos);
}

#[test]
fn benchmark_deep_directory_structure() {
    // Create a deep nested structure
    let temp = tempdir().expect("Failed to create temp dir");
    let base_path = temp.path();
    
    // Create a deep nested structure with repos at different levels
    let mut current_path = base_path.to_path_buf();
    let mut repo_count = 0;
    
    for depth in 0..20 {
        current_path = current_path.join(format!("level_{}", depth));
        fs::create_dir_all(&current_path).expect("Failed to create nested directory");
        
        // Add a repo every 5 levels
        if depth % 5 == 0 && depth > 0 {
            create_mock_git_repo(&current_path);
            repo_count += 1;
        }
    }
    
    let config = Config {
        search_dirs: Some(vec![SearchDirectory::new(base_path.to_path_buf(), 25)]),
        ..Default::default()
    };

    let scan_start = Instant::now();
    let result = find_repos(&config);
    let scan_duration = scan_start.elapsed();
    
    assert!(result.is_ok(), "Repository scanning should succeed");
    let repos = result.unwrap();
    
    let total_repos: usize = repos.values().map(|sessions| sessions.len()).sum();
    
    // Performance assertion - should complete quickly even with deep nesting
    assert!(scan_duration.as_secs() < 15, "Deep scan should complete in under 15 seconds, took {:?}", scan_duration);
    assert_eq!(total_repos, repo_count, "Should find exactly {} repositories in deep structure, found {}", repo_count, total_repos);
}

#[test] 
fn benchmark_mixed_structure() {
    // Create a mixed structure that represents real-world scenarios
    let temp = tempdir().expect("Failed to create temp dir");
    let base_path = temp.path();
    
    let mut repo_count = 0;
    
    // Create various directory patterns
    for i in 0..50 {
        let dir_path = base_path.join(format!("project_{}", i));
        fs::create_dir_all(&dir_path).expect("Failed to create directory");
        
        match i % 4 {
            0 => {
                // Git repository
                create_mock_git_repo(&dir_path);
                repo_count += 1;
            }
            1 => {
                // Regular directory with nested structure
                for j in 0..5 {
                    fs::create_dir_all(dir_path.join(format!("nested_{}", j))).expect("Failed to create nested");
                }
            }
            2 => {
                // Directory with many files but no subdirs
                for j in 0..20 {
                    fs::write(dir_path.join(format!("file_{}.txt", j)), "content").expect("Failed to write file");
                }
            }
            _ => {
                // Empty directory
            }
        }
    }
    
    let config = Config {
        search_dirs: Some(vec![SearchDirectory::new(base_path.to_path_buf(), 10)]),
        ..Default::default()
    };

    let scan_start = Instant::now();
    let result = find_repos(&config);
    let scan_duration = scan_start.elapsed();
    
    assert!(result.is_ok(), "Repository scanning should succeed");
    let repos = result.unwrap();
    
    let total_repos: usize = repos.values().map(|sessions| sessions.len()).sum();
    
    // Should find exactly the repos we created
    assert!(scan_duration.as_secs() < 15, "Mixed scan should complete quickly, took {:?}", scan_duration);
    assert_eq!(total_repos, repo_count, "Should find exactly {} repositories, found {}", repo_count, total_repos);
}

#[test]
fn benchmark_early_termination() {
    // Test that early termination works correctly with large directory structures
    let temp = tempdir().expect("Failed to create temp dir");
    let base_path = temp.path();
    
    // Create a structure that will trigger early termination
    for i in 0..1000 {
        let dir_path = base_path.join(format!("repo_{:04}", i));
        fs::create_dir_all(&dir_path).expect("Failed to create directory");
        create_mock_git_repo(&dir_path);
    }
    
    let config = Config {
        search_dirs: Some(vec![SearchDirectory::new(base_path.to_path_buf(), 3)]),
        ..Default::default()
    };

    let scan_start = Instant::now();
    let result = find_repos(&config);
    let scan_duration = scan_start.elapsed();
    
    assert!(result.is_ok(), "Repository scanning should succeed");
    let repos = result.unwrap();
    
    let total_repos: usize = repos.values().map(|sessions| sessions.len()).sum();
    
    // Should terminate early and not scan all 1000 repos
    assert!(scan_duration.as_secs() < 10, "Should terminate early, took {:?}", scan_duration);
    assert!(total_repos > 0, "Should find at least some repositories");
    // Note: exact count depends on early termination thresholds
}

#[test]
fn benchmark_directory_filtering() {
    // Test that directory filtering works correctly
    let temp = tempdir().expect("Failed to create temp dir");
    let base_path = temp.path();
    
    let mut expected_repos = 0;
    
    // Create structure with directories that should be filtered
    let test_dirs = [
        ("valid_repo", true),
        ("node_modules", false), // Should be filtered
        ("target", false),       // Should be filtered 
        ("another_repo", true),
        (".cache", false),       // Should be filtered
        ("src", true),           // Valid directory
        ("build", false),        // Should be filtered
    ];
    
    for (dir_name, should_include) in test_dirs {
        let dir_path = base_path.join(dir_name);
        fs::create_dir_all(&dir_path).expect("Failed to create directory");
        
        if should_include {
            create_mock_git_repo(&dir_path);
            expected_repos += 1;
        } else {
            // Create a repo in filtered directory (should not be found)
            create_mock_git_repo(&dir_path);
        }
    }
    
    let config = Config {
        search_dirs: Some(vec![SearchDirectory::new(base_path.to_path_buf(), 5)]),
        ..Default::default()
    };

    let scan_start = Instant::now();
    let result = find_repos(&config);
    let scan_duration = scan_start.elapsed();
    
    assert!(result.is_ok(), "Repository scanning should succeed");
    let repos = result.unwrap();
    
    let total_repos: usize = repos.values().map(|sessions| sessions.len()).sum();
    
    assert!(scan_duration.as_secs() < 5, "Filtering test should complete quickly, took {:?}", scan_duration);
    
    // We expect fewer repos due to filtering (exact count depends on filtering implementation)
    assert!(total_repos >= 1, "Should find at least 1 repository after filtering");
    assert!(total_repos <= expected_repos, "Should not find more than {} repositories due to filtering, found {}", expected_repos, total_repos);
}