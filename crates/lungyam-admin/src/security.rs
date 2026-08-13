use std::{fmt::Write as _, sync::Arc};

#[derive(Clone, Debug)]
pub(crate) struct CsrfToken(Arc<str>);

impl CsrfToken {
    pub(crate) fn generate() -> Result<Self, getrandom::Error> {
        let mut bytes = [0_u8; 32];
        getrandom::fill(&mut bytes)?;
        let mut token = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            write!(&mut token, "{byte:02x}").expect("writing into a String cannot fail");
        }
        Ok(Self(Arc::from(token)))
    }

    pub(crate) fn expose(&self) -> &str {
        &self.0
    }

    pub(crate) fn verify(&self, candidate: &str) -> bool {
        constant_time_eq(self.0.as_bytes(), candidate.as_bytes())
    }
}

fn constant_time_eq(expected: &[u8], candidate: &[u8]) -> bool {
    if expected.len() != candidate.len() {
        return false;
    }

    expected
        .iter()
        .zip(candidate)
        .fold(0_u8, |difference, (left, right)| difference | (left ^ right))
        == 0
}

#[cfg(test)]
mod tests {
    use super::CsrfToken;

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
}
