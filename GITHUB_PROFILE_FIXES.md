# GitHub Profile Configuration Fixes

This document describes the fixes applied to resolve two critical bugs in the GitHub profiles functionality, plus a major UX improvement for mode switching.

## Issues Fixed

### 1. Profile Switching Bug - FIXED ✅
**Problem**: The profile switching did not allow swapping back to searching local git repos. Once in a GitHub profile mode, users were stuck and couldn't return to local repository browsing.

**Root Cause**: The picker's mode switching logic cycled through modes but wasn't intuitive and didn't clearly show available options.

**Solution**: 
- **Completely redesigned mode switching** with a dedicated fuzzy-searchable mode picker
- When Tab is pressed, shows a separate picker with all available modes: "Local repos", "GitHub - profile1", "GitHub - profile2", etc.
- Users can fuzzy search through modes and select the desired one
- Escape returns to the original picker without changing modes
- Current mode is highlighted as the initial selection
- Much more intuitive and discoverable UX

### 2. GitHub Credentials Command Bug - FIXED ✅
**Problem**: The GitHub profiles were running their credential command every time repositories were accessed, even when using cached data. This was inefficient and could cause unnecessary API calls or authentication prompts.

**Root Cause**: The `get_repositories()` method always called `get_access_token()` before checking if cached data could be used.

**Solution**:
- Restructured the credential flow to only call `get_access_token()` when actually needed (cache miss or force refresh)
- Added proper cache validation before attempting to fetch fresh data
- Made cache duration configurable instead of hard-coded

### 3. Hard-coded Cache Duration - FIXED ✅
**Problem**: Cache duration was hard-coded to 1 hour, which was too frequent for most use cases.

**Solution**:
- Added `github_cache_duration_hours` configuration option
- Changed default from 1 hour to 30 days (720 hours)
- Made the cache duration configurable per user preference

## Major UX Improvement: Interactive Mode Picker

### New Mode Switching Behavior

**Before**: Tab key cycled through modes silently with no indication of available options.

**After**: Tab key opens a **fuzzy-searchable mode picker** with the following features:

- 🔍 **Fuzzy Search**: Type to filter available modes
- 📋 **Clear Options**: Shows all available modes with descriptive names
- 🎯 **Smart Selection**: Current mode is pre-selected
- ⌨️ **Intuitive Controls**: 
  - Arrow keys or type to navigate
  - Enter to select mode
  - Escape to cancel (returns to original picker unchanged)
- 💾 **State Persistence**: Selected mode is remembered between sessions

### Mode Picker Interface

```
┌─ Select Mode (3/3) ─────────────────────┐
│ > Local repos                           │
│   GitHub - work                         │  
│   GitHub - personal                     │
│                                         │
│                                         │
└─────────────────────────────────────────┘
Filter: _
```

Users can:
- Type "local" to quickly find local repos
- Type "work" to find work GitHub profile  
- Use arrow keys to navigate
- Press Enter to switch to selected mode
- Press Escape to stay in current mode

## Configuration Changes

### New Configuration Options

```toml
# Cache duration for GitHub repository data (in hours)
# Default: 720 hours (30 days)
github_cache_duration_hours = 168  # 1 week example

# Mode switching keys (defaults shown)
picker_switch_mode_key = "tab"  # Opens mode picker
picker_refresh_key = "f5"       # Forces refresh of current mode

# Existing profile configuration remains the same
[[github_profiles]]
name = "work"
credentials_command = "gh auth token --hostname github.com"  
clone_root_path = "~/work/github"
clone_method = "SSH"
```

### API Changes

**GitHubClient::get_repositories()**
- **Before**: `get_repositories(&self, profile: &GitHubProfile, force_refresh: bool)`
- **After**: `get_repositories(&self, profile: &GitHubProfile, config: &Config, force_refresh: bool)`

Added `Config` parameter to access the configurable cache duration.

## Usage Improvements

1. **Efficient Caching**: Credentials are only fetched when necessary
2. **Intuitive Mode Switching**: Tab opens a clear, searchable mode picker  
3. **Force Refresh**: F5 key forces refresh of current mode (runs credentials for GitHub profiles)
4. **State Persistence**: Active profile is remembered between sessions
5. **Better Discoverability**: Users can easily see and search all available modes

## Technical Implementation

### Mode Picker Architecture

The new mode picker is implemented as a lightweight, non-recursive picker that:
- Runs in its own terminal session to avoid recursion issues
- Uses simple filtering logic for fuzzy search
- Maintains the same visual style as the main picker
- Handles all standard navigation keys
- Properly restores the original picker state on cancel

### Error Handling

- Graceful handling of terminal initialization failures
- Proper error propagation without breaking the main picker
- Fallback behavior if mode switching fails

## Testing

All existing tests continue to pass, ensuring backward compatibility while adding the new functionality.

## Files Modified

- `src/configs.rs`: Added `github_cache_duration_hours` configuration option
- `src/github.rs`: Fixed credential command usage and made cache duration configurable  
- `src/picker/mod.rs`: **Major rewrite** of mode switching with new interactive mode picker
- `tests/cli.rs`: Updated test to include new config field

## Backward Compatibility

All changes are backward compatible:
- Existing configurations without `github_cache_duration_hours` will use the new default (30 days)
- Existing GitHub profile configurations continue to work unchanged
- Tab key behavior is enhanced but maintains the same core functionality
- API changes are internal and don't affect user configuration

## User Experience

The new mode switching provides a **dramatically improved** user experience:

✅ **Clear and Discoverable**: Users can see all available modes at a glance  
✅ **Fuzzy Search**: Quickly find modes by typing partial names  
✅ **Intuitive Navigation**: Standard picker controls (arrows, enter, escape)  
✅ **Visual Feedback**: Current mode highlighted, counts shown  
✅ **Non-disruptive**: Escape returns to original state  
✅ **Fast**: No unnecessary API calls or credential runs  

This transforms mode switching from a confusing cycle-through mechanism to an intuitive, discoverable interface that makes it obvious how to get back to local repository searching.