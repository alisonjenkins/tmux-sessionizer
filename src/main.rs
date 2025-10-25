use std::env;

use clap::{CommandFactory, Parser};
use clap_complete::CompleteEnv;
use error_stack::Report;

use tms::{
    cli::{Cli, SubCommandGiven},
    error::{Result, Suggestion},
    get_single_selection_streaming,
    session::create_sessions_streaming,
    tmux::Tmux,
};

#[tokio::main]
async fn main() -> Result<()> {
    // Install debug hooks for formatting of error handling
    Report::install_debug_hook::<Suggestion>(|value, context| {
        context.push_body(format!("{value}"));
    });
    #[cfg(any(not(debug_assertions), test))]
    Report::install_debug_hook::<std::panic::Location>(|_value, _context| {});

    let bin_name = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.file_name().map(|exe| exe.to_string_lossy().to_string()))
        .unwrap_or("tms".into());
    match CompleteEnv::with_factory(Cli::command)
        .bin(bin_name)
        .try_complete(env::args_os(), None)
    {
        Ok(true) => return Ok(()),
        Err(e) => {
            panic!("failed to generate completions: {e}");
        }
        Ok(false) => {}
    };

    // Use CLAP to parse the command line arguments
    let cli_args = Cli::parse();

    let tmux = Tmux::default();

    // Handle sub-commands first, which includes config validation
    // If this fails, the error should be properly propagated without reaching streaming code
    let mut config = match cli_args.handle_sub_commands(&tmux).await {
        Ok(SubCommandGiven::Yes) => return Ok(()),
        Ok(SubCommandGiven::No(config)) => *config, // continue with valid config
        Err(e) => {
            // Config error - this should exit with code 1 as expected by tests
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    };

    // Validate the config early to catch configuration errors before TTY checks
    if let Err(e) = config.search_dirs() {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }

    // Only check for TTY after we have a valid config
    // Check if stdout is a TTY to avoid issues in sandboxed environments
    use std::io::IsTerminal;
    if !std::io::stdout().is_terminal() {
        eprintln!("Error: No terminal available. This command requires an interactive terminal.");
        std::process::exit(1);
    }

    // Now it's safe to proceed with streaming (trace logs are suppressed by default)
    let (receiver, sessions_map) = match create_sessions_streaming(&config).await {
        Ok((receiver, sessions_map)) => (receiver, sessions_map),
        Err(e) => {
            eprintln!("Error creating session stream: {}", e);
            std::process::exit(1);
        }
    };

    let selected_str = match get_single_selection_streaming(
        None, // No preview for now - we can add this later
        &config,
        &tmux,
        receiver,
    ).await {
        Ok(Some(str)) => str,
        Ok(None) => return Ok(()), // User cancelled
        Err(e) => {
            eprintln!("Error in selection: {}", e);
            std::process::exit(1);
        }
    };

    // Look up the actual session object to get proper path handling
    match sessions_map.lock() {
        Ok(sessions) => {
            // Check if this is a GitHub repository selection
            if selected_str.starts_with("github:") {
                let repo_path = &selected_str[7..]; // Remove "github:" prefix
                let repo_name = std::path::Path::new(repo_path)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("unknown")
                    .to_string();
                
                // Create a GitHub session
                let github_session = tms::session::Session::new(
                    repo_name.clone(),
                    tms::session::SessionType::GitHub {
                        path: std::path::PathBuf::from(repo_path),
                        repo_name: repo_name.clone(),
                    }
                );
                
                // Update frecency data for this session
                config.update_session_frecency(&repo_name);
                
                // Save the config with updated frecency data (ignore errors to not interrupt workflow)
                let _ = config.save();
                
                // Switch to the GitHub session
                if let Err(e) = github_session.switch_to(&tmux, &config) {
                    eprintln!("Error switching to GitHub session: {}", e);
                    std::process::exit(1);
                }
            } else if let Some(session) = sessions.get(&selected_str) {
                // Update frecency data for this session
                config.update_session_frecency(&session.name);
                
                // Save the config with updated frecency data (ignore errors to not interrupt workflow)
                let _ = config.save();
                
                // Use the proper session.switch_to method which handles paths correctly
                if let Err(e) = session.switch_to(&tmux, &config) {
                    eprintln!("Error switching to session: {}", e);
                    std::process::exit(1);
                }
            } else {
                // Fallback: if we can't find the session, try to create it as a simple session
                // This shouldn't happen in normal operation
                eprintln!("Warning: Could not find session data for '{}', creating simple session", selected_str);
                if tmux.session_exists(&selected_str) {
                    tmux.switch_to_session(&selected_str);
                } else {
                    tmux.new_session(Some(&selected_str), None);
                    tmux.switch_to_session(&selected_str);
                }
                
                // Still track this session access for frecency
                config.update_session_frecency(&selected_str);
                let _ = config.save();
            }
        }
        Err(e) => {
            eprintln!("Error accessing session data: {}", e);
            std::process::exit(1);
        }
    }

    Ok(())
}
