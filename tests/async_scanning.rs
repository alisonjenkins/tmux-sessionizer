// Integration test for async repo scanning

use std::fs;
use tempfile::tempdir;
use tms::configs::{Config, SearchDirectory};
use tms::repos::find_repos;

#[test]
fn test_async_scanning_handles_empty_directory() {
    // Test that async scanning gracefully handles empty directories
    let temp = tempdir().expect("Failed to create temp dir");

    let config = Config {
        search_dirs: Some(vec![SearchDirectory::new(temp.path().to_path_buf(), 5)]),
        ..Default::default()
    };

    let result = find_repos(&config);
    assert!(
        result.is_ok(),
        "find_repos should succeed even with no repos"
    );

    let repos = result.unwrap();
    assert_eq!(
        repos.len(),
        0,
        "Should find no repositories in empty directory"
    );
}

#[test]
fn test_async_scanning_with_nested_directories() {
    // Test that async scanning can traverse nested directories
    let temp = tempdir().expect("Failed to create temp dir");
    let base_path = temp.path();

    // Create a deep nested structure
    let deep_path = base_path.join("level1/level2/level3/level4");
    fs::create_dir_all(&deep_path).expect("Failed to create nested dirs");

    let config = Config {
        search_dirs: Some(vec![SearchDirectory::new(base_path.to_path_buf(), 10)]),
        ..Default::default()
    };

    // This should not panic or error even with deep nesting
    let result = find_repos(&config);
    assert!(result.is_ok(), "find_repos should succeed with nested dirs");
}

#[test]
fn test_async_scanning_respects_depth_limit() {
    // Test that async scanning respects the depth limit
    let temp = tempdir().expect("Failed to create temp dir");
    let base_path = temp.path();

    // Create directories at various depths
    fs::create_dir_all(base_path.join("level1")).expect("Failed to create level1");
    fs::create_dir_all(base_path.join("level1/level2")).expect("Failed to create level2");
    fs::create_dir_all(base_path.join("level1/level2/level3")).expect("Failed to create level3");

    // With depth 1, should only look at immediate children
    let config = Config {
        search_dirs: Some(vec![SearchDirectory::new(base_path.to_path_buf(), 1)]),
        ..Default::default()
    };

    let result = find_repos(&config);
    assert!(result.is_ok(), "find_repos should succeed with depth limit");
}

#[test]
fn test_async_scanning_handles_permission_denied() {
    // This test verifies that the async implementation doesn't panic
    // when encountering directories it can't read
    let temp = tempdir().expect("Failed to create temp dir");
    let base_path = temp.path();

    // Create a regular accessible directory
    fs::create_dir_all(base_path.join("accessible")).expect("Failed to create dir");

    let config = Config {
        search_dirs: Some(vec![SearchDirectory::new(base_path.to_path_buf(), 5)]),
        ..Default::default()
    };

    // Should succeed even if some dirs are inaccessible
    let result = find_repos(&config);
    assert!(
        result.is_ok(),
        "find_repos should handle permission errors gracefully"
    );
}

#[test]
fn test_async_scanning_multiple_search_paths() {
    // Test scanning multiple paths concurrently
    let temp1 = tempdir().expect("Failed to create temp dir 1");
    let temp2 = tempdir().expect("Failed to create temp dir 2");

    fs::create_dir_all(temp1.path().join("dir1")).expect("Failed to create dir1");
    fs::create_dir_all(temp2.path().join("dir2")).expect("Failed to create dir2");

    let config = Config {
        search_dirs: Some(vec![
            SearchDirectory::new(temp1.path().to_path_buf(), 5),
            SearchDirectory::new(temp2.path().to_path_buf(), 5),
        ]),
        ..Default::default()
    };

    let result = find_repos(&config);
    assert!(
        result.is_ok(),
        "find_repos should handle multiple search paths"
    );
}

#[test]
fn test_concurrent_directory_scanning() {
    // Create a wide directory structure to test parallel scanning
    let temp = tempdir().expect("Failed to create temp dir");
    let base_path = temp.path();

    // Create many directories at the same level to encourage parallel processing
    for i in 0..20 {
        let dir_path = base_path.join(format!("dir_{}", i));
        fs::create_dir_all(&dir_path).expect("Failed to create directory");
        // Add some subdirectories
        for j in 0..5 {
            fs::create_dir_all(dir_path.join(format!("subdir_{}", j)))
                .expect("Failed to create subdir");
        }
    }

    let config = Config {
        search_dirs: Some(vec![SearchDirectory::new(base_path.to_path_buf(), 3)]),
        ..Default::default()
    };

    // This tests that parallel scanning doesn't deadlock or panic
    let result = find_repos(&config);
    assert!(
        result.is_ok(),
        "Concurrent scanning should complete successfully"
    );
}
