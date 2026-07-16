//! Full-screen terminal control plane for a Switchyard project.

mod app;
mod terminal;
mod views;

use std::{error::Error, path::Path};

/// Opens the project TUI and restores the terminal before returning.
pub fn run(project_dir: &Path) -> Result<(), Box<dyn Error>> {
    let project_dir = project_dir.canonicalize()?;
    let mut terminal = terminal::TerminalSession::enter()?;
    let mut app = app::App::load(project_dir)?;
    while !app.should_quit() {
        terminal.draw(|frame| views::render(frame, &app))?;
        if crossterm::event::poll(std::time::Duration::from_millis(100))? {
            app.handle_event(crossterm::event::read()?);
        }
        app.tick();
    }
    Ok(())
}
