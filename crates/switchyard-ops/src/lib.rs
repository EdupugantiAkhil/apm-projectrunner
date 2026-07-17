//! Operations and read-model projections shared by Switchyard's interactive clients.
//!
//! This crate owns no view or command-line parsing concerns and must never depend on
//! `ratatui`, `crossterm`, or `clap`.

pub mod connections;
pub mod execution;
pub mod instances;
pub mod profiles;
pub mod projections;
pub mod run_scripts;

pub use connections::{
    ConnectionMatrix, ConnectionRow, ProviderDetail, RouteChange, RouteHistoryEntry, RouteStatus,
    SwitchPreview, connection_matrix, project_route_status, route_status, switch_preview,
};
pub use execution::{OperationEvent, OperationSpec};
pub use instances::{
    CreateInstanceError, CreateInstanceRequest, CreatedInstance, InstancePreview, create_instance,
    preview_instance,
};
pub use profiles::{
    DiscoveredSourceProfiles, ProfileAdapterKind, ProfileContentHash, ProfileError, ProfileListing,
    ProfileOrigin, ProfileRow, ProfileService, ProfileTrust, SourceManifestError,
    SourceProfileManifest, discover_source_profiles, import_source_profile, list_profiles,
    load_profile_block, load_source_profile_block, project_profile_rows, remove_imported_profile,
};
pub use projections::{
    BindingRow, DefinitionHeader, DefinitionMetadata, DefinitionTopology, DeploymentEntry,
    InstanceRow, ManifestService, ServiceManifest, ServiceRow, SourceChoice, list_deployments,
    list_devices, list_sources, load_definition_choices,
};
pub use run_scripts::{RunScript, StructuredCommand};
