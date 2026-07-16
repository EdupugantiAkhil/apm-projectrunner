//! Full-screen terminal control plane for a Switchyard project.

mod app;
mod execution;
mod run_scripts;
mod terminal;
mod views;

use signal_hook::{consts::SIGINT, flag, low_level};
use std::{
    error::Error,
    io::{self, Write},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

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
        if let Some(request) = app.take_pending_clone() {
            let root = app.project_dir.clone();
            let result = terminal.suspend(|| {
                println!("Switchyard yielded the terminal to Git.\n");
                let interrupted = Arc::new(AtomicBool::new(false));
                let signal_id = flag::register(SIGINT, Arc::clone(&interrupted));
                let result = match signal_id {
                    Ok(signal_id) => {
                        let result = app::execute_interactive_clone(&root, request);
                        low_level::unregister(signal_id);
                        if interrupted.load(Ordering::Relaxed) && result.is_err() {
                            Err("Git clone was interrupted".into())
                        } else {
                            result
                        }
                    }
                    Err(error) => Err(format!("could not prepare native Git prompt: {error}")),
                };
                if let Err(error) = &result {
                    eprintln!("\nSwitchyard: {error}");
                    print!("Press Enter to return to the TUI…");
                    let _ = io::stdout().flush();
                    let mut input = String::new();
                    let _ = io::stdin().read_line(&mut input);
                }
                result
            })?;
            app.finish_interactive_clone(result);
        }
    }
    Ok(())
}
