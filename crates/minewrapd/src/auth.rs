//! Token-auth parity with v1 `auth.py`: a constant-time compare of the
//! bearer token against an allow-list; failures are 401 at REST and the
//! WS handshake is denied before `accept`.

/// Auth context: the daemon's bearer token(s). The token is stored in
/// the OS keyring by the GUI or written once to `data/token` (0600) by
/// the daemon (v1 `config_store.py`), and is never logged.
#[derive(Clone)]
pub struct TokenAuth {
    token: String,
}

impl TokenAuth {
    pub fn new(token: String) -> Self {
        Self { token }
    }

    pub fn verify(&self, presented: &str) -> bool {
        if presented.len() != self.token.len() {
            return false;
        }
        // Constant-time compare (v1 `auth.py` parity via
        // `hmac.compare_digest`): compare every byte, fold the
        // differences, never early-exit on a mismatch.
        let mut diff: u8 = 0;
        for i in 0..presented.len() {
            diff |= presented.as_bytes()[i] ^ self.token.as_bytes()[i];
        }
        diff == 0
    }

    /// Generate a fresh token (secrets crate equivalent: 32 random bytes,
    /// hex-encoded). No external crate — `getrandom` isn't a dep yet, so
    /// this uses `/dev/urandom` (Linux-only, matches the desktop target).
    pub fn generate() -> String {
        let mut bytes = [0u8; 32];
        if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
            use std::io::Read;
            let _ = f.read_exact(&mut bytes);
        }
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut out = String::with_capacity(64);
        for b in bytes {
            out.push(HEX[(b >> 4) as usize] as char);
            out.push(HEX[(b & 0xF) as usize] as char);
        }
        out
    }
}

/// Extract the bearer token from an `Authorization: Bearer …` header
/// value. Returns `None` when absent/malformed.
pub fn bearer_from_header(header: &str) -> Option<String> {
    let t = header.trim();
    let prefix = "Bearer ";
    t.strip_prefix(prefix).map(|s| s.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_rejects_wrong_and_missing() {
        let auth = TokenAuth::new("s3cr3t".to_string());
        assert!(auth.verify("s3cr3t"));
        assert!(!auth.verify("s3cr3tX"));
        assert!(!auth.verify("s3cr3"));
        assert!(!auth.verify("wrong"));
    }

    #[test]
    fn bearer_parses() {
        assert_eq!(bearer_from_header("Bearer abc").as_deref(), Some("abc"));
        assert_eq!(bearer_from_header("Bearer  abc  ").as_deref(), Some("abc"));
        assert_eq!(bearer_from_header("abc"), None);
    }
}
