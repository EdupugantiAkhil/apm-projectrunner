//! Messages shared by AppCUI background operations and the shell.

/// A background operation update delivered to the UI thread.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OpUpdate {
    Started(String),
    Output(String),
    Finished(Result<(), String>),
}

/// A command sent from the UI thread to a background operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OpCommand {
    Continue,
    Cancel,
}
