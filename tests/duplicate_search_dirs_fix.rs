use std::fs;
use tempfile::TempDir;
use tms::configs::{Config, SearchDirectory};

#[test]
fn test_duplicate_search_directories_bug_fix() {
    // Create a temporary directory structure for testing
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let base_path = temp_dir.path();
    
    // Create a test repository structure
    let repo_path = base_path.join("test_repo");
    fs::create_dir_all(&repo_path).expect("Failed to create repo dir");
    fs::create_dir_all(repo_path.join(".git")).expect("Failed to create .git dir");
    
    // Create a config with duplicate search directories (the original bug scenario)
    let mut config = Config::default();
    config.search_dirs = Some(vec![
        SearchDirectory::new(base_path.to_path_buf(), 10),
        SearchDirectory::new(base_path.to_path_buf(), 10), // Exact duplicate
        SearchDirectory::new(base_path.to_path_buf(), 5),  // Same path, different depth
    ]);
    
    // Get the deduplicated search directories
    let search_dirs = config.search_dirs().expect("Failed to get search dirs");
    
    // Should have only 1 unique path after deduplication
    assert_eq!(search_dirs.len(), 1, "Should deduplicate identical paths");
    
    // Should keep the entry with the maximum depth (10)
    let dedup_dir = &search_dirs[0];
    assert_eq!(dedup_dir.path, base_path.canonicalize().unwrap());
    assert_eq!(dedup_dir.depth, 10, "Should keep the entry with maximum depth");
}

#[test]
fn test_no_false_deduplication() {
    // Ensure we don't deduplicate legitimately different paths
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let base_path = temp_dir.path();
    
    // Create different directories
    let repo1_path = base_path.join("repo1");
    let repo2_path = base_path.join("repo2");
    fs::create_dir_all(&repo1_path).expect("Failed to create repo1 dir");
    fs::create_dir_all(&repo2_path).expect("Failed to create repo2 dir");
    
    let mut config = Config::default();
    config.search_dirs = Some(vec![
        SearchDirectory::new(repo1_path, 5),
        SearchDirectory::new(repo2_path, 10),
    ]);
    
    let search_dirs = config.search_dirs().expect("Failed to get search dirs");
    
    // Should have both directories since they're different paths
    assert_eq!(search_dirs.len(), 2, "Should not deduplicate different paths");
    
    // Verify both paths are present
    let paths: Vec<_> = search_dirs.iter().map(|d| &d.path).collect();
    assert!(paths.iter().any(|p| p.ends_with("repo1")));
    assert!(paths.iter().any(|p| p.ends_with("repo2")));
}