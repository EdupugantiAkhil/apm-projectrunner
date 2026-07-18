//! Full-screen terminal control plane for a Switchyard project.

mod handoff;
mod shell;
mod state;
mod tabs;
pub mod tasks;

use std::{error::Error, path::Path};

use handoff::{ExitOutcome, execute_interactive_clone};

/// Opens the project TUI and restores the terminal before returning.
pub fn run(project_dir: &Path) -> Result<(), Box<dyn Error>> {
    let project_dir = project_dir.canonicalize()?;
    loop {
        let outcome = shell::run_app(&project_dir)?;
        match outcome {
            ExitOutcome::Exit => return Ok(()),
            ExitOutcome::CloneHandoff(request) => {
                execute_interactive_clone(&project_dir, request);
            }
        }
    }
}
