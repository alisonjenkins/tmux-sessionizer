use tms::configs::{LocalCachedSession, LocalSessionType, SearchDirectory};
use tms::perf_json;
use std::path::PathBuf;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct LargeTestData {
    repos: Vec<LocalCachedSession>,
    directories: Vec<SearchDirectory>,
    metadata: std::collections::HashMap<String, String>,
}

impl LargeTestData {
    fn generate_large_dataset(size: usize) -> Self {
        let repos = (0..size)
            .map(|i| LocalCachedSession {
                name: format!("repo_{}", i),
                path: format!("/path/to/repo_{}", i),
                session_type: match i % 3 {
                    0 => LocalSessionType::Git,
                    1 => LocalSessionType::Jujutsu,
                    _ => LocalSessionType::Bookmark,
                },
            })
            .collect();

        let directories = (0..size / 10)
            .map(|i| SearchDirectory::new(PathBuf::from(format!("/search/dir/{}", i)), 10))
            .collect();

        let metadata = (0..size / 5)
            .map(|i| (format!("key_{}", i), format!("value_{}", i)))
            .collect();

        LargeTestData {
            repos,
            directories,
            metadata,
        }
    }
}

#[test]
fn test_performance_comparison_small_data() {
    let data = LargeTestData::generate_large_dataset(100);
    
    // Test serialization performance
    let json_simd = perf_json::to_string_pretty(&data).unwrap();
    let json_serde = serde_json::to_string_pretty(&data).unwrap();
    
    // Both should produce valid JSON
    assert!(json_simd.len() > 100);
    assert!(json_serde.len() > 100);
    
    // Test deserialization performance  
    let _deserialized_simd: LargeTestData = perf_json::from_str(&json_simd).unwrap();
    let _deserialized_serde: LargeTestData = serde_json::from_str(&json_serde).unwrap();
}

#[test] 
fn test_performance_comparison_large_data() {
    let data = LargeTestData::generate_large_dataset(1000);
    
    // Test with larger dataset
    let json = perf_json::to_string_pretty(&data).unwrap();
    let _deserialized: LargeTestData = perf_json::from_str(&json).unwrap();
    
    // Verify the data is preserved correctly
    assert_eq!(data.repos.len(), _deserialized.repos.len());
    assert_eq!(data.directories.len(), _deserialized.directories.len());
    assert_eq!(data.metadata.len(), _deserialized.metadata.len());
}

#[test]
fn benchmark_serialization_performance() {
    let data = LargeTestData::generate_large_dataset(500);
    let iterations = 10;
    
    let (simd_duration, serde_duration) = perf_json::bench::benchmark_serialization(&data, iterations);
    
    println!("Serialization benchmark (iterations: {}):", iterations);
    println!("  SIMD JSON: {:?}", simd_duration);
    println!("  Serde JSON: {:?}", serde_duration);
    
    // Performance improvement varies by platform and data size
    // We don't assert specific performance requirements in tests
    assert!(simd_duration.as_nanos() > 0);
    assert!(serde_duration.as_nanos() > 0);
}

#[test]
fn benchmark_deserialization_performance() {
    let data = LargeTestData::generate_large_dataset(500);
    let json = perf_json::to_string_pretty(&data).unwrap();
    let iterations = 10;
    
    let (simd_duration, serde_duration) = perf_json::bench::benchmark_deserialization::<LargeTestData>(&json, iterations);
    
    println!("Deserialization benchmark (iterations: {}):", iterations);
    println!("  SIMD JSON: {:?}", simd_duration);  
    println!("  Serde JSON: {:?}", serde_duration);
    
    // Performance results vary by platform
    assert!(simd_duration.as_nanos() > 0);
    assert!(serde_duration.as_nanos() > 0);
}

#[tokio::test]
async fn test_file_operations_performance() {
    use tempfile::NamedTempFile;
    
    let data = LargeTestData::generate_large_dataset(200);
    let temp_file = NamedTempFile::new().unwrap();
    let path = temp_file.path();
    
    // Test file write performance
    let start = std::time::Instant::now();
    perf_json::to_file(path, &data).await.unwrap();
    let write_duration = start.elapsed();
    
    // Test file read performance
    let start = std::time::Instant::now();
    let _loaded: LargeTestData = perf_json::from_file(path).await.unwrap();
    let read_duration = start.elapsed();
    
    println!("File operations performance:");
    println!("  Write: {:?}", write_duration);
    println!("  Read: {:?}", read_duration);
    
    assert!(write_duration.as_nanos() > 0);
    assert!(read_duration.as_nanos() > 0);
}