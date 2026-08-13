use std::{fmt::Write as _, net::SocketAddr, sync::OnceLock};

use lungyam_core::config::Config;

static CSRF_TOKEN: OnceLock<CsrfToken> = OnceLock::new();

#[derive(Debug)]
pub(crate) struct CsrfToken(String);

impl CsrfToken {
    fn generate() -> Result<Self, getrandom::Error> {
        let mut bytes = [0_u8; 32];
        getrandom::fill(&mut bytes)?;
        let mut token = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            write!(&mut token, "{byte:02x}").expect("writing into a String cannot fail");
        }
        Ok(Self(token))
    }

    pub(crate) fn expose(&self) -> &str {
        &self.0
    }

    pub(crate) fn verify(&self, candidate: &str) -> bool {
        constant_time_eq(self.0.as_bytes(), candidate.as_bytes())
    }
}

pub(crate) fn csrf_token() -> &'static CsrfToken {
    CSRF_TOKEN.get_or_init(|| {
        CsrfToken::generate().expect("failed to obtain system entropy for admin CSRF token")
    })
}

pub(crate) fn writes_enabled(config: &Config) -> bool {
    !config.admin.read_only
        && config
            .admin
            .listen
            .parse::<SocketAddr>()
            .is_ok_and(|address| address.ip().is_loopback())
}

fn constant_time_eq(expected: &[u8], candidate: &[u8]) -> bool {
    if expected.len() != candidate.len() {
        return false;
    }

    expected
        .iter()
        .zip(candidate)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use lungyam_core::config::{AdminConfig, Config, ServerConfig, UpstreamConfig};

    use super::{CsrfToken, writes_enabled};

    #[test]
    fn generated_token_is_hex_and_verifies_exactly() {
        let token = CsrfToken::generate().expect("system entropy");
        assert_eq!(token.expose().len(), 64);
        assert!(token.expose().bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert!(token.verify(token.expose()));
        assert!(!token.verify("00"));

        let mut changed = token.expose().as_bytes().to_vec();
        changed[0] = if changed[0] == b'a' { b'b' } else { b'a' };
        let changed = String::from_utf8(changed).expect("ascii token");
        assert!(!token.verify(&changed));
    }

    #[test]
    fn writes_require_explicit_opt_in_and_loopback_listener() {
        let mut config = test_config();
        assert!(!writes_enabled(&config));

        config.admin.read_only = false;
        assert!(writes_enabled(&config));

        config.admin.listen = "0.0.0.0:9090".to_owned();
        assert!(!writes_enabled(&config));
    }

    fn test_config() -> Config {
        let mut upstreams = BTreeMap::new();
        upstreams.insert(
            "api".to_owned(),
            UpstreamConfig {
                endpoints: vec!["127.0.0.1:3000".to_owned()],
                connect_timeout_ms: None,
                read_timeout_ms: None,
                write_timeout_ms: None,
                health_check_interval_seconds: 5,
            },
        );
        Config {
            server: ServerConfig {
                listen: "127.0.0.1:8080".to_owned(),
            },
            admin: AdminConfig {
                enabled: true,
                listen: "127.0.0.1:9090".to_owned(),
                read_only: true,
            },
            upstreams,
            routes: Vec::new(),
        }
    }
}
