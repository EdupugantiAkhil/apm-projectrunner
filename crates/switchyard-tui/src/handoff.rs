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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CloneHandoff {
    pub(crate) name: String,
    pub(crate) url: String,
    pub(crate) git_ref: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
#[allow(dead_code)] // TODO(part 2): the Code tab constructs clone handoff outcomes.
pub(crate) enum ExitOutcome {
    #[default]
    Exit,
    CloneHandoff(CloneHandoff),
}

#[derive(Clone, Default)]
pub(crate) struct OutcomeCell(Rc<RefCell<ExitOutcome>>);

impl OutcomeCell {
    #[allow(dead_code)] // TODO(part 2): called by the Code tab before closing the shell.
    pub(crate) fn request_clone(&self, request: CloneHandoff) {
        *self.0.borrow_mut() = ExitOutcome::CloneHandoff(request);
    }

    pub(crate) fn take(&self) -> ExitOutcome {
        std::mem::take(&mut *self.0.borrow_mut())
    }
}

/// Runs Git only after AppCUI has returned and restored the real terminal.
pub(crate) fn execute_interactive_clone(root: &Path, request: CloneHandoff) {
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
    if let Err(error) = result {
        eprintln!("\nSwitchyard: {error}");
        print!("Press Enter to return to the TUI…");
        let _ = io::stdout().flush();
        let mut input = String::new();
        let _ = io::stdin().read_line(&mut input);
    }
}

fn clone_source(root: &Path, request: CloneHandoff) -> Result<(), String> {
    // This intentionally preserves the pre-rewrite clone handoff. New UI reads and
    // mutations use switchyard-ops.
    // TODO(part 2): use an ops-owned clone entry point once one is available.
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

fn unique_source_name(store: &StateStore, base: &str) -> Result<String, String> {
    if store
        .source(base)
        .map_err(|error| error.to_string())?
        .is_none()
    {
        return Ok(base.to_owned());
    }
    for suffix in 2..=10_000 {
        let candidate = format!("{base}-{suffix}");
        if store
            .source(&candidate)
            .map_err(|error| error.to_string())?
            .is_none()
        {
            return Ok(candidate);
        }
    }
    Err(format!(
        "could not find an available source name based on `{base}`"
    ))
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
