//! Native Pingora-based proxy runtime for Lungyam.

use async_trait::async_trait;
use lungyam_core::PROJECT_NAME;
use pingora::prelude::*;

/// A minimal proxy that forwards every request to one plain-HTTP upstream.
#[derive(Debug, Clone)]
pub struct SingleUpstreamProxy {
    upstream: String,
}

impl SingleUpstreamProxy {
    /// Creates a single-upstream proxy policy.
    #[must_use]
    pub fn new(upstream: impl Into<String>) -> Self {
        Self {
            upstream: upstream.into(),
        }
    }

    /// Returns the configured upstream address.
    #[must_use]
    pub fn upstream(&self) -> &str {
        &self.upstream
    }
}

#[async_trait]
impl ProxyHttp for SingleUpstreamProxy {
    type CTX = ();

    fn new_ctx(&self) -> Self::CTX {}

    async fn upstream_peer(
        &self,
        _session: &mut Session,
        _ctx: &mut Self::CTX,
    ) -> Result<Box<HttpPeer>> {
        Ok(Box::new(HttpPeer::new(
            self.upstream.clone(),
            false,
            String::new(),
        )))
    }
}

/// Starts the native Lungyam proxy and blocks until the server exits.
pub fn run(listen: &str, upstream: &str) {
    let mut server = Server::new(None).expect("failed to create Pingora server");
    server.bootstrap();

    let mut service = http_proxy_service(
        &server.configuration,
        SingleUpstreamProxy::new(upstream),
    );
    service.add_tcp(listen);
    server.add_service(service);

    server.run_forever();
}

/// Returns the runtime banner used during bootstrap.
#[must_use]
pub fn runtime_banner() -> String {
    format!("{PROJECT_NAME} proxy")
}

#[cfg(test)]
mod tests {
    use super::{SingleUpstreamProxy, runtime_banner};

    #[test]
    fn banner_identifies_proxy_runtime() {
        assert_eq!(runtime_banner(), "Lungyam proxy");
    }

    #[test]
    fn proxy_keeps_upstream_address() {
        let proxy = SingleUpstreamProxy::new("127.0.0.1:3000");
        assert_eq!(proxy.upstream(), "127.0.0.1:3000");
    }
}
