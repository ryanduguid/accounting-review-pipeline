//! Internal disclosure algorithms. The removed Python library API is not exposed.
use crate::{
    entities::{self, Entity},
    patterns,
    pyjson::{self, Json, PyStr},
    verify,
};
use sha2::{Digest, Sha256};

pub const MAX_RECORD_BYTES: usize = 4096;
pub const SCHEMA: &str = "evatt.disclosure-evidence/v1";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug)]
pub enum Error {
    Invalid(String),
    Refused(&'static str),
}

fn invalid(message: &str) -> Error {
    Error::Invalid(message.into())
}

pub fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

fn context(destination: &str, decision: &str) -> Result<(), Error> {
    if !patterns::full("_DESTINATION", destination).map_err(Error::Invalid)? {
        return Err(invalid(
            "destination must be a canonical opaque identifier of 1 to 128 characters",
        ));
    }
    if !patterns::full("_DECISION_REF", decision).map_err(Error::Invalid)? {
        return Err(invalid(
            "decision reference must be an opaque identifier of 1 to 128 characters",
        ));
    }
    Ok(())
}

fn evidence(payload: &[u8], entries: &[Entity], destination: &str, decision: &str) -> Result<Json, Error> {
    context(destination, decision)?;
    if payload.starts_with(b"\xef\xbb\xbf") {
        return Err(invalid("disclosure payload must be UTF-8 without a BOM"));
    }
    let text = std::str::from_utf8(payload).map_err(|_| invalid("disclosure payload must be UTF-8 without a BOM"))?;
    let mapping = entities::document(entries);
    entities::load(&mapping).map_err(|_| invalid("invalid entity map for disclosure evidence"))?;
    if !verify::findings(text, entries).map_err(Error::Invalid)?.is_empty() {
        return Err(Error::Refused("the disclosure payload has verifier findings"));
    }
    let mut encoded = b"evatt.entity-map/v1\0".to_vec();
    encoded.extend_from_slice(pyjson::canonical(&mapping).as_bytes());
    Ok(Json::object([
        ("schema", Json::str(SCHEMA)),
        ("output_sha256", Json::str(&digest(payload))),
        ("entity_map_sha256", Json::str(&digest(&encoded))),
        ("tool_version", Json::str(VERSION)),
        ("destination", Json::str(destination)),
        ("decision_ref", Json::str(decision)),
    ]))
}

pub fn create(payload: &[u8], entries: &[Entity], destination: &str, decision: &str) -> Result<Vec<u8>, Error> {
    let record = evidence(payload, entries, destination, decision)?;
    Ok((pyjson::pretty(&record) + "\n").into_bytes())
}

fn parse(record: &[u8]) -> Result<Json, Error> {
    if record.len() > MAX_RECORD_BYTES {
        return Err(invalid("disclosure record must be at most 4096 bytes of UTF-8 JSON"));
    }
    let text = std::str::from_utf8(record).map_err(|_| invalid("invalid disclosure record JSON"))?;
    let document = pyjson::loads_record(text).map_err(|_| invalid("invalid disclosure record JSON"))?;
    if !document.has_exact_keys(&[
        "schema",
        "output_sha256",
        "entity_map_sha256",
        "tool_version",
        "destination",
        "decision_ref",
    ]) {
        return Err(invalid("invalid disclosure record fields"));
    }
    let Json::Object(pairs) = &document else { unreachable!() };
    if pairs.iter().any(|(_, v)| v.as_pystr().is_none_or(|s| s.0.len() > 128)) {
        return Err(invalid("invalid disclosure record field types or lengths"));
    }
    if !document.at("schema").is_str(SCHEMA) {
        return Err(invalid("unsupported disclosure record schema"));
    }
    for key in ["output_sha256", "entity_map_sha256"] {
        let digest = document.at(key).as_pystr().and_then(PyStr::to_utf8).unwrap_or_default();
        if !patterns::full("_DIGEST", &digest).map_err(Error::Invalid)? {
            return Err(invalid("invalid disclosure record digest"));
        }
    }
    let get = |key| document.at(key).as_pystr().and_then(PyStr::to_utf8).unwrap_or_default();
    context(&get("destination"), &get("decision_ref"))?;
    let version = get("tool_version");
    if !patterns::full("_TOOL_VERSION", &version).map_err(Error::Invalid)? {
        return Err(invalid("invalid disclosure record tool version"));
    }
    Ok(document)
}

pub fn check(
    payload: &[u8],
    entries: &[Entity],
    record: &[u8],
    destination: &str,
    decision: &str,
) -> Result<(), Error> {
    let document = parse(record)?;
    let expected = evidence(payload, entries, destination, decision)?;
    if !pyjson::py_eq(&document, &expected, false) {
        return Err(Error::Refused(
            "disclosure evidence does not match the supplied context",
        ));
    }
    Ok(())
}
