//! Versioned configuration contracts for the APM ProjectRunner router.
//!
//! Consumers should name the schema module they support. The root re-exports are only
//! conveniences for code which intentionally follows the current schema.

pub mod v1alpha1;

pub use v1alpha1::*;

/// Environment prefix used before the rename to APM ProjectRunner.
const RENAMED_ENVIRONMENT_PREFIX: &str = "SWITCHYARD_";
/// Environment prefix that replaced [`RENAMED_ENVIRONMENT_PREFIX`].
const ENVIRONMENT_PREFIX: &str = "APMPR_";

/// Reports every `SWITCHYARD_*` name in `names`, as sorted `OLD -> NEW` pairs.
///
/// The rename to APM ProjectRunner moved these to `APMPR_*`. A stale variable would
/// otherwise be silently ignored, which for `SWITCHYARD_ROUTER_TOKEN` reads as an
/// unexplained "must be set" failure while the value sits right there under its old name.
///
/// Only names are examined and returned. Values are never read: one of these variables is a
/// router token, so it must not be echoed, and a name that is not valid UTF-8 cannot carry
/// the prefix and is skipped rather than failing.
pub fn renamed_environment_variables<I, S>(names: I) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let mut stale: Vec<String> = names
        .into_iter()
        .filter_map(|name| {
            let suffix = name
                .as_ref()
                .to_str()?
                .strip_prefix(RENAMED_ENVIRONMENT_PREFIX)?;
            Some(format!(
                "{RENAMED_ENVIRONMENT_PREFIX}{suffix} -> {ENVIRONMENT_PREFIX}{suffix}"
            ))
        })
        .collect();
    stale.sort();
    stale
}

/// Reads the current environment and formats the [`renamed_environment_variables`] report.
///
/// Returns `None` when no stale variable is set. Every shipped binary calls this before doing
/// any work, so a renamed variable is reported the same way whichever one the user invoked.
pub fn stale_environment_error() -> Option<String> {
    let stale = renamed_environment_variables(std::env::vars_os().map(|(name, _)| name));
    (!stale.is_empty()).then(|| {
        format!(
            "Switchyard was renamed to APM ProjectRunner, so these environment variables are no longer read; rename them and retry:\n  {}",
            stale.join("\n  ")
        )
    })
}

#[cfg(test)]
mod rename_tests {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    #[test]
    fn renamed_variables_report_names_without_reading_values() {
        let stale = super::renamed_environment_variables([
            OsString::from("SWITCHYARD_ROUTER_TOKEN"),
            OsString::from("PATH"),
            OsString::from("SWITCHYARD_BUNDLE"),
            OsString::from("APMPR_ROUTER_TOKEN"),
            // Not valid UTF-8, so it cannot carry the prefix. It must be skipped rather
            // than panicking the process that is only trying to report stale names.
            OsString::from_vec(b"\xffNOT_UTF8".to_vec()),
        ]);
        assert_eq!(
            stale,
            vec![
                "SWITCHYARD_BUNDLE -> APMPR_BUNDLE".to_owned(),
                "SWITCHYARD_ROUTER_TOKEN -> APMPR_ROUTER_TOKEN".to_owned(),
            ]
        );
        assert!(super::renamed_environment_variables([OsString::from("APMPR_BUNDLE")]).is_empty());
    }
}
