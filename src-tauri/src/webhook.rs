use crate::config::WebhookConfig;

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

pub fn prepare(config: &mut WebhookConfig) -> bool {
    if !config.secret.expose().trim().is_empty() {
        return false;
    }
    config.secret = crate::oauth::secret().into();
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
        assert!(first.expose().len() >= 40, "256 bits of base64url");
        assert!(!prepare(&mut config), "a second launch keeps the secret it wrote");
        assert_eq!(config.secret, first);
    }
}
