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
    style::{Color, Style},
    text::{Line, Span},
    widgets::{
        block::Position, Block, Borders, Clear, HighlightSpacing, List, ListDirection, ListItem,
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
    session::SessionContainer,
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

/// UI state for the picker
#[derive(Debug, Clone, PartialEq)]
enum UIState {
    /// Normal picker operation
    Normal,
    /// Mode selection overlay
    ModeSelection {
        selection: usize,
        filter: String,
        cursor_pos: usize,
    },
    /// Loading state with progress message
    Loading(String),
    /// Error display
    Error(String),
}

/// Background operation status
#[derive(Debug, Clone)]
enum BackgroundOp {
    None,
    LoadingLocal,
    LoadingGitHub(String),
    RefreshingCurrent,
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
    // UI State management
    ui_state: UIState,
    background_op: BackgroundOp,
    // Error/message display
    status_message: Option<String>,
    error_message: Option<String>,
}

fn create_available_modes(config: &Config) -> Vec<PickerMode> {
    let mut available_modes = vec![PickerMode::Local];
    
    // Add GitHub profiles as modes (deduplicate by name to prevent duplicate modes)
    let mut seen_profile_names = std::collections::HashSet::new();
    for profile in config.get_github_profiles() {
        if seen_profile_names.insert(profile.name.clone()) {
            available_modes.push(PickerMode::GitHub(profile.name));
        }
    }
    
    available_modes
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
        let available_modes = create_available_modes(config);

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
            ui_state: UIState::Normal,
            background_op: BackgroundOp::None,
            status_message: None,
            error_message: None,
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
        let available_modes = create_available_modes(config);

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
            ui_state: UIState::Normal,
            background_op: BackgroundOp::None,
            status_message: None,
            error_message: None,
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
            self.start_loading_github_mode(false).await;
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
            
            // Check for background operation completion
            self.check_background_operations().await;
            
            terminal
                .draw(|f| self.render_with_overlays(f))
                .map_err(|e| TmsError::TuiError(e.to_string()))?;

            // Use a shorter timeout for better responsiveness
            let timeout = std::time::Duration::from_millis(50);

            match crossterm::event::poll(timeout).map_err(|e| TmsError::TuiError(e.to_string()))? {
                true => {
                    if let Event::Key(key) = event::read().map_err(|e| TmsError::TuiError(e.to_string()))? {
                        if key.kind == KeyEventKind::Press {
                            if let Some(result) = self.handle_key_event(key).await? {
                                return Ok(result);
                            }
                        }
                    }
                }
                false => {
                    // No input available, continue the loop
                    continue;
                }
            }
        }
    }

    /// Handle key events based on current UI state
    async fn handle_key_event(&mut self, key: crossterm::event::KeyEvent) -> Result<Option<Option<String>>> {
        match &self.ui_state {
            UIState::Normal => self.handle_normal_key_event(key).await,
            UIState::ModeSelection { .. } => {
                self.handle_mode_selection_key_event(key).await;
                Ok(None)
            }
            UIState::Loading(_) => {
                // In loading state, only allow cancel
                if matches!(self.keymap.0.get(&key.into()), Some(PickerAction::Cancel)) {
                    Ok(Some(None))
                } else {
                    Ok(None)
                }
            }
            UIState::Error(_) => {
                // Any key dismisses error
                self.ui_state = UIState::Normal;
                self.error_message = None;
                Ok(None)
            }
        }
    }

    /// Handle key events in normal mode
    async fn handle_normal_key_event(&mut self, key: crossterm::event::KeyEvent) -> Result<Option<Option<String>>> {
        // Check for mode switching key
        let switch_key = &self.config.get_picker_switch_mode_key();
        let refresh_key = &self.config.get_picker_refresh_key();
        
        if key.code == KeyCode::Tab && switch_key == "tab" {
            self.enter_mode_selection();
            return Ok(None);
        } else if key.code == KeyCode::F(5) && refresh_key == "f5" {
            self.start_refresh_current_mode().await;
            return Ok(None);
        }
        
        match self.keymap.0.get(&key.into()) {
            Some(PickerAction::Cancel) => Ok(Some(None)),
            Some(PickerAction::Confirm) => {
                if let Some(selected) = self.get_selected() {
                    let selected = selected.to_owned();
                    Ok(Some(self.handle_selection(&selected).await?))
                } else {
                    Ok(None)
                }
            }
            Some(PickerAction::SwitchMode) => {
                self.enter_mode_selection();
                Ok(None)
            }
            Some(PickerAction::Refresh) => {
                self.start_refresh_current_mode().await;
                Ok(None)
            }
            Some(PickerAction::Backspace) => {
                self.remove_filter();
                Ok(None)
            }
            Some(PickerAction::Delete) => {
                self.delete();
                Ok(None)
            }
            Some(PickerAction::DeleteWord) => {
                self.delete_word();
                Ok(None)
            }
            Some(PickerAction::DeleteToLineStart) => {
                self.delete_to_line(false);
                Ok(None)
            }
            Some(PickerAction::DeleteToLineEnd) => {
                self.delete_to_line(true);
                Ok(None)
            }
            Some(PickerAction::MoveUp) => {
                self.move_up();
                Ok(None)
            }
            Some(PickerAction::MoveDown) => {
                self.move_down();
                Ok(None)
            }
            Some(PickerAction::PageUp) => {
                self.page_up();
                Ok(None)
            }
            Some(PickerAction::PageDown) => {
                self.page_down();
                Ok(None)
            }
            Some(PickerAction::CursorLeft) => {
                self.move_cursor_left();
                Ok(None)
            }
            Some(PickerAction::CursorRight) => {
                self.move_cursor_right();
                Ok(None)
            }
            Some(PickerAction::MoveToLineStart) => {
                self.move_to_start();
                Ok(None)
            }
            Some(PickerAction::MoveToLineEnd) => {
                self.move_to_end();
                Ok(None)
            }
            Some(PickerAction::Noop) => Ok(None),
            None => {
                if let KeyCode::Char(c) = key.code {
                    self.update_filter(c);
                }
                Ok(None)
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

    /// Render the picker with overlays based on UI state
    fn render_with_overlays(&mut self, f: &mut Frame) {
        // Always render the base picker
        self.render(f);
        
        // Render overlays based on UI state
        match &self.ui_state {
            UIState::Normal => {
                // Render status message if any
                if let Some(ref message) = self.status_message {
                    self.render_status_overlay(f, message);
                }
            }
            UIState::ModeSelection { selection, filter, cursor_pos } => {
                self.render_mode_selection_overlay(f, *selection, filter, *cursor_pos);
            }
            UIState::Loading(message) => {
                self.render_loading_overlay(f, message);
            }
            UIState::Error(error) => {
                self.render_error_overlay(f, error);
            }
        }
    }

    /// Render mode selection overlay
    fn render_mode_selection_overlay(&self, f: &mut Frame, selection: usize, filter: &str, cursor_pos: usize) {
        let area = f.area();
        
        // Create a centered popup
        let popup_area = popup_area(area, 60, 70);
        
        // Clear the area
        f.render_widget(Clear, popup_area);
        
        let colors = if let Some(colors) = self.colors {
            colors.to_owned()
        } else {
            PickerColorConfig::default_colors()
        };

        // Filter modes based on filter text
        let filtered_modes: Vec<(usize, &PickerMode)> = self.available_modes.iter().enumerate()
            .filter(|(_, mode)| {
                if filter.is_empty() {
                    true
                } else {
                    mode.display_name().to_lowercase().contains(&filter.to_lowercase())
                }
            })
            .collect();

        // Find the current mode in filtered list for initial selection
        let mut adjusted_selection = selection.min(filtered_modes.len().saturating_sub(1));
        
        // If no filter and selection is 0, try to find and select the current mode
        if filter.is_empty() && selection == 0 {
            if let Some(current_filtered_index) = filtered_modes.iter().position(|(_, mode)| *mode == &self.current_mode) {
                adjusted_selection = current_filtered_index;
            }
        }

        // Split area for list and input
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(3),
                Constraint::Length(3),
            ])
            .split(popup_area);

        // Render mode list
        let items: Vec<ListItem> = filtered_modes.iter()
            .map(|(_, mode)| {
                let display_name = mode.display_name();
                if *mode == &self.current_mode {
                    ListItem::new(format!("● {} (current)", display_name))
                } else {
                    ListItem::new(format!("  {}", display_name))
                }
            })
            .collect();

        let mut list_state = ListState::default();
        if !filtered_modes.is_empty() && adjusted_selection < filtered_modes.len() {
            list_state.select(Some(adjusted_selection));
        }

        let list = List::new(items)
            .highlight_style(colors.highlight_style())
            .highlight_symbol("> ")
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(colors.border_color()))
                    .title("Select Mode")
                    .title_style(Style::default().fg(colors.info_color())),
            );
        f.render_stateful_widget(list, layout[0], &mut list_state);

        // Render filter input
        let input_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(colors.border_color()))
            .title("Filter");
            
        let input = Paragraph::new(filter)
            .block(input_block)
            .style(Style::default().fg(colors.prompt_color()));
        f.render_widget(input, layout[1]);

        // Set cursor position
        if cursor_pos <= filter.len() {
            f.set_cursor_position(layout::Position {
                x: layout[1].x + cursor_pos as u16 + 1,
                y: layout[1].y + 1,
            });
        }
    }

    /// Render loading overlay
    fn render_loading_overlay(&self, f: &mut Frame, message: &str) {
        let area = f.area();
        let popup_area = popup_area(area, 50, 20);
        
        f.render_widget(Clear, popup_area);
        
        let colors = if let Some(colors) = self.colors {
            colors.to_owned()
        } else {
            PickerColorConfig::default_colors()
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(colors.border_color()))
            .title("Loading")
            .title_style(Style::default().fg(colors.info_color()));
            
        let paragraph = Paragraph::new(message)
            .block(block)
            .style(Style::default().fg(colors.prompt_color()));
        f.render_widget(paragraph, popup_area);
    }

    /// Render error overlay
    fn render_error_overlay(&self, f: &mut Frame, error: &str) {
        let area = f.area();
        let popup_area = popup_area(area, 60, 30);
        
        f.render_widget(Clear, popup_area);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Red))
            .title("Error - Press any key to continue")
            .title_style(Style::default().fg(Color::Red));
            
        let paragraph = Paragraph::new(error)
            .block(block)
            .style(Style::default().fg(Color::Red));
        f.render_widget(paragraph, popup_area);
    }

    /// Render status overlay
    fn render_status_overlay(&self, f: &mut Frame, message: &str) {
        let area = f.area();
        
        // Render status at the bottom
        let status_area = layout::Rect {
            x: area.x,
            y: area.bottom().saturating_sub(1),
            width: area.width,
            height: 1,
        };
        
        let colors = if let Some(colors) = self.colors {
            colors.to_owned()
        } else {
            PickerColorConfig::default_colors()
        };

        let status = Paragraph::new(message)
            .style(Style::default().fg(colors.info_color()));
        f.render_widget(status, status_area);
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

    /// Enter mode selection UI state
    fn enter_mode_selection(&mut self) {
        if self.available_modes.len() <= 1 {
            return;
        }

        // Start with no selection initially, will be set correctly in rendering
        self.ui_state = UIState::ModeSelection {
            selection: 0,
            filter: String::new(),
            cursor_pos: 0,
        };
    }

    /// Handle key events in mode selection state
    async fn handle_mode_selection_key_event(&mut self, key: crossterm::event::KeyEvent) {
        if let UIState::ModeSelection { selection, filter, cursor_pos } = &mut self.ui_state {
            // Filter modes based on current filter text
            let filtered_modes: Vec<(usize, &PickerMode)> = self.available_modes.iter().enumerate()
                .filter(|(_, mode)| {
                    if filter.is_empty() {
                        true
                    } else {
                        mode.display_name().to_lowercase().contains(&filter.to_lowercase())
                    }
                })
                .collect();
            
            match key.code {
                KeyCode::Esc => {
                    self.ui_state = UIState::Normal;
                }
                KeyCode::Enter => {
                    if *selection < filtered_modes.len() {
                        let (original_index, _) = filtered_modes[*selection];
                        let selected_mode = self.available_modes[original_index].clone();
                        if selected_mode != self.current_mode {
                            self.switch_to_mode(selected_mode).await;
                        }
                    }
                    self.ui_state = UIState::Normal;
                }
                KeyCode::Up => {
                    if !filtered_modes.is_empty() {
                        *selection = if *selection == 0 {
                            filtered_modes.len() - 1
                        } else {
                            *selection - 1
                        };
                    }
                }
                KeyCode::Down => {
                    if !filtered_modes.is_empty() {
                        *selection = (*selection + 1) % filtered_modes.len();
                    }
                }
                KeyCode::Char(c) => {
                    filter.insert(*cursor_pos, c);
                    *cursor_pos += 1;
                    *selection = 0; // Reset selection when filtering
                }
                KeyCode::Backspace => {
                    if *cursor_pos > 0 {
                        filter.remove(*cursor_pos - 1);
                        *cursor_pos -= 1;
                        *selection = 0; // Reset selection when filtering
                    }
                }
                _ => {}
            }
        }
    }

    /// Switch to a new mode
    async fn switch_to_mode(&mut self, new_mode: PickerMode) {
        self.current_mode = new_mode.clone();
        self.clear_and_save_mode();
        
        match &new_mode {
            PickerMode::Local => {
                self.start_loading_local_mode(false).await;
            }
            PickerMode::GitHub(_) => {
                self.start_loading_github_mode(false).await;
            }
        }
    }

    /// Start loading local mode data in the background
    async fn start_loading_local_mode(&mut self, force_refresh: bool) {
        self.background_op = BackgroundOp::LoadingLocal;
        self.ui_state = UIState::Loading("Loading local repositories...".to_string());
        
        // Start background operation
        // This is where you'd spawn the actual loading operation
        // For now, we'll simulate with a simple load
        if let Err(e) = self.load_local_mode_data(force_refresh).await {
            self.set_error(format!("Failed to load local repositories: {}", e));
        } else {
            self.ui_state = UIState::Normal;
        }
        self.background_op = BackgroundOp::None;
    }

    /// Start loading GitHub mode data in the background
    async fn start_loading_github_mode(&mut self, force_refresh: bool) {
        if let PickerMode::GitHub(profile_name) = &self.current_mode {
            self.background_op = BackgroundOp::LoadingGitHub(profile_name.clone());
            self.ui_state = UIState::Loading(format!("Loading GitHub repositories for '{}'...", profile_name));
            
            if let Err(e) = self.load_github_mode_data(force_refresh).await {
                self.set_error(format!("Failed to load GitHub repositories: {}", e));
            } else {
                self.ui_state = UIState::Normal;
            }
            self.background_op = BackgroundOp::None;
        }
    }

    /// Start refreshing current mode
    async fn start_refresh_current_mode(&mut self) {
        self.background_op = BackgroundOp::RefreshingCurrent;
        
        match &self.current_mode {
            PickerMode::Local => {
                self.start_loading_local_mode(true).await;
            }
            PickerMode::GitHub(_) => {
                self.start_loading_github_mode(true).await;
            }
        }
    }

    /// Check for background operation completion
    async fn check_background_operations(&mut self) {
        // This method would check for completed background operations
        // and update the UI state accordingly. For now, we'll keep it simple
        // since we're doing synchronous operations.
    }

    /// Set error message and switch to error state
    fn set_error(&mut self, error: String) {
        self.error_message = Some(error.clone());
        self.ui_state = UIState::Error(error);
        self.background_op = BackgroundOp::None;
    }

    /// Set status message
    fn set_status(&mut self, message: String) {
        self.status_message = Some(message);
    }

    /// Clear status message
    fn clear_status(&mut self) {
        self.status_message = None;
    }

    /// Clear current data and save the new mode state
    fn clear_and_save_mode(&mut self) {
        // Clear current items and reset selection
        self.matcher = Nucleo::new(nucleo::Config::DEFAULT, Arc::new(request_redraw), None, 1);
        self.selection = ListState::default();
        self.total_items_added = 0;
        
        // Save the active profile state
        if let Some(ref state_manager) = self.state_manager {
            let profile_name = match &self.current_mode {
                PickerMode::Local => Some("local".to_string()),
                PickerMode::GitHub(name) => Some(name.clone()),
            };
            let _ = state_manager.set_active_profile(profile_name);
        }
    }

    async fn load_github_mode_data(&mut self, force_refresh: bool) -> Result<()> {
        if let PickerMode::GitHub(profile_name) = &self.current_mode {
            if let Some(ref github_client) = self.github_client {
                if let Some(profile) = self.config.get_github_profiles().iter()
                    .find(|p| &p.name == profile_name) {
                    
                    match github_client.get_repositories(profile, self.config, force_refresh).await {
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
                            self.set_error(format!("Error loading GitHub profile '{}': {}", profile_name, e));
                        }
                    }
                }
            }
        }
        Ok(())
    }

    async fn load_local_mode_data(&mut self, force_refresh: bool) -> Result<()> {
        // Use cached sessions for better performance
        match crate::session::create_sessions_cached(self.config, force_refresh).await {
            Ok(sessions) => {
                // Clear current matcher and add local sessions
                self.matcher = Nucleo::new(nucleo::Config::DEFAULT, Arc::new(request_redraw), None, 1);
                let injector = self.matcher.injector();
                
                let session_list = sessions.list_sorted(self.config);
                for session_name in &session_list {
                    injector.push(session_name.clone(), |_, dst| dst[0] = session_name.clone().into());
                }
                
                self.total_items_added = session_list.len();
                self.selection = ListState::default();
            }
            Err(e) => {
                self.set_error(format!("Error loading local sessions: {}", e));
                // Fallback to direct session creation if cache fails
                if let Ok(sessions) = crate::session::create_sessions(self.config).await {
                    self.matcher = Nucleo::new(nucleo::Config::DEFAULT, Arc::new(request_redraw), None, 1);
                    let injector = self.matcher.injector();
                    
                    let session_list = sessions.list_sorted(self.config);
                    for session_name in &session_list {
                        injector.push(session_name.clone(), |_, dst| dst[0] = session_name.clone().into());
                    }
                    
                    self.total_items_added = session_list.len();
                    self.selection = ListState::default();
                }
            }
        }
        Ok(())
    }

    async fn refresh_current_mode(&mut self) -> Result<()> {
        match &self.current_mode {
            PickerMode::Local => {
                // For local mode, use cached sessions unless in streaming mode
                if self.receiver.is_some() {
                    // For streaming mode, we can't easily restart the scan, so we keep existing items
                    // The user can manually refresh by restarting the application
                    // This could be enhanced in the future to support re-scanning
                } else {
                    // Force refresh for local sessions when explicitly requested (F5)
                    self.load_local_mode_data(true).await?;
                }
            }
            PickerMode::GitHub(_) => {
                // Force refresh for GitHub profiles when explicitly requested
                self.load_github_mode_data(true).await?;
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
                        match github_client.get_repositories(profile, self.config, false).await {
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
                                            self.set_error(format!("Error cloning repository: {}", e));
                                            Err(e)
                                        }
                                    }
                                } else {
                                    self.set_error(format!("Repository '{}' not found in profile", repo_name));
                                    Ok(None)
                                }
                            }
                            Err(e) => {
                                self.set_error(format!("Error getting repositories: {}", e));
                                Err(e)
                            }
                        }
                    } else {
                        self.set_error(format!("GitHub profile '{}' not found", profile_name));
                        Ok(None)
                    }
                } else {
                    self.set_error("GitHub client not available".to_string());
                    Ok(None)
                }
            }
        }
    }
}

fn request_redraw() {}

/// Helper function to calculate popup area
fn popup_area(area: layout::Rect, percent_x: u16, percent_y: u16) -> layout::Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::configs::{Config, GitHubProfile};

    #[test]
    fn test_no_duplicate_modes_creation() {
        // Create a config with GitHub profiles
        let mut config = Config::default();
        config.github_profiles = Some(vec![
            GitHubProfile {
                name: "work".to_string(),
                credentials_command: "echo token1".to_string(),
                clone_root_path: "~/work".to_string(),
                clone_method: None,
            },
            GitHubProfile {
                name: "personal".to_string(),
                credentials_command: "echo token2".to_string(),
                clone_root_path: "~/personal".to_string(),
                clone_method: None,
            },
        ]);

        // Simulate mode creation like in the constructor
        let available_modes = create_available_modes(&config);

        // Should have exactly 3 modes: Local + 2 GitHub profiles
        assert_eq!(available_modes.len(), 3, "Should have exactly 3 modes");

        let mode_names: Vec<String> = available_modes.iter().map(|m| m.display_name()).collect();
        println!("Created modes: {:?}", mode_names);

        // Check for duplicates
        let mut seen = std::collections::HashSet::new();
        for mode in &available_modes {
            let name = mode.display_name();
            assert!(seen.insert(name.clone()), "Duplicate mode found: {}", name);
        }

        // Verify specific modes exist
        assert!(available_modes.iter().any(|m| matches!(m, PickerMode::Local)), "Should have Local mode");
        assert!(available_modes.iter().any(|m| matches!(m, PickerMode::GitHub(name) if name == "work")), "Should have work GitHub mode");
        assert!(available_modes.iter().any(|m| matches!(m, PickerMode::GitHub(name) if name == "personal")), "Should have personal GitHub mode");
    }

    #[test] 
    fn test_config_get_github_profiles_no_duplicates() {
        // Test that get_github_profiles doesn't introduce duplicates
        let mut config = Config::default();
        config.github_profiles = Some(vec![
            GitHubProfile {
                name: "work".to_string(),
                credentials_command: "echo token1".to_string(),
                clone_root_path: "~/work".to_string(),
                clone_method: None,
            },
            GitHubProfile {
                name: "work".to_string(), // Intentional duplicate name
                credentials_command: "echo token2".to_string(),
                clone_root_path: "~/work2".to_string(),
                clone_method: None,
            },
        ]);

        let profiles = config.get_github_profiles();
        println!("Profile count: {}", profiles.len());
        for (i, profile) in profiles.iter().enumerate() {
            println!("Profile {}: name='{}', cmd='{}', path='{}'", 
                     i, profile.name, profile.credentials_command, profile.clone_root_path);
        }

        // This should show 2 profiles even though they have the same name
        // (the config parsing doesn't deduplicate by name, that would be the UI's job)
        assert_eq!(profiles.len(), 2, "Should have 2 profiles even with duplicate names");
        
        // But both should have name "work"
        assert_eq!(profiles[0].name, "work");
        assert_eq!(profiles[1].name, "work");
        
        // But different credentials commands
        assert_ne!(profiles[0].credentials_command, profiles[1].credentials_command);
    }

    #[test]
    fn test_create_available_modes_deduplicates() {
        // Test the new deduplication functionality
        let mut config = Config::default();
        config.github_profiles = Some(vec![
            GitHubProfile {
                name: "work".to_string(),
                credentials_command: "echo token1".to_string(),
                clone_root_path: "~/work1".to_string(),
                clone_method: None,
            },
            GitHubProfile {
                name: "personal".to_string(),
                credentials_command: "echo token2".to_string(),
                clone_root_path: "~/personal".to_string(),
                clone_method: None,
            },
            GitHubProfile {
                name: "work".to_string(), // Duplicate name - should be deduplicated
                credentials_command: "echo token3".to_string(),
                clone_root_path: "~/work2".to_string(),
                clone_method: None,
            },
        ]);

        let available_modes = create_available_modes(&config);
        
        println!("Available modes count: {}", available_modes.len());
        for (i, mode) in available_modes.iter().enumerate() {
            println!("Mode {}: '{}'", i, mode.display_name());
        }

        // Should have exactly 3 modes: Local + 2 unique GitHub profiles (work deduplicated)
        assert_eq!(available_modes.len(), 3, "Should have exactly 3 modes after deduplication");

        let mode_names: Vec<String> = available_modes.iter().map(|m| m.display_name()).collect();
        
        // Verify no duplicates
        let mut seen = std::collections::HashSet::new();
        for mode_name in &mode_names {
            assert!(seen.insert(mode_name.clone()), "Duplicate mode found: {}", mode_name);
        }

        // Verify specific modes exist
        assert!(mode_names.contains(&"Local repos".to_string()));
        assert!(mode_names.contains(&"Github - work".to_string()));
        assert!(mode_names.contains(&"Github - personal".to_string()));
        
        // Should not have two "Github - work" entries
        let work_count = mode_names.iter().filter(|name| *name == "Github - work").count();
        assert_eq!(work_count, 1, "Should have exactly one 'Github - work' mode");
    }
}
