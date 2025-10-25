# GitHub Profiles Feature - Implementation Summary

## Overview

Successfully implemented a comprehensive GitHub profiles feature for tmux-sessionizer with proper XDG Base Directory compliance that allows users to:
- Switch between local repositories and multiple GitHub profiles 
- Clone and manage repositories directly from GitHub
- Persist mode preferences between sessions using XDG state directories
- Cache GitHub API responses in XDG cache directories
- Maintain immutable configurations (compatible with Nix Home Manager)

## Files Modified/Added

### New Files
- `src/github.rs` - GitHub API client and repository management
- `src/state.rs` - XDG-compliant state and cache management
- `GITHUB_PROFILE_FEATURE.md` - User documentation
- `example_github_config.toml` - Example configuration
- `IMPLEMENTATION_SUMMARY.md` - This summary

### Modified Files
- `src/configs.rs` - Added GitHub profile configuration structures, removed active_profile
- `src/picker/mod.rs` - Extended picker to support multiple modes with state management
- `src/keymap.rs` - Added new picker actions (SwitchMode, Refresh)
- `src/session.rs` - Added GitHub session type and handling
- `src/main.rs` - Updated to handle GitHub repository selections
- `src/lib.rs` - Updated picker constructors and added state module
- `Cargo.toml` - Added HTTP client dependencies
- `tests/cli.rs` - Fixed test to exclude removed config fields

## Key Features Implemented

### 1. XDG Base Directory Compliance
- **State Directory**: `$XDG_STATE_HOME/tms/` (defaults to `~/.local/state/tms/`)
  - Contains `state.json` with current active profile
  - Persists picker mode between sessions
- **Cache Directory**: `$XDG_CACHE_HOME/tms/github/` (defaults to `~/.cache/tms/github/`)
  - Contains `<profile-name>.json` files with cached repository lists
  - 1-hour cache expiration to minimize API calls
- **Configuration Separation**: Runtime state separated from immutable configuration

### 2. Configuration System
- **GitHubProfile** struct with name, credentials command, clone path, and clone method
- **Configurable keybinds** for mode switching (default: Tab) and refresh (default: F5)
- **Removed active_profile** from configuration (now in state directory)
- **Flexible authentication** - any shell command that outputs a GitHub token

### 3. State Management Architecture
- **StateManager** struct handling XDG directories and JSON persistence
- **Environment variable support** for `XDG_STATE_HOME` and `XDG_CACHE_HOME`
- **Fallback to XDG defaults** when environment variables are not set
- **Thread-safe state operations** with proper error handling

### 4. Multi-Mode Picker Interface
- **PickerMode enum** to represent different modes (Local, GitHub profiles)
- **Mode switching** with Tab key (configurable)
- **Visual mode indicator** in picker title showing current mode
- **Automatic state persistence** when switching modes or making selections

### 5. GitHub Integration
- **GitHubClient** using StateManager for cache directory paths
- **Repository caching** with 1-hour expiration in XDG cache directory
- **Async repository fetching** with pagination support
- **Clone management** - repositories are only cloned once per profile
- **Support for SSH and HTTPS** cloning methods

### 6. Session Management
- **New SessionType::GitHub** variant for cloned repositories
- **Proper path handling** for GitHub repositories
- **Seamless integration** with existing tmux session creation
- **Frecency tracking** for GitHub repositories

### 7. Error Handling
- **Graceful credential failures** - profiles are skipped with warnings
- **Network error handling** - API failures are reported without crashing
- **Clone error management** - failed clones don't interrupt workflow
- **Cache corruption handling** - automatic fallback to fresh data

## Technical Details

### XDG Base Directory Implementation
```rust
fn get_xdg_state_home() -> Result<PathBuf> {
    if let Ok(path) = std::env::var("XDG_STATE_HOME") {
        Ok(PathBuf::from(path))
    } else if let Some(home) = dirs::home_dir() {
        Ok(home.join(".local/state"))
    } else {
        Err(TmsError::IoError.into())
    }
}

fn get_xdg_cache_home() -> Result<PathBuf> {
    if let Ok(path) = std::env::var("XDG_CACHE_HOME") {
        Ok(PathBuf::from(path))
    } else if let Some(home) = dirs::home_dir() {
        Ok(home.join(".cache"))
    } else {
        Err(TmsError::IoError.into())
    }
}
```

### State Management
- **AppState struct**: JSON-serializable state container
- **Atomic operations**: State updates are atomic (write to temp file, then move)
- **Directory creation**: Automatically creates XDG directories if they don't exist
- **Thread safety**: Mutex-based test synchronization to prevent race conditions

### Dependencies Added
- `reqwest` (0.12) - HTTP client for GitHub API
- `serde_json` (1.0) - JSON parsing for API responses and state management

### Cache Strategy
- **Cache location**: `$XDG_CACHE_HOME/tms/github/` with per-profile JSON files
- **Cache format**: JSON with repository metadata and timestamp
- **Cache duration**: 1 hour (configurable via CACHE_DURATION_SECONDS)
- **Cache invalidation**: Explicit refresh via F5 key or automatic on expiration

## Configuration vs State vs Cache

### Configuration (`~/.config/tms/config.toml`)
- **Immutable settings** that define how the application behaves
- GitHub profile definitions, keybindings, search paths
- Can be managed by configuration management tools (Nix, etc.)
- Version controlled and shared across machines

### State (`$XDG_STATE_HOME/tms/state.json`)
- **Runtime preferences** that change during use
- Currently active picker mode/profile
- Session-specific data that persists between runs
- Machine-specific and not version controlled

### Cache (`$XDG_CACHE_HOME/tms/github/*.json`)
- **Performance optimization** data that can be regenerated
- GitHub repository lists with timestamps
- Can be safely deleted without losing functionality
- Automatically managed with expiration policies

## Usage Flow

1. User launches `tms` as usual
2. StateManager loads active profile from XDG state directory
3. Picker initializes in the last used mode (or "Local repos" by default)
4. User can press Tab to cycle through available modes
5. When switching to GitHub mode, cached repositories are loaded from XDG cache directory
6. F5 refreshes repository list and updates cache
7. Selecting a GitHub repository:
   - Clones the repo to the configured path (if not already present)
   - Creates a new tmux session in the cloned directory
   - Updates state to remember this profile selection

## Benefits of XDG Compliance

1. **Immutable Configurations**: Configuration files can be read-only or managed by tools like Nix
2. **Proper Separation of Concerns**: State, cache, and configuration are in appropriate locations
3. **Standards Compliance**: Follows established Linux/Unix conventions
4. **Better User Experience**: Users can easily find, backup, or clear specific types of data
5. **System Integration**: Works well with system cleanup tools and backup strategies

## Testing

- **Comprehensive test coverage** including XDG directory handling
- **Thread-safe test isolation** using mutex-based synchronization
- **Environment variable testing** with proper cleanup and restoration
- **All existing tests pass** - no regressions in existing functionality
- **Temporary directory management** for safe test execution

## Future Enhancements

- Support for custom cache expiration per profile
- GitHub Enterprise server support
- GitLab/Bitbucket integration using similar patterns
- Cache compression for large repository lists
- Metrics and usage analytics stored in state directory