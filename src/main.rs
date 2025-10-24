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
    let config = match cli_args.handle_sub_commands(&tmux) {
        Ok(SubCommandGiven::Yes) => return Ok(()),
        Ok(SubCommandGiven::No(config)) => config, // continue with valid config
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
    let receiver = match create_sessions_streaming(&config).await {
        Ok(receiver) => receiver,
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
    ) {
        Ok(Some(str)) => str,
        Ok(None) => return Ok(()), // User cancelled
        Err(e) => {
            eprintln!("Error in selection: {}", e);
            std::process::exit(1);
        }
    };

    // For now, create a simple session from the selected string
    // TODO: We need to store the actual sessions for selection
    if tmux.session_exists(&selected_str) {
        tmux.switch_to_session(&selected_str);
    } else {
        // Create a new session - this is a simplified version
        // In the full implementation, we'd need to track the actual session objects
        tmux.new_session(Some(&selected_str), None);
        tmux.switch_to_session(&selected_str);
    }

    Ok(())
}
