use std::fs::File;
use std::io::Read;

/// Mint a random session token. The dashboard embeds it in the served page and
/// requires it on every API request; this is the primary defense against other
/// pages in the user's browser (or a rebound DNS name) driving the API.
///
/// We read 16 bytes (128 bits) from `/dev/urandom` and fail closed if that is
/// unavailable — there is deliberately no guessable time/pid fallback, and no
/// extra crate is pulled in for this. The dashboard is therefore unix-only.
pub fn generate_token() -> Result<String, String> {
    let mut buf = [0u8; 16];
    File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut buf))
        .map_err(|e| format!("Cannot read /dev/urandom for a session token: {e}"))?;
    Ok(hex_encode(&buf))
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

/// Authorize an API request. Requires all of:
/// - a loopback `Host` header (blocks DNS-rebinding, which still carries the
///   attacker's host name),
/// - a loopback `Origin` when one is present (blocks cross-origin callers),
/// - a token matching the server's, supplied either via the `X-Berth-Token`
///   header (POSTs) or the `token` query parameter (SSE/`EventSource`, which
///   cannot set headers).
pub fn authorize(
    host: Option<&str>,
    origin: Option<&str>,
    token: Option<&str>,
    expected: &str,
) -> bool {
    if !host.map(is_loopback_host).unwrap_or(false) {
        return false;
    }

    if let Some(origin) = origin {
        if !origin_is_loopback(origin) {
            return false;
        }
    }

    token == Some(expected)
}

/// Whether an HTTP `Host` value (with optional port) names the loopback host.
pub fn is_loopback_host(host: &str) -> bool {
    let host = host.trim();

    let hostname = if let Some(rest) = host.strip_prefix('[') {
        // Bracketed IPv6 literal, e.g. `[::1]:4600`.
        match rest.split_once(']') {
            Some((inner, _)) => inner,
            None => return false,
        }
    } else if host.matches(':').count() > 1 {
        // Bare IPv6 literal; a `host:port` form has exactly one colon.
        host
    } else {
        // `host` or `host:port`; hostnames/IPv4 contain at most one colon.
        host.rsplit_once(':').map(|(h, _)| h).unwrap_or(host)
    };

    hostname == "127.0.0.1" || hostname == "localhost" || hostname == "::1"
}

/// Whether an `Origin` value (a URL) points at the loopback host.
fn origin_is_loopback(origin: &str) -> bool {
    let authority = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
        .unwrap_or(origin)
        .split('/')
        .next()
        .unwrap_or("");

    is_loopback_host(authority)
}

/// Extract a query-string parameter from a request URL (`/path?a=1&b=2`).
pub fn query_param<'a>(url: &'a str, key: &str) -> Option<&'a str> {
    let query = url.split_once('?')?.1;
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(k, _)| *k == key)
        .map(|(_, v)| v)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "deadbeef";

    #[test]
    fn hex_encode_pads_and_orders() {
        assert_eq!(hex_encode(&[0x00, 0x0f, 0xa0, 0xff]), "000fa0ff");
    }

    #[test]
    fn loopback_hosts_accepted() {
        for h in [
            "127.0.0.1",
            "127.0.0.1:4600",
            "localhost",
            "localhost:8080",
            "[::1]:4600",
            "::1",
        ] {
            assert!(is_loopback_host(h), "{h} should be loopback");
        }
    }

    #[test]
    fn non_loopback_hosts_rejected() {
        for h in ["example.com", "evil.com:4600", "10.0.0.5", "192.168.1.2:80"] {
            assert!(!is_loopback_host(h), "{h} should not be loopback");
        }
    }

    #[test]
    fn authorize_requires_loopback_host() {
        assert!(!authorize(None, None, Some(TOKEN), TOKEN));
        assert!(!authorize(Some("evil.com"), None, Some(TOKEN), TOKEN));
        assert!(authorize(Some("127.0.0.1:4600"), None, Some(TOKEN), TOKEN));
    }

    #[test]
    fn authorize_rejects_cross_origin() {
        assert!(!authorize(
            Some("localhost:4600"),
            Some("http://evil.com"),
            Some(TOKEN),
            TOKEN
        ));
        assert!(authorize(
            Some("localhost:4600"),
            Some("http://localhost:4600"),
            Some(TOKEN),
            TOKEN
        ));
    }

    #[test]
    fn authorize_requires_matching_token() {
        assert!(!authorize(Some("localhost"), None, None, TOKEN));
        assert!(!authorize(Some("localhost"), None, Some("wrong"), TOKEN));
        assert!(authorize(Some("localhost"), None, Some(TOKEN), TOKEN));
    }

    #[test]
    fn query_param_extracts_value() {
        assert_eq!(query_param("/api/x?token=abc", "token"), Some("abc"));
        assert_eq!(
            query_param("/api/x?a=1&token=abc&b=2", "token"),
            Some("abc")
        );
        assert_eq!(query_param("/api/x", "token"), None);
        assert_eq!(query_param("/api/x?a=1", "token"), None);
    }
}
