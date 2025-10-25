pub mod cli;
pub mod configs;
pub mod dirty_paths;
pub mod error;
pub mod github;
pub mod keymap;
pub mod local_cache;
pub mod marks;
pub mod perf_json;
pub mod picker;
pub mod repos;
pub mod session;
pub mod state;
pub mod tmux;

use configs::Config;
use std::process;
use tokio::sync::mpsc;

use crate::{
    error::{Result, TmsError},
    picker::{Picker, Preview},
    tmux::Tmux,
};

pub fn execute_command(command: &str, args: Vec<String>) -> process::Output {
    process::Command::new(command)
        .args(args)
        .stdin(process::Stdio::inherit())
        .output()
        .unwrap_or_else(|_| panic!("Failed to execute command `{command}`"))
}

pub async fn get_single_selection(
    list: &[String],
    preview: Option<Preview>,
    config: &Config,
    tmux: &Tmux,
) -> Result<Option<String>> {
    let mut picker = Picker::new(
        list,
        preview,
        config.shortcuts.as_ref(),
        config.input_position.unwrap_or_default(),
        tmux,
        config,
    )
    .set_colors(config.picker_colors.as_ref());

    picker.run().await
}

/// Streaming version that shows a picker and starts scanning in the background
pub async fn get_single_selection_streaming(
    preview: Option<Preview>,
    config: &Config,
    tmux: &Tmux,
    receiver: mpsc::UnboundedReceiver<String>,
) -> Result<Option<String>> {
    let mut picker = Picker::new_streaming(
        preview,
        config.shortcuts.as_ref(),
        config.input_position.unwrap_or_default(),
        tmux,
        receiver,
        config,
    )
    .set_colors(config.picker_colors.as_ref());

    picker.run().await
}
