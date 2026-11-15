//! What an event is to whoever sends one, apart from the route it is sent to.
//!
//! A routine standing on `event:stripe/invoice.payment_failed` holds no slot
//! and nothing sweeps it. What fires it is a POST to
//! `/events/stripe/invoice.payment_failed` on the server's own origin, the route
//! `server::events` serves, which hands the two identifiers and the body to
//! `Runtime::deliver_event` and answers with what became of them. That is the
//! whole of the event source: Guaca polls no service and signs in to none for
//! this, and whatever posts there is the operator's to wire.
//!
//! What lives here is what that route shares with the rest of the app: the
//! secret a post carries, written into the settings at boot, the most a body
//! may be, and how a delivery is told apart from its retries.
//!
//! # A secret in a header
//!
//! The secret is a bearer token in the `Authorization` header, and that
//! placement is the mechanism rather than a convention: a browser will not
//! attach that header cross-origin without a preflight, so a page open in the
//! operator's browser cannot fire a routine by posting a form. It is the
//! webhook's own credential and grants nothing else, and the workspace token
//! does not fire a routine either.
//!
//! # What the body is
//!
//! Data. It is handed to the routine's part untouched and the plain-text
//! projection fences it under a line that says so, because the instruction
//! carries the operator's authority and the body carries none: it is whatever
//! the service put on the wire. Capped at [`MOST_BODY_BYTES`], which is more
//! than a vendor's event and less than what a turn should spend reading.

use crate::config::WebhookConfig;

/// The most a body may be. Everything under it reaches the agent whole.
pub const MOST_BODY_BYTES: usize = 64 * 1024;

pub fn delivery_id(values: [Option<&[u8]>; 3]) -> Result<String, &'static str> {
    let Some(value) = values.into_iter().flatten().next() else {
        return Ok(format!("generated:{}", uuid::Uuid::new_v4()));
    };
    let start = value.iter().position(|byte| !byte.is_ascii_whitespace()).unwrap_or(value.len());
    let end = value.iter().rposition(|byte| !byte.is_ascii_whitespace()).map_or(start, |i| i + 1);
    let value = &value[start..end];
    if value.len() > 200 {
        return Err("delivery ID must be at most 200 bytes");
    }
    use std::fmt::Write;
    let mut encoded = String::with_capacity(7 + value.len() * 2);
    encoded.push_str("header:");
    for byte in value {
        write!(encoded, "{byte:02x}").expect("writing to a string cannot fail");
    }
    Ok(encoded)
}

/// Gives the config a secret if it has none. Answers whether anything changed,
/// so the caller can write the file once rather than on every launch.
pub fn prepare(config: &mut WebhookConfig) -> bool {
    if !config.secret.trim().is_empty() {
        return false;
    }
    config.secret = crate::oauth::secret();
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delivery_ids_use_header_precedence_and_are_bounded() {
        assert_eq!(
            delivery_id([Some(b"  first  "), Some(b"second"), Some(b"third")]).unwrap(),
            "header:6669727374"
        );
        assert_eq!(
            delivery_id([None, Some(b"second"), Some(b"third")]).unwrap(),
            "header:7365636f6e64"
        );
        assert_eq!(delivery_id([None, None, Some(b"third")]).unwrap(), "header:7468697264");
        assert_eq!(delivery_id([Some(&[b'x'; 200]), None, None]).unwrap().len(), 407);
        assert!(delivery_id([Some(&[b'x'; 201]), Some(b"fallback"), None]).is_err());
        assert!(delivery_id([Some("é".repeat(101).as_bytes()), None, None]).is_err());
        assert_eq!(delivery_id([Some(b"  "), None, None]).unwrap(), "header:");
        assert_eq!(delivery_id([Some(b"\xff"), None, None]).unwrap(), "header:ff");
        let first = delivery_id([None, None, None]).unwrap();
        let second = delivery_id([None, None, None]).unwrap();
        assert!(uuid::Uuid::parse_str(first.strip_prefix("generated:").unwrap()).is_ok());
        assert_ne!(first, second);
    }

    #[test]
    fn preparing_writes_a_secret_once() {
        let mut config = WebhookConfig::default();
        assert!(prepare(&mut config), "an empty config needs a secret");
        let first = config.secret.clone();
        assert!(first.len() >= 40, "256 bits of base64url: {first:?}");
        assert!(!prepare(&mut config), "a second launch keeps the secret it wrote");
        assert_eq!(config.secret, first);
    }
}
