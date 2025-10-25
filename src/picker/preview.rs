use crossterm::style::Colored;
use ratatui::{
    buffer::Buffer,
    layout::{Direction, Rect},
    style::{Color, Style, Stylize},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph, Widget, Wrap},
};

pub struct PreviewWidget {
    buffer: String,
    border_color: Color,
    direction: Direction,
}

impl PreviewWidget {
    pub fn new(buffer: String, border_color: Color, direction: Direction) -> Self {
        Self {
            buffer,
            border_color,
            direction,
        }
    }
}

impl Widget for PreviewWidget {
    fn render(self, area: Rect, buf: &mut Buffer)
    where
        Self: Sized,
    {
        let text = str_to_text(&self.buffer, (area.width - 1).into());
        let border_position = if self.direction == Direction::Horizontal {
            Borders::LEFT
        } else {
            Borders::BOTTOM
        };

        Paragraph::new(text)
            .block(
                Block::default()
                    .borders(border_position)
                    .border_style(Style::default().fg(self.border_color)),
            )
            .wrap(Wrap { trim: false })
            .render(area, buf);
    }
}

fn str_to_text(s: &str, max: usize) -> Text<'_> {
    let mut text = Text::default();
    let mut style = Style::default();
    let mut tspan = String::new();
    let mut ansi_state;

    for l in s.lines() {
        let mut line = Line::default();
        ansi_state = false;

        for (i, ch) in l.chars().enumerate() {
            if !ansi_state {
                if ch == '\x1b' && l.chars().nth(i + 1) == Some('[') {
                    if !tspan.is_empty() {
                        let span = Span::styled(tspan.clone(), style);
                        line.spans.push(span);
                    }

                    tspan.clear();
                    ansi_state = true;
                } else {
                    tspan.push(ch);

                    if (line.width() + tspan.chars().count()) == max || i == (l.chars().count() - 1)
                    {
                        let span = Span::styled(tspan.clone(), style);
                        line.spans.push(span);
                        tspan.clear();
                        break;
                    }
                }
            } else {
                match ch {
                    '[' => {}
                    'm' => {
                        style = match tspan.as_str() {
                            "" => style.reset(),
                            "0" => style.reset(),
                            "1" => style.bold(),
                            "3" => style.italic(),
                            "4" => style.underlined(),
                            "5" => style.rapid_blink(),
                            "6" => style.slow_blink(),
                            "7" => style.reversed(),
                            "9" => style.crossed_out(),
                            "22" => style.not_bold(),
                            "23" => style.not_italic(),
                            "24" => style.not_underlined(),
                            "25" => style.not_rapid_blink().not_slow_blink(),
                            "27" => style.not_reversed(),
                            "29" => style.not_crossed_out(),
                            "30" => style.fg(Color::Black),
                            "31" => style.fg(Color::Red),
                            "32" => style.fg(Color::Green),
                            "33" => style.fg(Color::Yellow),
                            "34" => style.fg(Color::Blue),
                            "35" => style.fg(Color::Magenta),
                            "36" => style.fg(Color::Cyan),
                            "37" => style.fg(Color::Gray),
                            "40" => style.bg(Color::Black),
                            "41" => style.bg(Color::Red),
                            "42" => style.bg(Color::Green),
                            "43" => style.bg(Color::Yellow),
                            "44" => style.bg(Color::Blue),
                            "45" => style.bg(Color::Magenta),
                            "46" => style.bg(Color::Cyan),
                            "47" => style.bg(Color::Gray),
                            "90" | "100" => style.fg(Color::DarkGray),
                            "91" | "101" => style.fg(Color::LightRed),
                            "92" | "102" => style.fg(Color::LightGreen),
                            "93" | "103" => style.fg(Color::LightYellow),
                            "94" | "104" => style.fg(Color::LightBlue),
                            "95" | "105" => style.fg(Color::LightMagenta),
                            "96" | "106" => style.fg(Color::LightCyan),
                            "97" | "107" => style.fg(Color::White),
                            code => {
                                if let Some(colored) = Colored::parse_ansi(code) {
                                    match colored {
                                        Colored::ForegroundColor(c) => {
                                            // Convert crossterm::Color to ratatui::Color
                                            let ratatui_color = match c {
                                                crossterm::style::Color::Black => Color::Black,
                                                crossterm::style::Color::DarkGrey => Color::DarkGray,
                                                crossterm::style::Color::Red => Color::Red,
                                                crossterm::style::Color::DarkRed => Color::Red,
                                                crossterm::style::Color::Green => Color::Green,
                                                crossterm::style::Color::DarkGreen => Color::Green,
                                                crossterm::style::Color::Yellow => Color::Yellow,
                                                crossterm::style::Color::DarkYellow => Color::Yellow,
                                                crossterm::style::Color::Blue => Color::Blue,
                                                crossterm::style::Color::DarkBlue => Color::Blue,
                                                crossterm::style::Color::Magenta => Color::Magenta,
                                                crossterm::style::Color::DarkMagenta => Color::Magenta,
                                                crossterm::style::Color::Cyan => Color::Cyan,
                                                crossterm::style::Color::DarkCyan => Color::Cyan,
                                                crossterm::style::Color::White => Color::White,
                                                crossterm::style::Color::Grey => Color::Gray,
                                                crossterm::style::Color::Rgb { r, g, b } => Color::Rgb(r, g, b),
                                                crossterm::style::Color::AnsiValue(v) => Color::Indexed(v),
                                                _ => Color::Reset,
                                            };
                                            style.fg(ratatui_color)
                                        },
                                        Colored::BackgroundColor(c) => {
                                            // Convert crossterm::Color to ratatui::Color
                                            let ratatui_color = match c {
                                                crossterm::style::Color::Black => Color::Black,
                                                crossterm::style::Color::DarkGrey => Color::DarkGray,
                                                crossterm::style::Color::Red => Color::Red,
                                                crossterm::style::Color::DarkRed => Color::Red,
                                                crossterm::style::Color::Green => Color::Green,
                                                crossterm::style::Color::DarkGreen => Color::Green,
                                                crossterm::style::Color::Yellow => Color::Yellow,
                                                crossterm::style::Color::DarkYellow => Color::Yellow,
                                                crossterm::style::Color::Blue => Color::Blue,
                                                crossterm::style::Color::DarkBlue => Color::Blue,
                                                crossterm::style::Color::Magenta => Color::Magenta,
                                                crossterm::style::Color::DarkMagenta => Color::Magenta,
                                                crossterm::style::Color::Cyan => Color::Cyan,
                                                crossterm::style::Color::DarkCyan => Color::Cyan,
                                                crossterm::style::Color::White => Color::White,
                                                crossterm::style::Color::Grey => Color::Gray,
                                                crossterm::style::Color::Rgb { r, g, b } => Color::Rgb(r, g, b),
                                                crossterm::style::Color::AnsiValue(v) => Color::Indexed(v),
                                                _ => Color::Reset,
                                            };
                                            style.bg(ratatui_color)
                                        },
                                        Colored::UnderlineColor(c) => {
                                            // Convert crossterm::Color to ratatui::Color
                                            let ratatui_color = match c {
                                                crossterm::style::Color::Black => Color::Black,
                                                crossterm::style::Color::DarkGrey => Color::DarkGray,
                                                crossterm::style::Color::Red => Color::Red,
                                                crossterm::style::Color::DarkRed => Color::Red,
                                                crossterm::style::Color::Green => Color::Green,
                                                crossterm::style::Color::DarkGreen => Color::Green,
                                                crossterm::style::Color::Yellow => Color::Yellow,
                                                crossterm::style::Color::DarkYellow => Color::Yellow,
                                                crossterm::style::Color::Blue => Color::Blue,
                                                crossterm::style::Color::DarkBlue => Color::Blue,
                                                crossterm::style::Color::Magenta => Color::Magenta,
                                                crossterm::style::Color::DarkMagenta => Color::Magenta,
                                                crossterm::style::Color::Cyan => Color::Cyan,
                                                crossterm::style::Color::DarkCyan => Color::Cyan,
                                                crossterm::style::Color::White => Color::White,
                                                crossterm::style::Color::Grey => Color::Gray,
                                                crossterm::style::Color::Rgb { r, g, b } => Color::Rgb(r, g, b),
                                                crossterm::style::Color::AnsiValue(v) => Color::Indexed(v),
                                                _ => Color::Reset,
                                            };
                                            style.underline_color(ratatui_color)
                                        }
                                    }
                                } else {
                                    style
                                }
                            }
                        };

                        tspan.clear();
                        ansi_state = false;
                    }
                    _ => tspan.push(ch),
                }
            }
        }

        text.lines.push(line);
    }

    text
}
