// Real-world performance benchmark that works in Nix sandboxed environments

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
fn real_world_benchmark() {
    // Create a realistic directory structure similar to a developer's workspace
    let temp = tempdir().expect("Failed to create temp dir");
    let base_path = temp.path();
    
    // Create different types of projects
    let project_types = [
        ("rust-projects", 15, true),      // 15 Rust projects (with git)
        ("node-projects", 20, true),      // 20 Node.js projects (with git) 
        ("python-projects", 10, true),    // 10 Python projects (with git)
        ("docs", 5, false),              // 5 documentation directories (no git)
        ("archived", 8, false),          // 8 archived projects (no git)
        ("temp-work", 12, false),        // 12 temporary work directories (no git)
    ];
    
    let mut total_repos = 0;
    let mut _total_dirs = 0;
    
    for (category, count, has_git) in project_types {
        let category_path = base_path.join(category);
        fs::create_dir_all(&category_path).expect("Failed to create category directory");
        
        for i in 1..=count {
            let project_path = category_path.join(format!("project_{:02}", i));
            fs::create_dir_all(&project_path).expect("Failed to create project directory");
            
            // Create some realistic subdirectories
            let subdirs = ["src", "docs", "tests"];
            for subdir in subdirs {
                let sub_path = project_path.join(subdir);
                fs::create_dir_all(&sub_path).expect("Failed to create subdirectory");
                
                // Create some files
                for j in 1..=5 {
                    fs::write(sub_path.join(format!("file_{}.txt", j)), "sample content")
                        .expect("Failed to create file");
                }
            }
            
            // Create mock git repository if specified
            if has_git {
                create_mock_git_repo(&project_path);
                total_repos += 1;
            }
            
            _total_dirs += 1;
        }
    }
    
    // Configure scanner
    let config = Config {
        search_dirs: Some(vec![SearchDirectory::new(base_path.to_path_buf(), 8)]),
        ..Default::default()
    };
    
    // Warm up run (to account for filesystem caching)
    let _ = find_repos(&config);
    
    // Actual performance benchmark
    let benchmark_start = Instant::now();
    let result = find_repos(&config);
    let scan_duration = benchmark_start.elapsed();
    
    assert!(result.is_ok(), "Repository scanning should succeed");
    let repos = result.unwrap();
    let found_repos: usize = repos.values().map(|sessions| sessions.len()).sum();
    
    // Performance assertions
    assert_eq!(found_repos, total_repos, "Should find all {} repositories, found {}", total_repos, found_repos);
    assert!(scan_duration.as_millis() < 2000, "Scan should complete in under 2000ms for realistic workspace, took {:?}", scan_duration);
}

#[test]  
fn benchmark_large_monorepo_structure() {
    // Simulate a large monorepo with many nested directories but few actual repos
    let temp = tempdir().expect("Failed to create temp dir");
    let base_path = temp.path();
    
    let mut _total_dirs = 0;
    
    // Create main repo
    let main_repo = base_path.join("large-monorepo");
    fs::create_dir_all(&main_repo).expect("Failed to create main repo");
    create_mock_git_repo(&main_repo);
    
    // Create many nested directories
    for service in 1..=20 {
        let service_dir = main_repo.join(format!("services/service-{:02}", service));
        fs::create_dir_all(&service_dir).expect("Failed to create service directory");
        _total_dirs += 1;
        
        // Create subdirectories for each service
        let subdirs = ["src", "test", "docs", "config", "scripts", "assets"];
        for subdir in subdirs {
            for level in 1..=3 {
                let nested_path = service_dir.join(format!("{}/level-{}", subdir, level));
                fs::create_dir_all(&nested_path).expect("Failed to create nested directory");
                _total_dirs += 1;
                
                // Create files
                for file_num in 1..=10 {
                    fs::write(nested_path.join(format!("file_{}.rs", file_num)), "// code")
                        .expect("Failed to create file");
                }
            }
        }
    }
    
    let config = Config {
        search_dirs: Some(vec![SearchDirectory::new(base_path.to_path_buf(), 15)]), // Deep scanning
        ..Default::default()
    };
    
    let scan_start = Instant::now();
    let result = find_repos(&config);
    let scan_duration = scan_start.elapsed();
    
    assert!(result.is_ok(), "Repository scanning should succeed");
    let repos = result.unwrap();
    let found_repos: usize = repos.values().map(|sessions| sessions.len()).sum();
    
    // Should find exactly 1 repository (the main one)
    assert_eq!(found_repos, 1, "Should find exactly 1 repository, found {}", found_repos);
    assert!(scan_duration.as_millis() < 1000, "Large monorepo scan should complete in under 1000ms, took {:?}", scan_duration);
}

#[test]
fn benchmark_performance_with_exclusions() {
    // Test performance with common build directories that should be excluded
    let temp = tempdir().expect("Failed to create temp dir");
    let base_path = temp.path();
    
    let mut expected_repos = 0;
    
    // Create projects with build directories
    for i in 0..20 {
        let project_path = base_path.join(format!("project_{}", i));
        fs::create_dir_all(&project_path).expect("Failed to create project");
        
        // Create main repo
        create_mock_git_repo(&project_path);
        expected_repos += 1;
        
        // Create build directories that should be filtered
        let build_dirs = ["node_modules", "target", "build", ".cache", "dist"];
        for build_dir in build_dirs {
            let build_path = project_path.join(build_dir);
            fs::create_dir_all(&build_path).expect("Failed to create build directory");
            
            // Create many nested directories in build dirs to slow down scanning
            for j in 0..50 {
                fs::create_dir_all(build_path.join(format!("nested_{}", j)))
                    .expect("Failed to create nested build directory");
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
    let found_repos: usize = repos.values().map(|sessions| sessions.len()).sum();
    
    // Should find all repos quickly despite build directories
    assert_eq!(found_repos, expected_repos, "Should find all {} repositories despite build directories", expected_repos);
    assert!(scan_duration.as_millis() < 1500, "Should complete quickly even with build directories, took {:?}", scan_duration);
}