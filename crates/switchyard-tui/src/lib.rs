//! Full-screen terminal control plane for a Switchyard project.

mod dialogs;
mod handoff;
mod shell;
mod state;
mod tabs;
pub mod tasks;

use std::{env, error::Error, os::unix::process::CommandExt, path::Path, process::Command};

use handoff::{ExitOutcome, OutcomeCell, execute_interactive_clone};

/// Carries the post-handoff notice across the re-exec boundary.
const REOPEN_ENV: &str = "SWITCHYARD_TUI_REOPEN_CODE";
const NOTICE_ENV: &str = "SWITCHYARD_TUI_CODE_NOTICE";

/// Opens the project TUI and restores the terminal before returning.
pub fn run(project_dir: &Path) -> Result<(), Box<dyn Error>> {
    let project_dir = project_dir.canonicalize()?;
    let outcome = OutcomeCell::default();
    if env::var_os(REOPEN_ENV).is_some() {
        outcome.restore_after_restart(env::var(NOTICE_ENV).ok());
    }
    let exit = shell::run_app(&project_dir, outcome.clone())?;
    match exit {
        ExitOutcome::Exit => Ok(()),
        ExitOutcome::CloneHandoff(request) => {
            let result = execute_interactive_clone(&project_dir, request);
            // A second AppCUI instance in one process leaks the previous
            // backend input thread, which then steals keystrokes; replace the
            // process instead so the restarted TUI owns stdin alone.
            let exe = env::current_exe()?;
            let error = Command::new(exe)
                .args(env::args_os().skip(1))
                .env(REOPEN_ENV, "1")
                .env(NOTICE_ENV, handoff::clone_result_notice(&result))
                .exec();
            Err(Box::new(error))
        }
    }
}
