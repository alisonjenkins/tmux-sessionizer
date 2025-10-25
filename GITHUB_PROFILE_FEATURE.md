---
id: GITHUB_PROFILE_FEATURE
aliases:
  - GitHub Profiles Feature
tags: []
---
# GitHub Profiles Feature

This feature allows you to easily clone and work with repositories from GitHub directly from the tmux-sessionizer picker interface.

## Configuration

Add GitHub profiles to your `~/.config/tms/config.toml`:

```toml
[[github_profiles]]
name = "personal"
credentials_command = "gh auth token"  # Command to get GitHub token
clone_root_path = "~/git/github"       # Where to clone repositories
clone_method = "SSH"                   # "SSH" or "HTTPS"

[[github_profiles]]
name = "work"
credentials_command = "cat ~/.config/gh/work_token"
clone_root_path = "~/git/work"
clone_method = "HTTPS"

# Keybindings (optional, these are the defaults)
picker_switch_mode_key = "tab"  # Key to switch between modes
picker_refresh_key = "f5"       # Key to refresh repository list
```

## State and Cache Management

The application follows XDG Base Directory specifications for managing runtime state and cache:

### State Directory
- **Location**: `$XDG_STATE_HOME/tms/` (defaults to `~/.local/state/tms/`)
- **Contents**: `state.json` - stores the currently active profile
- **Purpose**: Persists your last used mode between sessions

### Cache Directory
- **Location**: `$XDG_CACHE_HOME/tms/github/` (defaults to `~/.cache/tms/github/`)
- **Contents**: `<profile-name>.json` files containing cached repository lists
- **Purpose**: Minimizes GitHub API calls by caching repository data for 1 hour

This separation ensures that:
- Your configuration can be immutable (e.g., managed by Nix Home Manager)
- Runtime state and cache data are stored in appropriate XDG directories
- Cache can be cleared without affecting configuration or state

## Usage

1. **Launch tmux-sessionizer** as usual (`tms`)

2. **Switch between modes** using Tab (or your configured key):
   - "Local repos" - Shows your local Git repositories
   - "Github - personal" - Shows repositories from your personal profile
   - "Github - work" - Shows repositories from your work profile

3. **Current mode is displayed** in the picker title bar

4. **Refresh repository list** using F5 (or your configured key):
   - Forces a fresh fetch from GitHub API
   - Updates the cache with latest repositories

5. **Select a repository**:
   - **Local repos**: Creates/switches to tmux session as usual
   - **GitHub repos**: Clones the repository (if not already cloned) and creates a tmux session

## Features

- **XDG Compliance**: Follows XDG Base Directory specification for state and cache
- **Caching**: GitHub repositories are cached for 1 hour to minimize API calls
- **Mode persistence**: Your last used mode is remembered between sessions
- **Clone management**: Repositories are only cloned once; subsequent selections reuse the existing clone
- **Flexible authentication**: Use any command to provide GitHub tokens (gh CLI, environment variables, files, etc.)
- **Clone methods**: Choose between SSH and HTTPS cloning per profile

## Credentials Commands

The `credentials_command` can be any shell command that outputs a GitHub personal access token:

- `gh auth token` - Using GitHub CLI
- `echo $GITHUB_TOKEN` - From environment variable
- `cat ~/.config/gh/token` - From a file
- `op item get "GitHub Token" --fields token` - Using 1Password CLI
- `pass show github/token` - Using pass password manager

## Directory Structure

With the example configuration above, your directories would be organized as:
```
~/.config/tms/config.toml       # Configuration (immutable)
~/.local/state/tms/state.json   # Active profile state
~/.cache/tms/github/            # Cached repository lists
├── personal.json
└── work.json

~/git/
├── github/           # Personal repositories
│   ├── repo1/
│   └── repo2/
├── work/             # Work repositories
│   ├── project-a/
│   └── project-b/
└── local-repos/      # Your existing local repositories
    ├── existing1/
    └── existing2/
```

## Error Handling

- If a credentials command fails, the profile is skipped with a warning
- Network errors during repository fetching are displayed to the user
- Clone failures are reported without interrupting the workflow
- Cache corruption is handled gracefully by forcing a refresh

## Environment Variables

- `XDG_STATE_HOME`: Override default state directory (`~/.local/state`)
- `XDG_CACHE_HOME`: Override default cache directory (`~/.cache`)
