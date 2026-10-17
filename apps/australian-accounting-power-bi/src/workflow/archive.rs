use super::{MAX_EXPORT_BYTES, MAX_EXPORT_FILES, MAX_EXPORT_MEMBER, Result};
use std::collections::BTreeMap;

pub type Members = BTreeMap<String, Vec<u8>>;

pub fn encode(members: &Members) -> Result<Vec<u8>> {
    if members.len() > MAX_EXPORT_FILES
        || members.values().any(|data| data.len() > MAX_EXPORT_MEMBER)
        || members.values().map(Vec::len).sum::<usize>() > MAX_EXPORT_BYTES
    {
        return Err("Export exceeds the fixed-case size limits.".into());
    }
    let (mut out, mut central) = (Vec::new(), Vec::new());
    for (name, data) in members {
        append(&mut out, &mut central, name, data)?;
    }
    let offset = out.len() as u32;
    out.extend_from_slice(&central);
    u32s(&mut out, &[0x0605_4b50]);
    u16s(&mut out, &[0, 0, members.len() as u16, members.len() as u16]);
    u32s(&mut out, &[central.len() as u32, offset]);
    u16s(&mut out, &[0]);
    if out.len() > MAX_EXPORT_BYTES {
        return Err("Export archive exceeds the size limit.".into());
    }
    Ok(out)
}

fn append(out: &mut Vec<u8>, central: &mut Vec<u8>, name: &str, data: &[u8]) -> Result<()> {
    let length: u16 = name.len().try_into().map_err(|_| "Export filename is too long.")?;
    if !name.is_ascii() {
        return Err("Export member is outside the stored regular-file profile.".into());
    }
    let offset = out.len() as u32;
    let crc = crc32(data);
    let size = data.len() as u32;
    u32s(out, &[0x0403_4b50]);
    u16s(out, &[20, 0, 0, 0, 33]);
    u32s(out, &[crc, size, size]);
    u16s(out, &[length, 0]);
    out.extend_from_slice(name.as_bytes());
    out.extend_from_slice(data);
    u32s(central, &[0x0201_4b50]);
    u16s(central, &[0x0314, 20, 0, 0, 0, 33]);
    u32s(central, &[crc, size, size]);
    u16s(central, &[length, 0, 0, 0, 0]);
    u32s(central, &[0o100600 << 16, offset]);
    central.extend_from_slice(name.as_bytes());
    Ok(())
}

pub fn decode(payload: &[u8]) -> Result<Members> {
    if payload.len() > MAX_EXPORT_BYTES {
        return Err("Export archive exceeds the size limit.".into());
    }
    let (mut members, mut offset, mut total) = (Members::new(), 0, 0usize);
    while payload.get(offset..offset + 4) == Some(&0x0403_4b50u32.to_le_bytes()) {
        let header = payload.get(offset..offset + 30).ok_or("Export ZIP is invalid.")?;
        let size = read32(header, 18) as usize;
        let name_length = read16(header, 26) as usize;
        let extra_length = read16(header, 28) as usize;
        if read16(header, 6) != 0
            || read16(header, 8) != 0
            || read32(header, 22) as usize != size
            || size > MAX_EXPORT_MEMBER
        {
            return Err("Export member is outside the stored regular-file profile.".into());
        }
        let start = offset + 30;
        let name = payload
            .get(start..start + name_length)
            .ok_or("Export ZIP is invalid.")?;
        let name = std::str::from_utf8(name).map_err(|_| "Export contains an ambiguous or unsafe path.")?;
        if !safe_name(name) || members.contains_key(name) {
            return Err("Export contains an ambiguous or unsafe path.".into());
        }
        let start = start + name_length + extra_length;
        let data = payload.get(start..start + size).ok_or("Export ZIP is invalid.")?;
        total += size;
        if members.len() == MAX_EXPORT_FILES || total > MAX_EXPORT_BYTES {
            return Err("Export exceeds inventory limits.".into());
        }
        members.insert(name.to_owned(), data.to_vec());
        offset = start + size;
    }
    if encode(&members)? != payload {
        return Err("Export ZIP is outside the canonical profile.".into());
    }
    Ok(members)
}

fn safe_name(name: &str) -> bool {
    name.split('/').all(|part| {
        !part.is_empty()
            && part != "."
            && part != ".."
            && part
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"_.-".contains(&byte))
    })
}

fn u16s(out: &mut Vec<u8>, values: &[u16]) {
    values
        .iter()
        .for_each(|value| out.extend_from_slice(&value.to_le_bytes()));
}

fn u32s(out: &mut Vec<u8>, values: &[u32]) {
    values
        .iter()
        .for_each(|value| out.extend_from_slice(&value.to_le_bytes()));
}

fn read16(data: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(data[at..at + 2].try_into().expect("bounded ZIP header"))
}

fn read32(data: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(data[at..at + 4].try_into().expect("bounded ZIP header"))
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_zip_matches_python_metadata_order_and_bytes() {
        let members = Members::from([("b".into(), vec![0]), ("a.txt".into(), b"one".to_vec())]);
        let payload = encode(&members).unwrap();
        assert_eq!(
            super::super::digest(&payload),
            "7e08d9e2e26b999c2f19c3be05993df8423e41cab47c0d41652cf20b9e70664a"
        );
        assert_eq!(decode(&payload).unwrap(), members);
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        for offset in [0, 4, 6, 8, 10, 12, 14, 26, payload.len() - 1] {
            let mut changed = payload.clone();
            changed[offset] ^= 1;
            assert!(decode(&changed).is_err(), "offset {offset}");
        }
        let mut trailing = payload.clone();
        trailing.push(0);
        assert!(decode(&trailing).is_err());
        assert!(decode(&payload[..payload.len() - 1]).is_err());
    }

    #[test]
    fn inventory_limits_and_unsafe_names_are_refused() {
        for name in ["../x", "a/../b", "/absolute", "a\\b", "a\0b", "a/"] {
            let payload = encode(&Members::from([(name.into(), vec![])])).unwrap();
            assert!(decode(&payload).is_err());
        }
        assert!(encode(&Members::from([("large".into(), vec![0; MAX_EXPORT_MEMBER + 1])])).is_err());
        let members = (0..65).map(|i| (format!("file{i}"), Vec::new())).collect();
        assert!(encode(&members).is_err());
    }
}
