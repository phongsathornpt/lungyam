//! Core domain types and policies shared by Lungyam runtimes.

pub mod config;
pub mod config_diff;
pub mod lifecycle;
pub mod revision;
pub mod revision_state;
pub mod routing;
pub mod runtime;
pub mod store;

/// Human-readable project name used by runtime adapters.
pub const PROJECT_NAME: &str = "Lungyam";

#[cfg(test)]
mod tests {
    use super::PROJECT_NAME;

    #[test]
    fn project_name_is_stable() {
        assert_eq!(PROJECT_NAME, "Lungyam");
    }
}
