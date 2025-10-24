use error_stack::ResultExt;

use crate::{Result, TmsError};

pub trait DirtyUtf8Path {
    fn to_string(&self) -> Result<String>;
}

/// Normalize a path string by removing duplicate slashes
fn normalize_path(path: &str) -> String {
    let bytes = path.as_bytes();
    let mut result = Vec::with_capacity(bytes.len());
    let mut prev_slash = false;

    for &byte in bytes {
        if byte == b'/' {
            if !prev_slash || result.is_empty() {
                // Keep first slash or non-consecutive slashes
                result.push(byte);
            }
            prev_slash = true;
        } else {
            result.push(byte);
            prev_slash = false;
        }
    }

    // Safety: We only modified slashes, so if input was valid UTF-8, output is too
    String::from_utf8(result).unwrap()
}

impl DirtyUtf8Path for std::path::PathBuf {
    fn to_string(&self) -> Result<String> {
        let path_str = self
            .to_str()
            .ok_or(TmsError::NonUtf8Path)
            .attach_printable("Not a valid utf8 path")?
            .to_string();
        Ok(normalize_path(&path_str))
    }
}
impl DirtyUtf8Path for std::path::Path {
    fn to_string(&self) -> Result<String> {
        let path_str = self
            .to_str()
            .ok_or(TmsError::NonUtf8Path)
            .attach_printable("Not a valid utf8 path")?
            .to_string();
        Ok(normalize_path(&path_str))
    }
}
impl DirtyUtf8Path for std::ffi::OsStr {
    fn to_string(&self) -> Result<String> {
        let path_str = self
            .to_str()
            .ok_or(TmsError::NonUtf8Path)
            .attach_printable("Not a valid utf8 path")?
            .to_string();
        Ok(normalize_path(&path_str))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_path_removes_duplicate_slashes() {
        assert_eq!(normalize_path("//home/user/git/repo"), "/home/user/git/repo");
        assert_eq!(normalize_path("/home//user/git/repo"), "/home/user/git/repo");
        assert_eq!(normalize_path("/home/user//git//repo"), "/home/user/git/repo");
        assert_eq!(normalize_path("///home/user/git/repo"), "/home/user/git/repo");
    }

    #[test]
    fn test_normalize_path_preserves_single_slashes() {
        assert_eq!(normalize_path("/home/user/git/repo"), "/home/user/git/repo");
        assert_eq!(normalize_path("/"), "/");
    }

    #[test]
    fn test_normalize_path_preserves_relative_paths() {
        assert_eq!(normalize_path("home/user/git/repo"), "home/user/git/repo");
        assert_eq!(normalize_path("./git/repo"), "./git/repo");
    }
}

