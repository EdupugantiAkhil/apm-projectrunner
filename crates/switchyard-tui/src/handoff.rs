use signal_hook::{consts::SIGINT, flag, low_level};
use std::{
    cell::RefCell,
    io::{self, Write},
    path::Path,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use switchyard_state::StateStore;

use crate::state::unique_source_name;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CloneHandoff {
    pub(crate) name: String,
    pub(crate) url: String,
    pub(crate) git_ref: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) enum ExitOutcome {
    #[default]
    Exit,
    CloneHandoff(CloneHandoff),
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct RestartContext {
    pub(crate) code_notice: Option<String>,
    pub(crate) reopen_code: bool,
}

#[derive(Clone, Default)]
pub(crate) struct OutcomeCell {
    outcome: Rc<RefCell<ExitOutcome>>,
    restart: Rc<RefCell<RestartContext>>,
}

impl OutcomeCell {
    pub(crate) fn request_clone(&self, request: CloneHandoff) {
        *self.outcome.borrow_mut() = ExitOutcome::CloneHandoff(request);
        self.restart.borrow_mut().reopen_code = true;
    }

    pub(crate) fn take(&self) -> ExitOutcome {
        std::mem::take(&mut *self.outcome.borrow_mut())
    }

    pub(crate) fn restart_context(&self) -> RestartContext {
        self.restart.borrow().clone()
    }

    /// Marks this process as a post-handoff restart (context arrives through
    /// the environment because the TUI re-execs itself after running Git).
    pub(crate) fn restore_after_restart(&self, notice: Option<String>) {
        let mut restart = self.restart.borrow_mut();
        restart.reopen_code = true;
        restart.code_notice = notice;
    }
}

/// The Code-tab notice shown after the post-clone restart.
pub(crate) fn clone_result_notice(result: &Result<(), String>) -> String {
    match result {
        Ok(()) => "Source cloned and registered successfully.".into(),
        Err(error) => format!("Clone failed: {error}"),
    }
}

/// Runs Git only after AppCUI has returned and restored the real terminal.
pub(crate) fn execute_interactive_clone(root: &Path, request: CloneHandoff) -> Result<(), String> {
    println!("Switchyard yielded the terminal to Git.\n");
    let interrupted = Arc::new(AtomicBool::new(false));
    let signal_id = flag::register(SIGINT, Arc::clone(&interrupted));
    let result = match signal_id {
        Ok(signal_id) => {
            let result = clone_source(root, request);
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
}

fn clone_source(root: &Path, request: CloneHandoff) -> Result<(), String> {
    // This intentionally preserves the pre-rewrite clone handoff. New UI reads and
    // mutations use switchyard-ops.
    // The terminal-handoff clone remains on the existing sources lifecycle API
    // because it must run after AppCUI restores the native terminal.
    let store = StateStore::open(root.join(".switchyard/state.sqlite3"))
        .map_err(|error| error.to_string())?
        .0;
    let manager = switchyard_sources::SourceManager::new(root);
    let name = unique_source_name(&store, &request.name)?;
    manager
        .create_clone_from_url_interactive(&store, &request.url, &name, request.git_ref.as_deref())
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_defaults_to_exit() {
        assert_eq!(OutcomeCell::default().take(), ExitOutcome::Exit);
    }

    #[test]
    fn clone_request_round_trips_through_shared_cell() {
        let cell = OutcomeCell::default();
        let observer = cell.clone();
        let request = CloneHandoff {
            name: "api".into(),
            url: "git@example.test:team/api.git".into(),
            git_ref: Some("feature/demo".into()),
        };
        cell.request_clone(request.clone());
        assert_eq!(observer.take(), ExitOutcome::CloneHandoff(request));
        assert_eq!(cell.take(), ExitOutcome::Exit);
    }
}
