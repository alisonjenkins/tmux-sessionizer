mod preview;

use std::{process, rc::Rc, sync::Arc};

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use nucleo::{
    pattern::{CaseMatching, Normalization},
    Nucleo,
};
use preview::PreviewWidget;
use ratatui::{
    layout::{self, Constraint, Direction, Layout},
    style::Style,
    text::{Line, Span},
    widgets::{
        block::Position, Block, Borders, HighlightSpacing, List, ListDirection, ListItem,
        ListState, Paragraph,
    },
    DefaultTerminal, Frame,
};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::{
    configs::{PickerColorConfig, Config},
    github::GitHubClient,
    keymap::{Keymap, PickerAction},
    state::StateManager,
    tmux::Tmux,
    Result, TmsError,
};

pub enum Preview {
    SessionPane,
    WindowPane,
    Directory,
}

#[derive(Debug, Default, PartialEq, Eq, Deserialize, Serialize, Clone, Copy)]
pub enum InputPosition {
    Top,
    #[default]
    Bottom,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickerMode {
    Local,
    GitHub(String), // profile name
}

impl PickerMode {
    pub fn display_name(&self) -> String {
        match self {
            PickerMode::Local => "Local repos".to_string(),
            PickerMode::GitHub(profile_name) => format!("Github - {}", profile_name),
        }
    }
}

pub struct Picker<'a> {
    matcher: Nucleo<String>,
    preview: Option<Preview>,
    colors: Option<&'a PickerColorConfig>,
    selection: ListState,
    filter: String,
    cursor_pos: u16,
    keymap: Keymap,
    input_position: InputPosition,
    tmux: &'a Tmux,
    page_size: usize,
    receiver: Option<mpsc::UnboundedReceiver<String>>,
    total_items_added: usize,
    // GitHub profile support
    current_mode: PickerMode,
    available_modes: Vec<PickerMode>,
    github_client: Option<GitHubClient>,
    state_manager: Option<StateManager>,
    config: &'a Config,
}

impl<'a> Picker<'a> {
    pub fn new(
        list: &[String],
        preview: Option<Preview>,
        keymap: Option<&Keymap>,
        input_position: InputPosition,
        tmux: &'a Tmux,
        config: &'a Config,
    ) -> Self {
        let matcher = Nucleo::new(nucleo::Config::DEFAULT, Arc::new(request_redraw), None, 1);

        let injector = matcher.injector();

        for str in list {
            injector.push(str.to_owned(), |_, dst| dst[0] = str.to_owned().into());
        }

        let keymap = if let Some(keymap) = keymap {
            Keymap::with_defaults(keymap)
        } else {
            Keymap::default()
        };

        // Setup available modes
        let mut available_modes = vec![PickerMode::Local];
        
        // Add GitHub profiles as modes
        for profile in config.get_github_profiles() {
            available_modes.push(PickerMode::GitHub(profile.name));
        }

        // Determine initial mode based on saved active profile
        let (current_mode, state_manager) = if let Ok(state_manager) = StateManager::new() {
            let active_profile = state_manager.get_active_profile().unwrap_or_default();
            let current_mode = if let Some(active_profile) = active_profile {
                if let Some(mode) = available_modes.iter().find(|mode| {
                    match mode {
                        PickerMode::Local => active_profile == "local",
                        PickerMode::GitHub(name) => name == &active_profile,
                    }
                }) {
                    mode.clone()
                } else {
                    PickerMode::Local
                }
            } else {
                PickerMode::Local
            };
            (current_mode, Some(state_manager))
        } else {
            (PickerMode::Local, None)
        };

        // Try to create GitHub client
        let github_client = GitHubClient::new().ok();

        Picker {
            matcher,
            preview,
            colors: None,
            selection: ListState::default(),
            filter: String::default(),
            cursor_pos: 0,
            keymap,
            input_position,
            tmux,
            page_size: 10, // Default page size, will be updated during render
            receiver: None,
            total_items_added: list.len(),
            current_mode,
            available_modes,
            github_client,
            state_manager,
            config,
        }
    }

    /// Create a new streaming picker that starts empty and receives items via channel
    pub fn new_streaming(
        preview: Option<Preview>,
        keymap: Option<&Keymap>,
        input_position: InputPosition,
        tmux: &'a Tmux,
        receiver: mpsc::UnboundedReceiver<String>,
        config: &'a Config,
    ) -> Self {
        let matcher = Nucleo::new(nucleo::Config::DEFAULT, Arc::new(request_redraw), None, 1);

        let keymap = if let Some(keymap) = keymap {
            Keymap::with_defaults(keymap)
        } else {
            Keymap::default()
        };

        // Setup available modes
        let mut available_modes = vec![PickerMode::Local];
        
        // Add GitHub profiles as modes
        for profile in config.get_github_profiles() {
            available_modes.push(PickerMode::GitHub(profile.name));
        }

        // Determine initial mode based on saved active profile
        let (current_mode, state_manager) = if let Ok(state_manager) = StateManager::new() {
            let active_profile = state_manager.get_active_profile().unwrap_or_default();
            let current_mode = if let Some(active_profile) = active_profile {
                if let Some(mode) = available_modes.iter().find(|mode| {
                    match mode {
                        PickerMode::Local => active_profile == "local",
                        PickerMode::GitHub(name) => name == &active_profile,
                    }
                }) {
                    mode.clone()
                } else {
                    PickerMode::Local
                }
            } else {
                PickerMode::Local
            };
            (current_mode, Some(state_manager))
        } else {
            (PickerMode::Local, None)
        };

        // Try to create GitHub client
        let github_client = GitHubClient::new().ok();

        Picker {
            matcher,
            preview,
            colors: None,
            selection: ListState::default(),
            filter: String::default(),
            cursor_pos: 0,
            keymap,
            input_position,
            tmux,
            page_size: 10,
            receiver: Some(receiver),
            total_items_added: 0,
            current_mode,
            available_modes,
            github_client,
            state_manager,
            config,
        }
    }

    pub fn set_colors(mut self, colors: Option<&'a PickerColorConfig>) -> Self {
        self.colors = colors;

        self
    }

    pub async fn run(&mut self) -> Result<Option<String>> {
        // Handle cases where no TTY is available (like in Nix sandbox or CI)
        // We need to check for TTY availability before initializing ratatui
        use std::io::IsTerminal;
        if !std::io::stdout().is_terminal() {
            return Err(TmsError::TuiError(
                "Cannot initialize terminal (no TTY available). This may indicate a configuration error or test environment.".to_string()
            ).into());
        }

        let mut terminal = ratatui::init();

        let selected_str = self
            .async_main_loop(&mut terminal)
            .await
            .map_err(|e| TmsError::TuiError(e.to_string()));

        ratatui::restore();

        Ok(selected_str?)
    }

    async fn async_main_loop(&mut self, terminal: &mut DefaultTerminal) -> Result<Option<String>> {
        // Load initial data for the current mode if it's a GitHub profile
        if matches!(self.current_mode, PickerMode::GitHub(_)) {
            if let Err(e) = self.refresh_current_mode().await {
                eprintln!("Warning: Error loading initial GitHub repositories: {}", e);
            }
        }
        loop {
            self.matcher.tick(1000);
            
            // Check for new streaming items
            if let Some(ref mut receiver) = self.receiver {
                // Process all available items without blocking
                while let Ok(item) = receiver.try_recv() {
                    let injector = self.matcher.injector();
                    injector.push(item.clone(), |_, dst| dst[0] = item.into());
                    self.total_items_added += 1;
                }
            }
            
            self.update_selection();
            terminal
                .draw(|f| self.render(f))
                .map_err(|e| TmsError::TuiError(e.to_string()))?;

            // Use a shorter timeout for better responsiveness during streaming
            let timeout = if self.receiver.is_some() { 
                std::time::Duration::from_millis(50) // 50ms for streaming
            } else { 
                std::time::Duration::from_millis(100) // 100ms for static
            };

            match crossterm::event::poll(timeout).map_err(|e| TmsError::TuiError(e.to_string()))? {
                true => {
                    if let Event::Key(key) = event::read().map_err(|e| TmsError::TuiError(e.to_string()))? {
                        if key.kind == KeyEventKind::Press {
                            // Check for mode switching key
                            let switch_key = &self.config.get_picker_switch_mode_key();
                            let refresh_key = &self.config.get_picker_refresh_key();
                            
                            if key.code == KeyCode::Tab && switch_key == "tab" {
                                self.switch_to_next_mode();
                                // Load data for the new mode
                                if let Err(e) = self.refresh_current_mode().await {
                                    eprintln!("Error loading data for mode: {}", e);
                                }
                                continue;
                            } else if key.code == KeyCode::F(5) && refresh_key == "f5" {
                                self.refresh_current_mode().await?;
                                continue;
                            }
                            
                            match self.keymap.0.get(&key.into()) {
                                Some(PickerAction::Cancel) => return Ok(None),
                                Some(PickerAction::Confirm) => {
                                    if let Some(selected) = self.get_selected() {
                                        let selected = selected.to_owned();
                                        return self.handle_selection(&selected).await;
                                    }
                                }
                                Some(PickerAction::SwitchMode) => {
                                    self.switch_to_next_mode();
                                }
                                Some(PickerAction::Refresh) => {
                                    self.refresh_current_mode().await?;
                                }
                                Some(PickerAction::Backspace) => self.remove_filter(),
                                Some(PickerAction::Delete) => self.delete(),
                                Some(PickerAction::DeleteWord) => self.delete_word(),
                                Some(PickerAction::DeleteToLineStart) => self.delete_to_line(false),
                                Some(PickerAction::DeleteToLineEnd) => self.delete_to_line(true),
                                Some(PickerAction::MoveUp) => self.move_up(),
                                Some(PickerAction::MoveDown) => self.move_down(),
                                Some(PickerAction::PageUp) => self.page_up(),
                                Some(PickerAction::PageDown) => self.page_down(),
                                Some(PickerAction::CursorLeft) => self.move_cursor_left(),
                                Some(PickerAction::CursorRight) => self.move_cursor_right(),
                                Some(PickerAction::MoveToLineStart) => self.move_to_start(),
                                Some(PickerAction::MoveToLineEnd) => self.move_to_end(),
                                Some(PickerAction::Noop) => {}
                                None => {
                                    if let KeyCode::Char(c) = key.code {
                                        self.update_filter(c)
                                    }
                                }
                            }
                        }
                    }
                }
                false => {
                    // No input available, continue the loop to check for streaming items
                    continue;
                }
            }
        }
    }

    fn update_selection(&mut self) {
        let snapshot = self.matcher.snapshot();
        if let Some(selected) = self.selection.selected() {
            if snapshot.matched_item_count() == 0 {
                self.selection.select(None);
            } else if selected > snapshot.matched_item_count() as usize {
                self.selection
                    .select(Some(snapshot.matched_item_count() as usize - 1));
            }
        } else if snapshot.matched_item_count() > 0 {
            self.selection.select(Some(0));
        }
    }

    fn render(&mut self, f: &mut Frame) {
        let preview_direction;
        let picker_pane;
        let preview_pane;
        let area = f.area();
        let mut input_position = self.input_position;

        let preview_split = if self.preview.is_some() {
            preview_direction = if area.width.div_ceil(2) >= area.height {
                picker_pane = 0;
                preview_pane = 1;
                Direction::Horizontal
            } else {
                picker_pane = 1;
                preview_pane = 0;
                input_position = InputPosition::Bottom;
                Direction::Vertical
            };
            Layout::new(
                preview_direction,
                [Constraint::Percentage(50), Constraint::Percentage(50)],
            )
            .split(area)
        } else {
            picker_pane = 0;
            preview_pane = 1;
            preview_direction = Direction::Horizontal;
            Rc::new([area])
        };

        let top_constraint;
        let bottom_constraint;
        let list_direction;
        let input_index;
        let list_index;
        let borders;
        let title_position;
        match input_position {
            InputPosition::Top => {
                top_constraint = Constraint::Length(1);
                bottom_constraint = Constraint::Length(preview_split[picker_pane].height - 1);
                list_direction = ListDirection::TopToBottom;
                input_index = 0;
                list_index = 1;
                borders = Borders::TOP;
                title_position = Position::Top;
            }
            InputPosition::Bottom => {
                top_constraint = Constraint::Length(preview_split[picker_pane].height - 1);
                bottom_constraint = Constraint::Length(1);
                list_direction = ListDirection::BottomToTop;
                input_index = 1;
                list_index = 0;
                borders = Borders::BOTTOM;
                title_position = Position::Bottom;
            }
        }
        let layout = Layout::new(Direction::Vertical, [top_constraint, bottom_constraint])
            .split(preview_split[picker_pane]);

        // Update page size based on the list area height
        self.page_size = layout[list_index].height.saturating_sub(1).max(1) as usize;

        let snapshot = self.matcher.snapshot();
        let matches = snapshot
            .matched_items(..snapshot.matched_item_count())
            .map(|item| ListItem::new(item.data.as_str()));

        let colors = if let Some(colors) = self.colors {
            colors.to_owned()
        } else {
            PickerColorConfig::default_colors()
        };

        let table = List::new(matches)
            .highlight_style(colors.highlight_style())
            .direction(list_direction)
            .highlight_spacing(HighlightSpacing::Always)
            .highlight_symbol("> ")
            .block(
                Block::default()
                    .borders(borders)
                    .border_style(Style::default().fg(colors.border_color()))
                    .title_style(Style::default().fg(colors.info_color()))
                    .title_position(title_position)
                    .title(if self.receiver.is_some() {
                        format!(
                            "{} - 🔍 {}/{} (scanning...)",
                            self.current_mode.display_name(),
                            snapshot.matched_item_count(),
                            snapshot.item_count()
                        )
                    } else {
                        format!(
                            "{} - {}/{}",
                            self.current_mode.display_name(),
                            snapshot.matched_item_count(),
                            snapshot.item_count()
                        )
                    }),
            );
        f.render_stateful_widget(table, layout[list_index], &mut self.selection);

        let prompt = Span::styled("> ", Style::default().fg(colors.prompt_color()));
        let input_text = Span::raw(&self.filter);
        let input_line = Line::from(vec![prompt, input_text]);
        let input = Paragraph::new(vec![input_line]);
        f.render_widget(input, layout[input_index]);
        f.set_cursor_position(layout::Position {
            x: layout[input_index].x + self.cursor_pos + 2,
            y: layout[input_index].y,
        });

        if self.preview.is_some() {
            let preview = PreviewWidget::new(
                self.get_preview_text(),
                colors.border_color(),
                preview_direction,
            );
            f.render_widget(preview, preview_split[preview_pane]);
        }
    }

    fn get_preview_text(&self) -> String {
        if let Some(item_data) = self.get_selected() {
            let output = match self.preview {
                Some(Preview::SessionPane) => self.tmux.capture_pane(item_data),
                Some(Preview::WindowPane) => self.tmux.capture_pane(
                    item_data
                        .split_once(' ')
                        .map(|val| val.0)
                        .unwrap_or_default(),
                ),
                Some(Preview::Directory) => process::Command::new("ls")
                    .args(["-1", item_data])
                    .output()
                    .unwrap_or_else(|_| {
                        panic!("Failed to execute the command for directory: {}", item_data)
                    }),
                None => panic!("preview rendering should not have occured"),
            };

            if output.status.success() {
                String::from_utf8(output.stdout).unwrap()
            } else {
                String::default()
            }
        } else {
            String::default()
        }
    }

    fn get_selected(&self) -> Option<&String> {
        if let Some(index) = self.selection.selected() {
            return self
                .matcher
                .snapshot()
                .get_matched_item(index as u32)
                .map(|item| item.data);
        }

        None
    }

    fn move_up(&mut self) {
        if self.input_position == InputPosition::Bottom {
            self.do_move_up()
        } else {
            self.do_move_down()
        }
    }

    fn move_down(&mut self) {
        if self.input_position == InputPosition::Bottom {
            self.do_move_down()
        } else {
            self.do_move_up()
        }
    }

    fn do_move_up(&mut self) {
        let item_count = self.matcher.snapshot().matched_item_count() as usize;
        if item_count == 0 {
            return;
        }

        let max = item_count - 1;

        match self.selection.selected() {
            Some(i) if i >= max => self.selection.select(Some(0)),
            Some(i) => self.selection.select(Some(i + 1)),
            None => self.selection.select(Some(0)),
        }
    }

    fn do_move_down(&mut self) {
        match self.selection.selected() {
            Some(0) => {
                let item_count = self.matcher.snapshot().matched_item_count() as usize;
                if item_count == 0 {
                    return;
                }
                self.selection.select(Some(item_count - 1))
            }
            Some(i) => self.selection.select(Some(i - 1)),
            None => self.selection.select(Some(0)),
        }
    }

    fn page_up(&mut self) {
        if self.input_position == InputPosition::Bottom {
            self.do_page_up()
        } else {
            self.do_page_down()
        }
    }

    fn page_down(&mut self) {
        if self.input_position == InputPosition::Bottom {
            self.do_page_down()
        } else {
            self.do_page_up()
        }
    }

    fn do_page_up(&mut self) {
        let item_count = self.matcher.snapshot().matched_item_count() as usize;
        if item_count == 0 {
            return;
        }

        let max = item_count - 1;

        match self.selection.selected() {
            Some(i) => {
                let new_index = i.saturating_add(self.page_size).min(max);
                self.selection.select(Some(new_index));
            }
            None => self.selection.select(Some(0)),
        }
    }

    fn do_page_down(&mut self) {
        let item_count = self.matcher.snapshot().matched_item_count() as usize;
        if item_count == 0 {
            return;
        }

        match self.selection.selected() {
            Some(i) => {
                let new_index = i.saturating_sub(self.page_size);
                self.selection.select(Some(new_index));
            }
            None => self.selection.select(Some(0)),
        }
    }

    fn move_cursor_left(&mut self) {
        if self.cursor_pos > 0 {
            self.cursor_pos -= 1;
        }
    }

    fn move_cursor_right(&mut self) {
        if self.cursor_pos < self.filter.len() as u16 {
            self.cursor_pos += 1;
        }
    }

    fn update_filter(&mut self, c: char) {
        if self.filter.len() == u16::MAX as usize {
            return;
        }

        let prev_filter = self.filter.clone();
        self.filter.insert(self.cursor_pos as usize, c);
        self.cursor_pos += 1;

        self.update_matcher_pattern(&prev_filter);
    }

    fn remove_filter(&mut self) {
        if self.cursor_pos == 0 {
            return;
        }

        let prev_filter = self.filter.clone();
        self.filter.remove(self.cursor_pos as usize - 1);

        self.cursor_pos -= 1;

        if self.filter != prev_filter {
            self.update_matcher_pattern(&prev_filter);
        }
    }

    fn delete(&mut self) {
        if (self.cursor_pos as usize) == self.filter.len() {
            return;
        }

        let prev_filter = self.filter.clone();
        self.filter.remove(self.cursor_pos as usize);

        if self.filter != prev_filter {
            self.update_matcher_pattern(&prev_filter);
        }
    }

    fn update_matcher_pattern(&mut self, _prev_filter: &str) {
        self.matcher.pattern.reparse(
            0,
            self.filter.as_str(),
            CaseMatching::Ignore,
            Normalization::Smart,
            false,
        );
        for _ in 0..10 {
            self.matcher.tick(1000);
        }
    }

    fn delete_word(&mut self) {
        let mut chars = self
            .filter
            .chars()
            .rev()
            .skip(self.filter.chars().count() - self.cursor_pos as usize);
        let length = std::cmp::min(
            u16::try_from(
                1 + chars.by_ref().take_while(|c| *c == ' ').count()
                    + chars.by_ref().take_while(|c| *c != ' ').count(),
            )
            .unwrap_or(self.cursor_pos),
            self.cursor_pos,
        );

        let prev_filter = self.filter.clone();
        let new_cursor_pos = self.cursor_pos - length;

        self.filter
            .drain((new_cursor_pos as usize)..(self.cursor_pos as usize));

        self.cursor_pos = new_cursor_pos;

        if self.filter != prev_filter {
            self.update_matcher_pattern(&prev_filter);
        }
    }

    fn delete_to_line(&mut self, forward: bool) {
        let prev_filter = self.filter.clone();

        if forward {
            self.filter.drain((self.cursor_pos as usize)..);
        } else {
            self.filter.drain(..(self.cursor_pos as usize));
            self.cursor_pos = 0;
        }

        if self.filter != prev_filter {
            self.update_matcher_pattern(&prev_filter);
        }
    }

    fn move_to_start(&mut self) {
        self.cursor_pos = 0;
    }

    fn move_to_end(&mut self) {
        self.cursor_pos = u16::try_from(self.filter.len()).unwrap_or_default();
    }

    fn switch_to_next_mode(&mut self) {
        if self.available_modes.len() <= 1 {
            return;
        }

        let current_index = self.available_modes.iter()
            .position(|mode| mode == &self.current_mode)
            .unwrap_or(0);
        
        let next_index = (current_index + 1) % self.available_modes.len();
        self.current_mode = self.available_modes[next_index].clone();
        
        // Clear current items and reset selection
        self.matcher = Nucleo::new(nucleo::Config::DEFAULT, Arc::new(request_redraw), None, 1);
        self.selection = ListState::default();
        self.total_items_added = 0;
        
        // We'll load items for the new mode in the main loop via refresh_current_mode
    }

    async fn refresh_current_mode(&mut self) -> Result<()> {
        match &self.current_mode {
            PickerMode::Local => {
                // For local mode, we don't need to do anything special since
                // the streaming should handle local repositories
                // This could be extended to re-scan local directories
            }
            PickerMode::GitHub(profile_name) => {
                if let Some(ref github_client) = self.github_client {
                    if let Some(profile) = self.config.get_github_profiles().iter()
                        .find(|p| &p.name == profile_name) {
                        
                        match github_client.get_repositories(profile, true).await {
                            Ok(repos) => {
                                // Clear current matcher and add GitHub repos
                                self.matcher = Nucleo::new(nucleo::Config::DEFAULT, Arc::new(request_redraw), None, 1);
                                let injector = self.matcher.injector();
                                
                                let repo_count = repos.len();
                                for repo in &repos {
                                    let display_name = format!("{} - {}", repo.name, 
                                        repo.description.as_deref().unwrap_or("No description"));
                                    injector.push(display_name.clone(), |_, dst| dst[0] = display_name.into());
                                }
                                
                                self.total_items_added = repo_count;
                                self.selection = ListState::default();
                            }
                            Err(e) => {
                                eprintln!("Error refreshing GitHub profile '{}': {}", profile_name, e);
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    async fn handle_selection(&mut self, selected: &str) -> Result<Option<String>> {
        match &self.current_mode {
            PickerMode::Local => {
                // Save current active profile
                if let Some(ref state_manager) = self.state_manager {
                    let _ = state_manager.set_active_profile(Some("local".to_string()));
                }
                
                Ok(Some(selected.to_owned()))
            }
            PickerMode::GitHub(profile_name) => {
                // Save current active profile
                if let Some(ref state_manager) = self.state_manager {
                    let _ = state_manager.set_active_profile(Some(profile_name.clone()));
                }

                if let Some(ref github_client) = self.github_client {
                    if let Some(profile) = self.config.get_github_profiles().iter()
                        .find(|p| &p.name == profile_name) {
                        
                        // Extract repo name from the selected display string
                        let repo_name = selected.split(" - ").next().unwrap_or(selected);
                        
                        // Get the repository details
                        match github_client.get_repositories(profile, false).await {
                            Ok(repos) => {
                                if let Some(repo) = repos.iter().find(|r| r.name == repo_name) {
                                    // Get clone root path
                                    let clone_root = crate::github::expand_clone_root_path(&profile.clone_root_path)?;
                                    
                                    // Clone the repository
                                    match github_client.clone_repository(repo, profile, &clone_root).await {
                                        Ok(repo_path) => {
                                            // Create a special marker for GitHub repos
                                            // We'll return a special format that indicates this is a GitHub repo
                                            Ok(Some(format!("github:{}", repo_path.to_string_lossy())))
                                        }
                                        Err(e) => {
                                            eprintln!("Error cloning repository: {}", e);
                                            Err(e)
                                        }
                                    }
                                } else {
                                    eprintln!("Repository '{}' not found in profile", repo_name);
                                    Ok(None)
                                }
                            }
                            Err(e) => {
                                eprintln!("Error getting repositories: {}", e);
                                Err(e)
                            }
                        }
                    } else {
                        eprintln!("GitHub profile '{}' not found", profile_name);
                        Ok(None)
                    }
                } else {
                    eprintln!("GitHub client not available");
                    Ok(None)
                }
            }
        }
    }
}

fn request_redraw() {}
