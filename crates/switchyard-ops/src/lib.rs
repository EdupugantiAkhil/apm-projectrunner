//! Operations and read-model projections shared by Switchyard's interactive clients.
//!
//! This crate owns no view or command-line parsing concerns and must never depend on
//! `ratatui`, `crossterm`, or `clap`.

pub mod execution;
pub mod projections;
pub mod run_scripts;

pub use execution::{OperationEvent, OperationSpec};
pub use projections::{
    BindingRow, DefinitionHeader, DefinitionMetadata, DefinitionTopology, DeploymentEntry,
    InstanceRow, ManifestService, ServiceManifest, ServiceRow, SourceChoice, list_deployments,
    list_devices, list_sources, load_definition_choices,
};
pub use run_scripts::{RunScript, StructuredCommand};
