//! Performance-optimized JSON operations using simd_json with fallback to serde_json
//! 
//! This module provides high-performance JSON serialization and deserialization
//! by leveraging SIMD instructions when available, with graceful fallback to
//! standard serde_json for compatibility.

use serde::{Deserialize, Serialize};
use std::io;

/// Performance-optimized JSON serialization
pub fn to_string_pretty<T>(value: &T) -> Result<String, JsonError>
where
    T: ?Sized + Serialize,
{
    // Use simd_json for serialization when possible
    match simd_json::to_string_pretty(value) {
        Ok(json) => Ok(json),
        Err(simd_err) => {
            // Fallback to serde_json if simd_json fails
            serde_json::to_string_pretty(value)
                .map_err(|serde_err| JsonError::SerializationFailed {
                    simd_error: simd_err.to_string(),
                    serde_error: serde_err.to_string(),
                })
        }
    }
}

/// Performance-optimized JSON deserialization from string
pub fn from_str<T>(s: &str) -> Result<T, JsonError>
where
    T: for<'a> Deserialize<'a>,
{
    // simd_json requires mutable data, so we need to clone the string
    let mut data = s.to_string();
    
    // simd_json functions are unsafe, so we need to wrap them
    match unsafe { simd_json::from_str(&mut data) } {
        Ok(value) => Ok(value),
        Err(simd_err) => {
            // Fallback to serde_json if simd_json fails
            serde_json::from_str(s)
                .map_err(|serde_err| JsonError::DeserializationFailed {
                    simd_error: simd_err.to_string(),
                    serde_error: serde_err.to_string(),
                })
        }
    }
}

/// Performance-optimized JSON deserialization from bytes
pub fn from_slice<T>(data: &mut [u8]) -> Result<T, JsonError>
where
    T: for<'a> Deserialize<'a>,
{
    match unsafe { simd_json::from_slice(data) } {
        Ok(value) => Ok(value),
        Err(simd_err) => {
            // Fallback to serde_json
            let s = std::str::from_utf8(data)
                .map_err(|utf8_err| JsonError::InvalidUtf8(utf8_err.to_string()))?;
            
            serde_json::from_str(s)
                .map_err(|serde_err| JsonError::DeserializationFailed {
                    simd_error: simd_err.to_string(),
                    serde_error: serde_err.to_string(),
                })
        }
    }
}

/// Performance-optimized JSON deserialization from file
pub async fn from_file<T>(path: &std::path::Path) -> Result<T, JsonError>
where
    T: for<'a> Deserialize<'a>,
{
    let mut contents = tokio::fs::read(path).await
        .map_err(JsonError::IoError)?;
    
    from_slice(&mut contents)
}

/// Performance-optimized JSON serialization to file
pub async fn to_file<T>(path: &std::path::Path, value: &T) -> Result<(), JsonError>
where
    T: ?Sized + Serialize,
{
    let json_string = to_string_pretty(value)?;
    
    tokio::fs::write(path, json_string).await
        .map_err(JsonError::IoError)
}

/// Errors that can occur during JSON operations
#[derive(Debug, thiserror::Error)]
pub enum JsonError {
    #[error("Serialization failed - SIMD: {simd_error}, Serde: {serde_error}")]
    SerializationFailed {
        simd_error: String,
        serde_error: String,
    },
    #[error("Deserialization failed - SIMD: {simd_error}, Serde: {serde_error}")]
    DeserializationFailed {
        simd_error: String,
        serde_error: String,
    },
    #[error("Invalid UTF-8: {0}")]
    InvalidUtf8(String),
    #[error("IO error: {0}")]
    IoError(#[from] io::Error),
}

/// Benchmark JSON operations performance
pub mod bench {
    use super::*;
    use std::time::Instant;
    
    pub fn benchmark_serialization<T>(data: &T, iterations: usize) -> (std::time::Duration, std::time::Duration)
    where
        T: ?Sized + Serialize,
    {
        // Benchmark simd_json
        let start = Instant::now();
        for _ in 0..iterations {
            let _ = simd_json::to_string_pretty(data).unwrap();
        }
        let simd_duration = start.elapsed();
        
        // Benchmark serde_json
        let start = Instant::now();
        for _ in 0..iterations {
            let _ = serde_json::to_string_pretty(data).unwrap();
        }
        let serde_duration = start.elapsed();
        
        (simd_duration, serde_duration)
    }
    
    pub fn benchmark_deserialization<T>(json: &str, iterations: usize) -> (std::time::Duration, std::time::Duration)
    where
        T: for<'a> Deserialize<'a>,
    {
        // Benchmark simd_json
        let start = Instant::now();
        for _ in 0..iterations {
            let mut data = json.to_string();
            let _: Result<T, _> = unsafe { simd_json::from_str(&mut data) };
        }
        let simd_duration = start.elapsed();
        
        // Benchmark serde_json
        let start = Instant::now();
        for _ in 0..iterations {
            let _: Result<T, _> = serde_json::from_str(json);
        }
        let serde_duration = start.elapsed();
        
        (simd_duration, serde_duration)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    struct TestData {
        name: String,
        count: u32,
        items: Vec<String>,
    }

    #[test]
    fn test_serialization_deserialization() {
        let test_data = TestData {
            name: "test".to_string(),
            count: 42,
            items: vec!["item1".to_string(), "item2".to_string()],
        };

        // Test serialization
        let json = to_string_pretty(&test_data).unwrap();
        assert!(json.contains("\"name\": \"test\""));
        assert!(json.contains("\"count\": 42"));

        // Test deserialization
        let deserialized: TestData = from_str(&json).unwrap();
        assert_eq!(deserialized, test_data);
    }

    #[test]
    fn test_from_slice() {
        let test_data = TestData {
            name: "slice_test".to_string(),
            count: 123,
            items: vec!["a".to_string(), "b".to_string()],
        };

        let json = to_string_pretty(&test_data).unwrap();
        let mut bytes = json.into_bytes();
        
        let deserialized: TestData = from_slice(&mut bytes).unwrap();
        assert_eq!(deserialized, test_data);
    }

    #[tokio::test]
    async fn test_file_operations() {
        use tempfile::NamedTempFile;
        
        let test_data = TestData {
            name: "file_test".to_string(),
            count: 456,
            items: vec!["file1".to_string(), "file2".to_string()],
        };

        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path();

        // Test writing to file
        to_file(path, &test_data).await.unwrap();

        // Test reading from file
        let deserialized: TestData = from_file(path).await.unwrap();
        assert_eq!(deserialized, test_data);
    }

    #[test]
    fn test_error_handling() {
        // Test invalid JSON
        let invalid_json = "{ invalid json }";
        let result: Result<TestData, _> = from_str(invalid_json);
        assert!(result.is_err());
        
        // The error should contain information from both parsers
        let error = result.unwrap_err();
        match error {
            JsonError::DeserializationFailed { simd_error: _, serde_error: _ } => {
                // Expected error type
            }
            _ => panic!("Unexpected error type"),
        }
    }
}