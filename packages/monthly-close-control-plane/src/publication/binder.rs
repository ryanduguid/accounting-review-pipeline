// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! Tamper-evident close binder (stored ZIP + SHA-256 manifest + optional hash-chained audit log).

use crate::{
    Result, publication,
    pyjson::{self, Json, PyStr},
    sha256,
    viewer::{self, PACK_NAMES},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

pub const MANIFEST_SCHEMA: &str = "close-binder-manifest-v1";
pub const MANIFEST_PATH: &str = "manifest.json";
pub const AUDIT_CHAIN_PATH: &str = "audit-chain.jsonl";
pub const GENESIS_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";
pub const BOUNDARY: &str = "Sealed close evidence binder only. It does not approve a close, post a journal, make a payment, lodge a return, or lock a period.";

const LOCAL_FILE_SIG: u32 = 0x0403_4b50;
const CENTRAL_DIR_SIG: u32 = 0x0201_4b50;
const EOCD_SIG: u32 = 0x0605_4b50;
const ZIP_VERSION: u16 = 20;
const UTF8_FLAG: u16 = 0x0800;
const DOS_TIME: u16 = 0;
const DOS_DATE: u16 = 0x0021; // 1980-01-01 for deterministic archives.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuditEventInput {
    pub timestamp: String,
    pub actor: String,
    pub action: String,
    pub target: String,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuditChainSummary {
    pub count: usize,
    pub head_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BinderAttachment {
    pub archive_path: String,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BinderRequest {
    pub pack_dir: PathBuf,
    pub output_zip: PathBuf,
    pub attachments: Vec<BinderAttachment>,
    pub audit_chain: Option<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BinderAnchor {
    pub manifest_sha256: String,
    pub audit_chain_head: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BinderVerification {
    pub pack_status: String,
    pub entry_count: usize,
    pub source_hashes: BTreeMap<String, String>,
    pub entry_hashes: BTreeMap<String, String>,
    pub manifest_sha256: String,
    pub audit_chain_head: Option<String>,
}

impl BinderVerification {
    pub fn anchor(&self) -> BinderAnchor {
        BinderAnchor {
            manifest_sha256: self.manifest_sha256.clone(),
            audit_chain_head: self.audit_chain_head.clone(),
        }
    }
}

type EntryMap = BTreeMap<String, Vec<u8>>;
type DigestMap = BTreeMap<String, String>;

struct BuiltManifest {
    bytes: Vec<u8>,
    pack_status: String,
    source_hashes: DigestMap,
    entry_hashes: DigestMap,
}

struct CollectedEntries {
    entries: EntryMap,
    audit_head: Option<String>,
}

struct EmbeddedCheck {
    pack_status: String,
    source_hashes: DigestMap,
    audit_chain_head: Option<String>,
}

fn non_empty_field(value: &str, name: &str) -> Result<()> {
    if value.trim().is_empty() {
        return Err(format!("Audit event field '{name}' must not be blank."));
    }
    Ok(())
}

fn audit_payload(seq: u64, event: &AuditEventInput, prev_sha256: &str) -> Json {
    Json::object([
        ("seq", Json::int(seq as i64)),
        ("timestamp", Json::str(&event.timestamp)),
        ("actor", Json::str(&event.actor)),
        ("action", Json::str(&event.action)),
        ("target", Json::str(&event.target)),
        ("detail", Json::str(&event.detail)),
        ("prev_sha256", Json::str(prev_sha256)),
    ])
}

pub fn build_audit_chain(events: &[AuditEventInput]) -> Result<Vec<u8>> {
    if events.is_empty() {
        return Err("Audit chain must contain at least one event.".into());
    }
    let mut out = String::new();
    let mut prev = GENESIS_HASH.to_owned();
    for (idx, event) in events.iter().enumerate() {
        non_empty_field(&event.timestamp, "timestamp")?;
        non_empty_field(&event.actor, "actor")?;
        non_empty_field(&event.action, "action")?;
        non_empty_field(&event.target, "target")?;
        non_empty_field(&event.detail, "detail")?;
        let seq = (idx as u64) + 1;
        let payload = audit_payload(seq, event, &prev);
        let entry_sha256 = sha256(pyjson::canonical(&payload).as_bytes());
        let mut full = payload;
        full.set("entry_sha256", Json::str(&entry_sha256));
        out.push_str(&pyjson::canonical(&full));
        out.push('\n');
        prev = entry_sha256;
    }
    Ok(out.into_bytes())
}

fn json_str(obj: &Json, key: &str, label: &str) -> Result<String> {
    obj.get(key)
        .and_then(Json::as_pystr)
        .and_then(PyStr::to_utf8)
        .ok_or_else(|| format!("{label}: field '{key}' must be UTF-8 text."))
}

const AUDIT_KEYS: [&str; 8] = [
    "seq",
    "timestamp",
    "actor",
    "action",
    "target",
    "detail",
    "prev_sha256",
    "entry_sha256",
];

fn verify_audit_line(line: &str, expected_seq: i64, expected_prev: &str) -> Result<String> {
    let record = pyjson::loads(line)
        .map_err(|_| format!("Audit chain line {expected_seq} is not valid JSON."))?;
    if !record.has_exact_keys(&AUDIT_KEYS) {
        return Err(format!(
            "Audit chain line {expected_seq} has unexpected keys."
        ));
    }
    let seq = record
        .at("seq")
        .as_plain_int()
        .and_then(|v| i64::try_from(v).ok());
    if seq != Some(expected_seq) {
        return Err(format!(
            "Audit chain line {expected_seq} has invalid sequence number."
        ));
    }
    let label = format!("audit-chain line {expected_seq}");
    let event = AuditEventInput {
        timestamp: json_str(&record, "timestamp", &label)?,
        actor: json_str(&record, "actor", &label)?,
        action: json_str(&record, "action", &label)?,
        target: json_str(&record, "target", &label)?,
        detail: json_str(&record, "detail", &label)?,
    };
    if json_str(&record, "prev_sha256", &label)? != expected_prev {
        return Err(format!(
            "Audit chain broken at line {expected_seq}: prev_sha256 mismatch."
        ));
    }
    let computed = sha256(
        pyjson::canonical(&audit_payload(expected_seq as u64, &event, expected_prev)).as_bytes(),
    );
    if json_str(&record, "entry_sha256", &label)? != computed {
        return Err(format!(
            "Audit chain tampered at line {expected_seq}: entry_sha256 mismatch."
        ));
    }
    Ok(computed)
}

pub fn verify_audit_chain(bytes: &[u8]) -> Result<AuditChainSummary> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| "Audit chain must be valid UTF-8 JSONL.".to_owned())?;
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return Err("Audit chain must contain at least one event.".into());
    }
    let mut prev = GENESIS_HASH.to_owned();
    for (idx, line) in lines.iter().enumerate() {
        prev = verify_audit_line(line, (idx as i64) + 1, &prev)?;
    }
    Ok(AuditChainSummary {
        count: lines.len(),
        head_sha256: prev,
    })
}

pub fn validate_archive_path(path: &str) -> Result<()> {
    if path.is_empty()
        || path.starts_with('/')
        || path.ends_with('/')
        || path.contains('\\')
        || path.bytes().any(|b| b < 0x20 || b == 0x7f)
    {
        return Err(format!("Invalid archive path: '{path}'."));
    }
    for segment in path.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." {
            return Err(format!("Invalid archive path segment in '{path}'."));
        }
    }
    Ok(())
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc: u32 = 0xffff_ffff;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

fn push_local_header(out: &mut Vec<u8>, name: &[u8], data: &[u8], checksum: u32) {
    let name_len = u16::try_from(name.len()).expect("archive name fits in u16");
    let data_len = u32::try_from(data.len()).expect("archive entry fits in u32");
    out.extend_from_slice(&LOCAL_FILE_SIG.to_le_bytes());
    out.extend_from_slice(&ZIP_VERSION.to_le_bytes());
    out.extend_from_slice(&UTF8_FLAG.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&DOS_TIME.to_le_bytes());
    out.extend_from_slice(&DOS_DATE.to_le_bytes());
    out.extend_from_slice(&checksum.to_le_bytes());
    out.extend_from_slice(&data_len.to_le_bytes());
    out.extend_from_slice(&data_len.to_le_bytes());
    out.extend_from_slice(&name_len.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(name);
    out.extend_from_slice(data);
}

fn push_central_header(out: &mut Vec<u8>, name: &[u8], data_len: u32, checksum: u32, offset: u32) {
    let name_len = u16::try_from(name.len()).expect("archive name fits in u16");
    out.extend_from_slice(&CENTRAL_DIR_SIG.to_le_bytes());
    out.extend_from_slice(&ZIP_VERSION.to_le_bytes());
    out.extend_from_slice(&ZIP_VERSION.to_le_bytes());
    out.extend_from_slice(&UTF8_FLAG.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&DOS_TIME.to_le_bytes());
    out.extend_from_slice(&DOS_DATE.to_le_bytes());
    out.extend_from_slice(&checksum.to_le_bytes());
    out.extend_from_slice(&data_len.to_le_bytes());
    out.extend_from_slice(&data_len.to_le_bytes());
    out.extend_from_slice(&name_len.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&offset.to_le_bytes());
    out.extend_from_slice(name);
}

fn write_zip(entries: &BTreeMap<String, Vec<u8>>) -> Vec<u8> {
    let mut archive = Vec::new();
    let mut central = Vec::new();
    let count = u16::try_from(entries.len()).expect("entry count fits in u16");
    for (name, data) in entries {
        let offset = u32::try_from(archive.len()).expect("archive fits in u32");
        let checksum = crc32(data);
        let data_len = u32::try_from(data.len()).expect("entry fits in u32");
        push_local_header(&mut archive, name.as_bytes(), data, checksum);
        push_central_header(&mut central, name.as_bytes(), data_len, checksum, offset);
    }
    let cd_offset = u32::try_from(archive.len()).expect("archive fits in u32");
    let cd_size = u32::try_from(central.len()).expect("central directory fits in u32");
    archive.extend_from_slice(&central);
    archive.extend_from_slice(&EOCD_SIG.to_le_bytes());
    archive.extend_from_slice(&0u16.to_le_bytes());
    archive.extend_from_slice(&0u16.to_le_bytes());
    archive.extend_from_slice(&count.to_le_bytes());
    archive.extend_from_slice(&count.to_le_bytes());
    archive.extend_from_slice(&cd_size.to_le_bytes());
    archive.extend_from_slice(&cd_offset.to_le_bytes());
    archive.extend_from_slice(&0u16.to_le_bytes());
    archive
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16> {
    let slice = bytes
        .get(offset..offset + 2)
        .ok_or_else(|| "Truncated ZIP archive.".to_owned())?;
    Ok(u16::from_le_bytes([slice[0], slice[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32> {
    let slice = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| "Truncated ZIP archive.".to_owned())?;
    Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn read_local_entry(
    bytes: &[u8],
    offset: usize,
    expected_name: &str,
    expected_crc: u32,
    expected_size: usize,
) -> Result<Vec<u8>> {
    if read_u32(bytes, offset)? != LOCAL_FILE_SIG {
        return Err("ZIP local file header signature mismatch.".into());
    }
    if read_u16(bytes, offset + 8)? != 0 {
        return Err("Binder ZIP supports stored (uncompressed) entries only.".into());
    }
    let crc = read_u32(bytes, offset + 14)?;
    let comp = read_u32(bytes, offset + 18)? as usize;
    let uncomp = read_u32(bytes, offset + 22)? as usize;
    let name_len = read_u16(bytes, offset + 26)? as usize;
    let extra_len = read_u16(bytes, offset + 28)? as usize;
    let name_end = offset + 30 + name_len;
    let name_bytes = bytes
        .get(offset + 30..name_end)
        .ok_or_else(|| "Truncated ZIP local entry name.".to_owned())?;
    let name = std::str::from_utf8(name_bytes)
        .map_err(|_| "ZIP entry name must be valid UTF-8.".to_owned())?;
    if name != expected_name
        || crc != expected_crc
        || comp != expected_size
        || uncomp != expected_size
    {
        return Err(format!("ZIP local header mismatch for '{expected_name}'."));
    }
    let data_start = name_end + extra_len;
    let payload = bytes
        .get(data_start..data_start + expected_size)
        .ok_or_else(|| format!("Truncated ZIP payload for '{expected_name}'."))?;
    if crc32(payload) != expected_crc {
        return Err(format!("ZIP CRC-32 mismatch for '{expected_name}'."));
    }
    Ok(payload.to_vec())
}

fn read_central_entry(bytes: &[u8], cursor: usize) -> Result<(String, Vec<u8>, usize)> {
    if read_u32(bytes, cursor)? != CENTRAL_DIR_SIG {
        return Err("ZIP central directory header signature mismatch.".into());
    }
    if read_u16(bytes, cursor + 10)? != 0 {
        return Err("Binder ZIP supports stored (uncompressed) entries only.".into());
    }
    let crc = read_u32(bytes, cursor + 16)?;
    let comp = read_u32(bytes, cursor + 20)? as usize;
    let uncomp = read_u32(bytes, cursor + 24)? as usize;
    let name_len = read_u16(bytes, cursor + 28)? as usize;
    let extra_len = read_u16(bytes, cursor + 30)? as usize;
    let comment_len = read_u16(bytes, cursor + 32)? as usize;
    let local_offset = read_u32(bytes, cursor + 42)? as usize;
    let name_end = cursor + 46 + name_len;
    let name_bytes = bytes
        .get(cursor + 46..name_end)
        .ok_or_else(|| "Truncated ZIP central directory name.".to_owned())?;
    let name = std::str::from_utf8(name_bytes)
        .map_err(|_| "ZIP entry name must be valid UTF-8.".to_owned())?;
    validate_archive_path(name)?;
    if comp != uncomp {
        return Err(format!("ZIP entry '{name}' has mismatched stored sizes."));
    }
    let payload = read_local_entry(bytes, local_offset, name, crc, uncomp)?;
    Ok((name.to_owned(), payload, name_end + extra_len + comment_len))
}

fn read_zip(bytes: &[u8]) -> Result<BTreeMap<String, Vec<u8>>> {
    if bytes.len() < 22 {
        return Err("ZIP archive is too short.".into());
    }
    let eocd = bytes.len() - 22;
    if read_u32(bytes, eocd)? != EOCD_SIG || read_u16(bytes, eocd + 20)? != 0 {
        return Err("ZIP end-of-central-directory record is missing or unsupported.".into());
    }
    let count = usize::from(read_u16(bytes, eocd + 10)?);
    let cd_size = read_u32(bytes, eocd + 12)? as usize;
    let cd_offset = read_u32(bytes, eocd + 16)? as usize;
    if cd_offset + cd_size != eocd {
        return Err("ZIP central directory bounds do not match archive size.".into());
    }
    let mut cursor = cd_offset;
    let mut entries = BTreeMap::new();
    for _ in 0..count {
        let (name, payload, next) = read_central_entry(bytes, cursor)?;
        if entries.insert(name.clone(), payload).is_some() {
            return Err(format!("ZIP contains duplicate entry '{name}'."));
        }
        cursor = next;
    }
    if cursor != eocd {
        return Err("ZIP central directory contains unexpected trailing bytes.".into());
    }
    Ok(entries)
}

fn read_source_hashes(document: &Json) -> Result<BTreeMap<String, String>> {
    let pairs = document
        .get("source_sha256")
        .and_then(Json::as_object)
        .ok_or_else(|| "Pack document missing source_sha256 object.".to_owned())?;
    let mut map = BTreeMap::new();
    for (key, value) in pairs {
        let name = key
            .to_utf8()
            .ok_or_else(|| "source_sha256 key must be UTF-8.".to_owned())?;
        let digest = value
            .as_pystr()
            .and_then(PyStr::to_utf8)
            .ok_or_else(|| "source_sha256 digest must be UTF-8.".to_owned())?;
        map.insert(name, digest);
    }
    Ok(map)
}

fn build_manifest(
    pack: &viewer::VerifiedPack,
    entries: &BTreeMap<String, Vec<u8>>,
    audit_head: Option<&str>,
) -> Result<BuiltManifest> {
    let pack_status = json_str(&pack.document, "overall_status", "close-review-pack.json")?;
    let source_hashes = read_source_hashes(&pack.document)?;
    let mut entry_hashes = BTreeMap::new();
    let mut manifest_entries = Vec::with_capacity(entries.len());
    for (path, data) in entries {
        let digest = sha256(data);
        manifest_entries.push(Json::object([
            ("path", Json::str(path)),
            ("bytes", Json::int(data.len() as i64)),
            ("sha256", Json::str(&digest)),
        ]));
        entry_hashes.insert(path.clone(), digest);
    }
    let entries_json = Json::Array(manifest_entries);
    let entries_digest = sha256(pyjson::canonical(&entries_json).as_bytes());
    let mut manifest = Json::object([
        ("schema", Json::str(MANIFEST_SCHEMA)),
        ("boundary", Json::str(BOUNDARY)),
        ("pack_status", Json::str(&pack_status)),
        (
            "current_report_dates",
            pack.document.at("current_report_dates").clone(),
        ),
        (
            "prior_report_dates",
            pack.document.at("prior_report_dates").clone(),
        ),
        ("source_sha256", pack.document.at("source_sha256").clone()),
        ("entries", entries_json),
        ("manifest_entries_sha256", Json::str(&entries_digest)),
    ]);
    if let Some(head) = audit_head {
        manifest.set("audit_chain_head_sha256", Json::str(head));
    }
    let bytes = format!("{}\n", pyjson::pretty_utf8(&manifest)?).into_bytes();
    Ok(BuiltManifest {
        bytes,
        pack_status,
        source_hashes,
        entry_hashes,
    })
}

fn write_file_sync(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

fn write_atomic_with(
    path: &Path,
    bytes: &[u8],
    write_fn: &mut impl FnMut(&Path, &[u8]) -> io::Result<()>,
    rename_fn: &mut impl FnMut(&Path, &Path) -> io::Result<()>,
) -> Result<()> {
    if path.exists() {
        return Err(format!(
            "Refusing to overwrite existing binder archive: {}.",
            path.display()
        ));
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or_else(|| "Binder output path must have a parent directory.".to_owned())?;
    fs::create_dir_all(parent)
        .map_err(|e| format!("Cannot create binder directory {}: {e}.", parent.display()))?;
    let mut random = [0u8; 6];
    getrandom::fill(&mut random).expect("OS entropy source");
    let suffix: String = random.iter().map(|b| format!("{b:02x}")).collect();
    let temp = parent.join(format!(".binder-{suffix}.partial"));
    if let Err(err) = write_fn(&temp, bytes) {
        let _ = fs::remove_file(&temp);
        return Err(format!("Cannot write binder archive: {err}."));
    }
    rename_fn(&temp, path).map_err(|err| {
        let _ = fs::remove_file(&temp);
        format!("Cannot finalise binder archive {}: {err}.", path.display())
    })
}

fn collect_entries(
    request: &BinderRequest,
    verified: &viewer::VerifiedPack,
    read_fn: &mut impl FnMut(&Path) -> io::Result<Vec<u8>>,
) -> Result<CollectedEntries> {
    let mut entries = BTreeMap::new();
    for name in PACK_NAMES {
        let data = read_fn(&request.pack_dir.join(name))
            .map_err(|e| format!("Cannot read verified pack artefact '{name}': {e}."))?;
        if verified.artefact_sha256.get(name).map(String::as_str) != Some(sha256(&data).as_str()) {
            return Err(format!(
                "Pack artefact '{name}' changed during binder export."
            ));
        }
        entries.insert(name.to_owned(), data);
    }
    let mut reserved: BTreeSet<&str> = PACK_NAMES.into_iter().collect();
    reserved.insert(MANIFEST_PATH);
    reserved.insert(AUDIT_CHAIN_PATH);
    for attachment in &request.attachments {
        validate_archive_path(&attachment.archive_path)?;
        if reserved.contains(attachment.archive_path.as_str())
            || entries.contains_key(&attachment.archive_path)
        {
            return Err(format!(
                "Attachment path '{}' conflicts with a reserved or duplicate binder entry.",
                attachment.archive_path
            ));
        }
        entries.insert(attachment.archive_path.clone(), attachment.bytes.clone());
    }
    let audit_head = if let Some(chain) = &request.audit_chain {
        let summary = verify_audit_chain(chain)?;
        entries.insert(AUDIT_CHAIN_PATH.to_owned(), chain.clone());
        Some(summary.head_sha256)
    } else {
        None
    };
    Ok(CollectedEntries {
        entries,
        audit_head,
    })
}

pub fn export_binder(request: &BinderRequest) -> Result<BinderVerification> {
    let target = publication::require_output_outside_repository(&request.output_zip)?;
    let verified = viewer::verify_pack(&request.pack_dir)?;
    let CollectedEntries {
        mut entries,
        audit_head,
    } = collect_entries(request, &verified, &mut |p| fs::read(p))?;
    let BuiltManifest {
        bytes: manifest_bytes,
        pack_status,
        source_hashes,
        entry_hashes,
    } = build_manifest(&verified, &entries, audit_head.as_deref())?;
    let manifest_sha256 = sha256(&manifest_bytes);
    entries.insert(MANIFEST_PATH.to_owned(), manifest_bytes);
    let zip_bytes = write_zip(&entries);
    write_atomic_with(&target, &zip_bytes, &mut write_file_sync, &mut |a, b| {
        fs::rename(a, b)
    })?;
    Ok(BinderVerification {
        pack_status,
        entry_count: entry_hashes.len(),
        source_hashes,
        entry_hashes,
        manifest_sha256,
        audit_chain_head: audit_head,
    })
}

fn verify_manifest_entries(
    manifest: &Json,
    entries: &BTreeMap<String, Vec<u8>>,
) -> Result<BTreeMap<String, String>> {
    let listed = manifest
        .get("entries")
        .and_then(Json::as_array)
        .ok_or_else(|| "manifest.json missing entries array.".to_owned())?;
    let expected_digest = json_str(manifest, "manifest_entries_sha256", MANIFEST_PATH)?;
    if sha256(pyjson::canonical(manifest.at("entries")).as_bytes()) != expected_digest {
        return Err("manifest.json manifest_entries_sha256 does not match entries.".into());
    }
    let mut entry_hashes = BTreeMap::new();
    for item in listed {
        let path = json_str(item, "path", MANIFEST_PATH)?;
        let expected_sha = json_str(item, "sha256", MANIFEST_PATH)?;
        let expected_len = item
            .get("bytes")
            .and_then(Json::as_plain_int)
            .and_then(|v| usize::try_from(v).ok())
            .ok_or_else(|| format!("manifest.json invalid byte length for '{path}'."))?;
        let data = entries
            .get(&path)
            .ok_or_else(|| format!("Binder archive missing manifest entry '{path}'."))?;
        if data.len() != expected_len || sha256(data) != expected_sha {
            return Err(format!("Binder entry '{path}' failed digest verification."));
        }
        if entry_hashes.insert(path.clone(), expected_sha).is_some() {
            return Err(format!("manifest.json lists duplicate entry '{path}'."));
        }
    }
    if entry_hashes.len() != entries.len() {
        return Err("Binder archive contains entries not listed in manifest.json.".into());
    }
    Ok(entry_hashes)
}

fn verify_embedded_pack_and_audit(
    manifest: &Json,
    entries: &BTreeMap<String, Vec<u8>>,
) -> Result<EmbeddedCheck> {
    let pack_payloads: BTreeMap<String, Vec<u8>> = PACK_NAMES
        .into_iter()
        .map(|name| (name.to_owned(), entries.get(name).unwrap().clone()))
        .collect();
    let verified_pack = viewer::verify_payloads(&pack_payloads)?;
    let pack_doc = &verified_pack.document;
    let pack_status = json_str(manifest, "pack_status", MANIFEST_PATH)?;
    if json_str(pack_doc, "overall_status", PACK_NAMES[0])? != pack_status {
        return Err("manifest.json pack_status does not match close-review-pack.json.".into());
    }
    let source_hashes = read_source_hashes(manifest)?;
    if read_source_hashes(pack_doc)? != source_hashes {
        return Err("manifest.json source_sha256 does not match close-review-pack.json.".into());
    }
    let recorded_head = manifest
        .get("audit_chain_head_sha256")
        .map(|_| json_str(manifest, "audit_chain_head_sha256", MANIFEST_PATH))
        .transpose()?;
    let audit_chain_head = match (entries.get(AUDIT_CHAIN_PATH), recorded_head) {
        (Some(chain_bytes), Some(expected_head)) => {
            let summary = verify_audit_chain(chain_bytes)?;
            if summary.head_sha256 != expected_head {
                return Err(
                    "manifest.json audit_chain_head_sha256 does not match audit-chain.jsonl."
                        .into(),
                );
            }
            Some(summary.head_sha256)
        }
        (None, None) => None,
        _ => return Err("Binder audit-chain.jsonl presence does not match manifest.json.".into()),
    };
    Ok(EmbeddedCheck {
        pack_status,
        source_hashes,
        audit_chain_head,
    })
}

fn valid_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn parse_manifest(manifest_bytes: &[u8]) -> Result<Json> {
    let manifest_text = std::str::from_utf8(manifest_bytes)
        .map_err(|_| "manifest.json must be valid UTF-8.".to_owned())?;
    let manifest =
        pyjson::loads(manifest_text).map_err(|_| "manifest.json is not valid JSON.".to_owned())?;
    if !manifest
        .get("schema")
        .is_some_and(|s| s.is_str(MANIFEST_SCHEMA))
    {
        return Err("manifest.json must declare schema 'close-binder-manifest-v1'.".into());
    }
    Ok(manifest)
}

pub fn verify_binder(zip_path: &Path, anchor: &BinderAnchor) -> Result<BinderVerification> {
    let raw = fs::read(zip_path)
        .map_err(|e| format!("Cannot read binder archive {}: {e}.", zip_path.display()))?;
    verify_binder_bytes(&raw, anchor)
}

pub fn verify_binder_bytes(raw: &[u8], anchor: &BinderAnchor) -> Result<BinderVerification> {
    if !valid_sha256_hex(&anchor.manifest_sha256) {
        return Err(
            "Trusted binder anchor manifest_sha256 must be 64 lowercase hex characters.".into(),
        );
    }
    let mut entries = read_zip(raw)?;
    let manifest_bytes = entries
        .remove(MANIFEST_PATH)
        .ok_or_else(|| "Binder archive is missing manifest.json.".to_owned())?;
    let manifest_sha256 = sha256(&manifest_bytes);
    if manifest_sha256 != anchor.manifest_sha256 {
        return Err("Binder manifest.json SHA-256 does not match the trusted anchor.".into());
    }
    let manifest = parse_manifest(&manifest_bytes)?;
    let entry_hashes = verify_manifest_entries(&manifest, &entries)?;
    for name in PACK_NAMES {
        if !entry_hashes.contains_key(name) {
            return Err(format!(
                "Binder archive is missing required pack artefact '{name}'."
            ));
        }
    }
    let EmbeddedCheck {
        pack_status,
        source_hashes,
        audit_chain_head,
    } = verify_embedded_pack_and_audit(&manifest, &entries)?;
    if audit_chain_head != anchor.audit_chain_head {
        return Err("Binder audit_chain_head_sha256 does not match the trusted anchor.".into());
    }
    Ok(BinderVerification {
        pack_status,
        entry_count: entry_hashes.len(),
        source_hashes,
        entry_hashes,
        manifest_sha256,
        audit_chain_head,
    })
}

#[cfg(test)]
mod tests;
