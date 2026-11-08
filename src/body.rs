use std::io::{self, Read};

use serde_json::{Map, Value};

pub const MAX_BODY: usize = 64 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub enum BodyError {
    UnsupportedMediaType,
    TooLarge,
    Invalid(&'static str),
}

/// Reads at most `limit + 1` bytes so callers can tell an oversized body apart.
pub fn read_limited(reader: impl Read, limit: usize) -> io::Result<Vec<u8>> {
    let mut buf = Vec::new();
    reader.take(limit as u64 + 1).read_to_end(&mut buf)?;
    Ok(buf)
}

/// Validates a request body and parses it as a JSON object.
/// Returns `Ok(None)` when the body is empty.
pub fn parse_json_object(
    content_type: Option<&str>,
    body: &[u8],
) -> Result<Option<Map<String, Value>>, BodyError> {
    if body.is_empty() {
        return Ok(None);
    }
    if !is_json(content_type) {
        return Err(BodyError::UnsupportedMediaType);
    }
    if body.len() > MAX_BODY {
        return Err(BodyError::TooLarge);
    }
    match serde_json::from_slice::<Value>(body) {
        Ok(Value::Object(map)) => Ok(Some(map)),
        Ok(_) => Err(BodyError::Invalid("body must be a JSON object")),
        Err(_) => Err(BodyError::Invalid("invalid JSON body")),
    }
}

fn is_json(content_type: Option<&str>) -> bool {
    content_type
        .and_then(|ct| ct.split(';').next())
        .is_some_and(|media| media.trim().eq_ignore_ascii_case("application/json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const JSON: Option<&str> = Some("application/json");

    #[test]
    fn empty_body_is_none_regardless_of_type() {
        assert_eq!(parse_json_object(None, b""), Ok(None));
        assert_eq!(parse_json_object(Some("text/plain"), b""), Ok(None));
    }

    #[test]
    fn content_type() {
        let ok = |ct| parse_json_object(Some(ct), b"{}").is_ok();
        assert!(ok("application/json"));
        assert!(ok("Application/JSON; charset=utf-8"));
        assert!(ok(" application/json ;charset=utf-8"));
        assert_eq!(
            parse_json_object(Some("text/plain"), b"{}"),
            Err(BodyError::UnsupportedMediaType)
        );
        assert_eq!(
            parse_json_object(None, b"{}"),
            Err(BodyError::UnsupportedMediaType)
        );
    }

    #[test]
    fn size_limit_boundary() {
        let fits = format!("{{\"m\":\"{}\"}}", "a".repeat(MAX_BODY - 8));
        assert_eq!(fits.len(), MAX_BODY);
        assert!(parse_json_object(JSON, fits.as_bytes()).is_ok());

        let over = format!("{{\"m\":\"{}\"}}", "a".repeat(MAX_BODY - 7));
        assert_eq!(
            parse_json_object(JSON, over.as_bytes()),
            Err(BodyError::TooLarge)
        );
    }

    #[test]
    fn read_limited_stops_after_limit_plus_one() {
        let data = [b'x'; 100];
        assert_eq!(read_limited(&data[..], 10).unwrap().len(), 11);
        assert_eq!(read_limited(&data[..], 200).unwrap().len(), 100);
    }

    #[test]
    fn rejects_non_objects_and_trailing_data() {
        for body in [&b"[]"[..], b"\"x\"", b"null", b"1", b"{}{}", b"{", b"{} x"] {
            assert!(
                matches!(parse_json_object(JSON, body), Err(BodyError::Invalid(_))),
                "{:?}",
                String::from_utf8_lossy(body)
            );
        }
    }

    #[test]
    fn trailing_whitespace_is_fine() {
        assert!(parse_json_object(JSON, b"{} \n").is_ok());
    }
}
