# Nix Build Fixes for Streaming Implementation

## Issues Fixed

The Nix build was failing due to two main issues related to the new streaming implementation:

### 1. **TTY/Terminal Initialization Issue**
**Problem**: The streaming picker was trying to initialize a ratatui terminal in the Nix sandbox environment where no TTY is available, causing a panic.

**Root Cause**: `ratatui::init()` was being called unconditionally, which fails in headless environments.

**Solution**: Added TTY availability check using `std::io::IsTerminal` before attempting terminal initialization:
```rust
use std::io::IsTerminal;
if !std::io::stdout().is_terminal() {
    return Err(TmsError::TuiError(
        "Cannot initialize terminal (no TTY available)...".to_string()
    ).into());
}
```

### 2. **Config Error Handling Flow**
**Problem**: Configuration errors were not being caught early enough. The main function would proceed to streaming initialization even with invalid configs, causing errors later in the process.

**Root Cause**: Config validation (`config.search_dirs()`) was only happening during streaming, not during initial config loading.

**Solution**: Added explicit config validation immediately after config loading in main:
```rust
// Validate the config early to catch configuration errors before TTY checks
if let Err(e) = config.search_dirs() {
    eprintln!("Error: {}", e);
    std::process::exit(1);
}
```

## Code Changes

### Modified Files
- `src/main.rs`: Added early config validation and TTY checks
- `src/picker/mod.rs`: Added TTY availability check before terminal initialization

### Key Changes
1. **Early Config Validation**: Validate config immediately after loading to catch errors like missing search paths
2. **TTY Detection**: Check for terminal availability before initializing ratatui
3. **Proper Error Propagation**: Ensure config errors exit with code 1 as expected by tests
4. **Graceful Error Handling**: Provide clear error messages for different failure scenarios

## Test Results

### Before Fix
```
error: test failed, to rerun pass `--test cli`
error: builder for '/nix/store/...-tmux-sessionizer-0.5.0.drv' failed with exit code 101
```

### After Fix
```
✅ All tests pass
✅ Nix build succeeds
✅ Proper error handling with exit code 1
✅ Clear error messages for missing config
```

## Verification

The fixes were verified by:
1. **Local Testing**: All tests pass including `tms_fails_with_missing_config`
2. **Nix Build**: `nix build` completes successfully  
3. **Error Handling**: Proper error messages and exit codes
4. **Functionality**: Streaming functionality works correctly when TTY is available

## Impact

These fixes ensure that:
- ✅ **Nix builds work** in sandboxed environments
- ✅ **Tests pass** in CI/CD pipelines  
- ✅ **Error handling** is robust and predictable
- ✅ **Streaming functionality** remains fully functional
- ✅ **Backward compatibility** is maintained

The streaming implementation now works correctly in all environments while providing appropriate error handling for edge cases.