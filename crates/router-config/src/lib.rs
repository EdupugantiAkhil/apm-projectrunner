//! Versioned configuration contracts for the Switchyard router.
//!
//! Consumers should name the schema module they support. The root re-exports are only
//! conveniences for code which intentionally follows the current schema.

pub mod v1alpha1;

pub use v1alpha1::*;
