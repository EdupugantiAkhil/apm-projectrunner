//! AppCUI background-operation bridge.

use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Mutex, mpsc},
    thread,
};

use appcui::{
    prelude::{BackgroundTaskConector, Handle, Window},
    system::BackgroundTask,
};
use switchyard_ops::execution::{self, OperationEvent, OperationSpec};

static OPERATION_JOBS: Mutex<VecDeque<OperationJob>> = Mutex::new(VecDeque::new());

#[derive(Clone, Debug)]
pub(crate) struct OperationJob {
    pub(crate) project_dir: PathBuf,
    pub(crate) label: String,
    pub(crate) deployment: Option<String>,
    pub(crate) destructive: bool,
    pub(crate) spec: OperationSpec,
}

/// A background operation update delivered to the UI thread.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum OpUpdate {
    Started {
        label: String,
        deployment: Option<String>,
        destructive: bool,
    },
    Output(String),
    Finished(i32),
    Failed(String),
}

/// AppCUI requires a response type even though operations do not query the UI.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OpCommand {
    Continue,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct OperationGate {
    running: bool,
}

impl OperationGate {
    pub(crate) fn try_start(&mut self) -> Result<(), &'static str> {
        if self.running {
            Err("Another project operation is already running.")
        } else {
            self.running = true;
            Ok(())
        }
    }

    pub(crate) fn finish(&mut self) {
        self.running = false;
    }

    pub(crate) fn is_running(self) -> bool {
        self.running
    }
}

pub(crate) fn start(
    job: OperationJob,
    receiver: Handle<Window>,
) -> Result<Handle<BackgroundTask<OpUpdate, OpCommand>>, String> {
    OPERATION_JOBS
        .lock()
        .map_err(|_| "operation task queue is unavailable".to_owned())?
        .push_back(job);
    Ok(BackgroundTask::run(execute, receiver))
}

fn execute(connector: &BackgroundTaskConector<OpUpdate, OpCommand>) {
    let job = OPERATION_JOBS
        .lock()
        .ok()
        .and_then(|mut jobs| jobs.pop_front());
    let Some(job) = job else {
        connector.notify(OpUpdate::Failed(
            "background operation started without a queued job".into(),
        ));
        return;
    };
    connector.notify(OpUpdate::Started {
        label: job.label,
        deployment: job.deployment,
        destructive: job.destructive,
    });

    let (sender, events) = mpsc::channel();
    let worker = thread::spawn(move || execution::run(&job.project_dir, job.spec, &sender));
    while let Ok(event) = events.recv() {
        let terminal = matches!(
            event,
            OperationEvent::Finished { .. } | OperationEvent::Failed(_)
        );
        connector.notify(match event {
            OperationEvent::Output(line) => OpUpdate::Output(line),
            OperationEvent::Finished { exit_code } => OpUpdate::Finished(exit_code),
            OperationEvent::Failed(error) => OpUpdate::Failed(error),
        });
        if terminal {
            break;
        }
    }
    let _ = worker.join();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_one_operation_can_enter_the_gate() {
        let mut gate = OperationGate::default();
        assert_eq!(gate.try_start(), Ok(()));
        assert!(gate.is_running());
        assert_eq!(
            gate.try_start(),
            Err("Another project operation is already running.")
        );
        gate.finish();
        assert_eq!(gate.try_start(), Ok(()));
    }
}
