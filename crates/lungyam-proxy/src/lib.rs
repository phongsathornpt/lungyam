//! Native proxy runtime for Lungyam.

use lungyam_core::PROJECT_NAME;

/// Returns the runtime banner used during bootstrap.
#[must_use]
pub fn runtime_banner() -> String {
    format!("{PROJECT_NAME} proxy")
}

#[cfg(test)]
mod tests {
    use super::runtime_banner;

    #[test]
    fn banner_identifies_proxy_runtime() {
        assert_eq!(runtime_banner(), "Lungyam proxy");
    }
}
