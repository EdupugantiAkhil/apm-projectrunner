use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    widgets::{Block, Borders, Paragraph},
};

pub(super) fn render(frame: &mut Frame<'_>, area: Rect) {
    frame.render_widget(
        Paragraph::new("Instances view is coming in a later release.")
            .style(Style::default().fg(Color::DarkGray))
            .block(Block::default().borders(Borders::ALL).title(" Instances ")),
        area,
    );
}
