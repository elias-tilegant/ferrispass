use std::io::{self, ErrorKind};

pub(crate) const MAX_KDBX_BYTES: u64 = 250_000_000;

const VERSION_HEADER_LEN: usize = 12;
const KDB_HEADER_LEN: usize = 124;

const KDB_ID: u32 = 0xb54b_fb65;
const KDBX2_ID: u32 = 0xb54b_fb66;
const KDBX_ID: u32 = 0xb54b_fb67;

const KDF_AES: [u8; 16] = [
    0xc9, 0xd9, 0xf3, 0x9a, 0x62, 0x8a, 0x44, 0x60, 0xbf, 0x74, 0x0d, 0x08, 0xc1, 0x8a, 0x4f, 0xea,
];
const KDF_AES_LEGACY_ALIAS: [u8; 16] = [
    0x7c, 0x02, 0xbb, 0x82, 0x79, 0xa7, 0x4a, 0xc0, 0x92, 0x7d, 0x11, 0x4a, 0x00, 0x64, 0x82, 0x38,
];
const KDF_ARGON2: [u8; 16] = [
    0xef, 0x63, 0x6d, 0xdf, 0x8c, 0x29, 0x44, 0x4b, 0x91, 0xf7, 0xa9, 0xa4, 0x03, 0xe3, 0x0a, 0x0c,
];
const KDF_ARGON2ID: [u8; 16] = [
    0x9e, 0x29, 0x8b, 0x19, 0x56, 0xdb, 0x47, 0x73, 0xb2, 0x3d, 0xfc, 0x3e, 0xc6, 0xf0, 0xa1, 0xe6,
];

// These are deliberately far above normal KeePass/KeePassXC settings while
// still preventing hostile headers from requesting unbounded work or memory.
// KeePassXC's UI allows Argon2 memory up to multiple GiB — vaults configured
// there must keep opening here, so the memory cap sits at 4 GiB and the
// combined memory×iterations budget stays high enough for any combination
// the per-parameter caps admit at realistic settings (e.g. 4 GiB × 16).
const MAX_AES_ROUNDS: u64 = 1_000_000_000;
const MAX_ARGON2_MEMORY: u64 = 4 * 1024 * 1024 * 1024;
const MAX_ARGON2_ITERATIONS: u64 = 100;
const MAX_ARGON2_PARALLELISM: u32 = 64;
const MAX_ARGON2_WORK: u64 = 64 * 1024 * 1024 * 1024;

pub(crate) fn validate_kdbx_size(size: u64) -> io::Result<()> {
    if size > MAX_KDBX_BYTES {
        return Err(invalid_data(format!(
            "KeePass database exceeds the {MAX_KDBX_BYTES}-byte size limit"
        )));
    }
    Ok(())
}

pub(crate) fn validate_kdbx_resources(data: &[u8]) -> io::Result<()> {
    validate_kdbx_size(u64::try_from(data.len()).unwrap_or(u64::MAX))?;

    let header = data
        .get(..VERSION_HEADER_LEN)
        .ok_or_else(|| invalid_data("truncated KeePass version header"))?;
    if header.get(..4) != Some(&[0x03, 0xd9, 0xa2, 0x9a]) {
        return Err(invalid_data("invalid KeePass database signature"));
    }

    match read_u32(header, 4, "database signature")? {
        KDB_ID => validate_kdb_header(data),
        KDBX2_ID => Ok(()),
        KDBX_ID => match read_u16(header, 10, "database major version")? {
            3 => validate_kdbx3_header(data),
            4 => validate_kdbx4_header(data),
            version => Err(invalid_data(format!(
                "unsupported KeePass database major version {version}"
            ))),
        },
        _ => Err(invalid_data("invalid KeePass database signature")),
    }
}

fn validate_kdb_header(data: &[u8]) -> io::Result<()> {
    if data.len() < KDB_HEADER_LEN {
        return Err(invalid_data("truncated KDB header"));
    }
    validate_aes_rounds(u64::from(read_u32(data, 120, "KDB AES rounds")?))
}

fn validate_kdbx3_header(data: &[u8]) -> io::Result<()> {
    let mut position = VERSION_HEADER_LEN;
    let mut rounds = None;

    loop {
        let entry_type = read_u8(data, position, "KDBX3 header entry type")?;
        let length_position = checked_add(position, 1, "KDBX3 header position")?;
        let length = usize::from(read_u16(
            data,
            length_position,
            "KDBX3 header entry length",
        )?);
        let value_start = checked_add(position, 3, "KDBX3 header position")?;
        let value_end = checked_add(value_start, length, "KDBX3 header entry length")?;
        let value = data
            .get(value_start..value_end)
            .ok_or_else(|| invalid_data("truncated KDBX3 header entry"))?;
        position = value_end;

        match entry_type {
            0 => break,
            3 | 10 => require_length(value, 4, "KDBX3 numeric header entry")?,
            6 => {
                require_length(value, 8, "KDBX3 AES rounds")?;
                rounds = Some(read_u64(value, 0, "KDBX3 AES rounds")?);
            }
            _ => {}
        }
    }

    if let Some(rounds) = rounds {
        validate_aes_rounds(rounds)?;
    }
    Ok(())
}

fn validate_kdbx4_header(data: &[u8]) -> io::Result<()> {
    let mut position = VERSION_HEADER_LEN;
    let mut kdf = None;

    loop {
        let entry_type = read_u8(data, position, "KDBX4 header entry type")?;
        let length_position = checked_add(position, 1, "KDBX4 header position")?;
        let length = usize::try_from(read_u32(
            data,
            length_position,
            "KDBX4 header entry length",
        )?)
        .map_err(|_| invalid_data("KDBX4 header entry length does not fit this platform"))?;
        let value_start = checked_add(position, 5, "KDBX4 header position")?;
        let value_end = checked_add(value_start, length, "KDBX4 header entry length")?;
        let value = data
            .get(value_start..value_end)
            .ok_or_else(|| invalid_data("truncated KDBX4 header entry"))?;
        position = value_end;

        match entry_type {
            0 => break,
            3 => require_length(value, 4, "KDBX4 compression header entry")?,
            11 => kdf = Some(parse_variant_dictionary(value)?),
            12 => {
                parse_variant_dictionary(value)?;
            }
            _ => {}
        }
    }

    if let Some(kdf) = kdf {
        validate_kdf(kdf)?;
    }
    Ok(())
}

#[derive(Default)]
struct KdfFields<'a> {
    uuid: Option<&'a [u8]>,
    memory: Option<u64>,
    iterations: Option<u64>,
    parallelism: Option<u32>,
    rounds: Option<u64>,
}

fn parse_variant_dictionary(buffer: &[u8]) -> io::Result<KdfFields<'_>> {
    if read_u16(buffer, 0, "variant dictionary version")? != 0x0100 {
        return Err(invalid_data("unsupported variant dictionary version"));
    }

    let mut position = 2;
    let mut fields = KdfFields::default();
    loop {
        let value_type = read_u8(buffer, position, "variant dictionary value type")?;
        position = checked_add(position, 1, "variant dictionary position")?;
        if value_type == 0 {
            if position != buffer.len() {
                return Err(invalid_data(
                    "trailing data after variant dictionary terminator",
                ));
            }
            return Ok(fields);
        }

        let key_length =
            usize::try_from(read_u32(buffer, position, "variant dictionary key length")?).map_err(
                |_| invalid_data("variant dictionary key length does not fit this platform"),
            )?;
        position = checked_add(position, 4, "variant dictionary position")?;
        let key_end = checked_add(position, key_length, "variant dictionary key length")?;
        let key = buffer
            .get(position..key_end)
            .ok_or_else(|| invalid_data("truncated variant dictionary key"))?;
        position = key_end;

        let value_length = usize::try_from(read_u32(
            buffer,
            position,
            "variant dictionary value length",
        )?)
        .map_err(|_| invalid_data("variant dictionary value length does not fit this platform"))?;
        position = checked_add(position, 4, "variant dictionary position")?;
        let value_end = checked_add(position, value_length, "variant dictionary value length")?;
        let value = buffer
            .get(position..value_end)
            .ok_or_else(|| invalid_data("truncated variant dictionary value"))?;
        position = value_end;

        match value_type {
            0x04 | 0x0c => require_length(value, 4, "variant dictionary 32-bit value")?,
            0x05 | 0x0d => require_length(value, 8, "variant dictionary 64-bit value")?,
            0x08 => require_length(value, 1, "variant dictionary boolean value")?,
            0x18 | 0x42 => {}
            _ => return Err(invalid_data("invalid variant dictionary value type")),
        }

        match (key, value_type) {
            (b"$UUID", 0x42) => fields.uuid = Some(value),
            (b"M", 0x05) => fields.memory = Some(read_u64(value, 0, "Argon2 memory")?),
            (b"I", 0x05) => fields.iterations = Some(read_u64(value, 0, "Argon2 iterations")?),
            (b"P", 0x04) => fields.parallelism = Some(read_u32(value, 0, "Argon2 parallelism")?),
            (b"R", 0x05) => fields.rounds = Some(read_u64(value, 0, "AES rounds")?),
            _ => {}
        }
    }
}

fn validate_kdf(fields: KdfFields<'_>) -> io::Result<()> {
    let Some(uuid) = fields.uuid else {
        return Ok(());
    };

    if uuid == KDF_AES || uuid == KDF_AES_LEGACY_ALIAS {
        if let Some(rounds) = fields.rounds {
            validate_aes_rounds(rounds)?;
        }
        return Ok(());
    }

    if uuid != KDF_ARGON2 && uuid != KDF_ARGON2ID {
        return Ok(());
    }

    if let Some(memory) = fields.memory
        && memory > MAX_ARGON2_MEMORY
    {
        return Err(invalid_data(format!(
            "Argon2 memory requirement exceeds the {MAX_ARGON2_MEMORY}-byte limit"
        )));
    }
    if let Some(iterations) = fields.iterations
        && iterations > MAX_ARGON2_ITERATIONS
    {
        return Err(invalid_data(format!(
            "Argon2 iteration count exceeds the {MAX_ARGON2_ITERATIONS}-iteration limit"
        )));
    }
    if let Some(parallelism) = fields.parallelism
        && parallelism > MAX_ARGON2_PARALLELISM
    {
        return Err(invalid_data(format!(
            "Argon2 parallelism exceeds the {MAX_ARGON2_PARALLELISM}-lane limit"
        )));
    }
    if let (Some(memory), Some(iterations)) = (fields.memory, fields.iterations)
        && memory
            .checked_mul(iterations)
            .is_none_or(|work| work > MAX_ARGON2_WORK)
    {
        return Err(invalid_data(
            "Argon2 memory/iteration cost exceeds the safety limit",
        ));
    }
    Ok(())
}

fn validate_aes_rounds(rounds: u64) -> io::Result<()> {
    if rounds > MAX_AES_ROUNDS {
        return Err(invalid_data(format!(
            "AES-KDF round count exceeds the {MAX_AES_ROUNDS}-round limit"
        )));
    }
    Ok(())
}

fn require_length(value: &[u8], expected: usize, name: &str) -> io::Result<()> {
    if value.len() != expected {
        return Err(invalid_data(format!(
            "invalid {name} length: expected {expected}, got {}",
            value.len()
        )));
    }
    Ok(())
}

fn checked_add(left: usize, right: usize, name: &str) -> io::Result<usize> {
    left.checked_add(right)
        .ok_or_else(|| invalid_data(format!("{name} overflow")))
}

fn read_u8(data: &[u8], offset: usize, name: &str) -> io::Result<u8> {
    data.get(offset)
        .copied()
        .ok_or_else(|| invalid_data(format!("truncated {name}")))
}

fn read_u16(data: &[u8], offset: usize, name: &str) -> io::Result<u16> {
    let end = checked_add(offset, 2, name)?;
    let value: [u8; 2] = data
        .get(offset..end)
        .and_then(|value| value.try_into().ok())
        .ok_or_else(|| invalid_data(format!("truncated {name}")))?;
    Ok(u16::from_le_bytes(value))
}

fn read_u32(data: &[u8], offset: usize, name: &str) -> io::Result<u32> {
    let end = checked_add(offset, 4, name)?;
    let value: [u8; 4] = data
        .get(offset..end)
        .and_then(|value| value.try_into().ok())
        .ok_or_else(|| invalid_data(format!("truncated {name}")))?;
    Ok(u32::from_le_bytes(value))
}

fn read_u64(data: &[u8], offset: usize, name: &str) -> io::Result<u64> {
    let end = checked_add(offset, 8, name)?;
    let value: [u8; 8] = data
        .get(offset..end)
        .and_then(|value| value.try_into().ok())
        .ok_or_else(|| invalid_data(format!("truncated {name}")))?;
    Ok(u64::from_le_bytes(value))
}

fn invalid_data(message: impl Into<String>) -> io::Error {
    io::Error::new(ErrorKind::InvalidData, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn database_size_limit_is_inclusive() {
        assert!(validate_kdbx_size(MAX_KDBX_BYTES).is_ok());
        assert_eq!(
            validate_kdbx_size(MAX_KDBX_BYTES + 1).unwrap_err().kind(),
            ErrorKind::InvalidData
        );
    }

    #[test]
    fn truncated_kdbx4_header_entry_is_rejected() {
        let mut data = version_header(4);
        data.push(11);
        data.extend_from_slice(&u32::MAX.to_le_bytes());

        assert_invalid_data(&data);
    }

    #[test]
    fn malformed_variant_dictionary_length_is_rejected() {
        let mut dictionary = 0x0100u16.to_le_bytes().to_vec();
        dictionary.push(0x42);
        dictionary.extend_from_slice(&u32::MAX.to_le_bytes());
        let data = kdbx4_with_kdf(dictionary);

        assert_invalid_data(&data);
    }

    #[test]
    fn common_argon2_settings_are_accepted() {
        let dictionary = kdf_dictionary(
            &KDF_ARGON2ID,
            &[
                (b"M", Value::U64(64 * 1024 * 1024)),
                (b"I", Value::U64(10)),
                (b"P", Value::U32(4)),
            ],
        );

        assert!(validate_kdbx_resources(&kdbx4_with_kdf(dictionary)).is_ok());
    }

    #[test]
    fn excessive_argon2_memory_is_rejected() {
        let dictionary = kdf_dictionary(
            &KDF_ARGON2,
            &[
                (b"M", Value::U64(MAX_ARGON2_MEMORY + 1)),
                (b"I", Value::U64(1)),
                (b"P", Value::U32(1)),
            ],
        );

        assert_invalid_data(&kdbx4_with_kdf(dictionary));
    }

    #[test]
    fn excessive_combined_argon2_cost_is_rejected() {
        let dictionary = kdf_dictionary(
            &KDF_ARGON2ID,
            &[
                // Both individually inside the per-parameter caps, but the
                // product busts the combined budget (4 GiB × 20 = 80 GiB).
                (b"M", Value::U64(4 * 1024 * 1024 * 1024)),
                (b"I", Value::U64(20)),
                (b"P", Value::U32(4)),
            ],
        );

        assert_invalid_data(&kdbx4_with_kdf(dictionary));
    }

    #[test]
    fn keepassxc_scale_argon2_settings_are_accepted() {
        // A KeePassXC vault configured with 1 GiB memory must open here —
        // rejecting it used to surface as a bogus "different master
        // password" on the sync-merge path.
        let dictionary = kdf_dictionary(
            &KDF_ARGON2ID,
            &[
                (b"M", Value::U64(1024 * 1024 * 1024)),
                (b"I", Value::U64(20)),
                (b"P", Value::U32(8)),
            ],
        );

        assert!(validate_kdbx_resources(&kdbx4_with_kdf(dictionary)).is_ok());
    }

    #[test]
    fn excessive_kdbx3_aes_rounds_are_rejected() {
        let mut data = version_header(3);
        push_kdbx3_field(&mut data, 6, &(MAX_AES_ROUNDS + 1).to_le_bytes());
        push_kdbx3_field(&mut data, 0, &[]);

        assert_invalid_data(&data);
    }

    #[test]
    fn missing_kdbx3_header_terminator_is_rejected() {
        let mut data = version_header(3);
        push_kdbx3_field(&mut data, 6, &1u64.to_le_bytes());

        assert_invalid_data(&data);
    }

    fn assert_invalid_data(data: &[u8]) {
        assert_eq!(
            validate_kdbx_resources(data).unwrap_err().kind(),
            ErrorKind::InvalidData
        );
    }

    fn version_header(major: u16) -> Vec<u8> {
        let mut data = vec![0x03, 0xd9, 0xa2, 0x9a];
        data.extend_from_slice(&KDBX_ID.to_le_bytes());
        data.extend_from_slice(&1u16.to_le_bytes());
        data.extend_from_slice(&major.to_le_bytes());
        data
    }

    fn kdbx4_with_kdf(dictionary: Vec<u8>) -> Vec<u8> {
        let mut data = version_header(4);
        data.push(11);
        data.extend_from_slice(&(dictionary.len() as u32).to_le_bytes());
        data.extend_from_slice(&dictionary);
        data.push(0);
        data.extend_from_slice(&0u32.to_le_bytes());
        data
    }

    fn push_kdbx3_field(data: &mut Vec<u8>, entry_type: u8, value: &[u8]) {
        data.push(entry_type);
        data.extend_from_slice(&(value.len() as u16).to_le_bytes());
        data.extend_from_slice(value);
    }

    enum Value {
        U32(u32),
        U64(u64),
    }

    fn kdf_dictionary(uuid: &[u8; 16], values: &[(&[u8], Value)]) -> Vec<u8> {
        let mut dictionary = 0x0100u16.to_le_bytes().to_vec();
        push_dictionary_value(&mut dictionary, 0x42, b"$UUID", uuid);
        for (key, value) in values {
            match value {
                Value::U32(value) => {
                    push_dictionary_value(&mut dictionary, 0x04, key, &value.to_le_bytes())
                }
                Value::U64(value) => {
                    push_dictionary_value(&mut dictionary, 0x05, key, &value.to_le_bytes())
                }
            }
        }
        dictionary.push(0);
        dictionary
    }

    fn push_dictionary_value(dictionary: &mut Vec<u8>, value_type: u8, key: &[u8], value: &[u8]) {
        dictionary.push(value_type);
        dictionary.extend_from_slice(&(key.len() as u32).to_le_bytes());
        dictionary.extend_from_slice(key);
        dictionary.extend_from_slice(&(value.len() as u32).to_le_bytes());
        dictionary.extend_from_slice(value);
    }
}
