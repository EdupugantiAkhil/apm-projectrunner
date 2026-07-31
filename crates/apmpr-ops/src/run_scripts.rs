//! Compatibility re-exports for the shared project run-action domain.

pub use apmpr_run_actions::{
    FILE_NAME, RunActionError, RunScript, StructuredCommand, acknowledge_shell_notice, create,
    delete, load, load_result, save, save_result, shell_notice_acknowledged, update, validate_name,
};
