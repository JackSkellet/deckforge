//! Authenticated ELGATO v1 assets. The user supplies the original, hash-pinned
//! application locally; no vendor key or decrypted asset is embedded or saved.
use aes_gcm::{Aes256Gcm, KeyInit, Nonce, aead::Aead};
use anyhow::{Context, Result, bail, ensure};
use ed25519_dalek::{Signature, VerifyingKey};
use hkdf::Hkdf;
use minicbor::{Decoder, data::Type};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs::File, io::Read, os::unix::fs::OpenOptionsExt, path::Path};
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::Zeroizing;

const LIMIT: usize = 32 * 1024 * 1024;
const REFERENCE_LIMIT: u64 = 64 * 1024 * 1024;
const REFERENCE_HASH: &str = "2963df27b13f97e5736e3f931348d89d2f693109f3cb9dc3c9899300e35ea9e3";
const MULTIPLIER: u64 = 0x5851_f42d_4c95_7f2d;
const INCREMENT: u64 = 0x1405_7b7e_f767_814f;
const MAX_METADATA: usize = 1024 * 1024;
const MAX_BLOCKS: usize = 8192;

struct Keys {
    application: [u8; 32],
    private: Zeroizing<[u8; 32]>,
    signer: [u8; 32],
}

/// Checks the original application identity without executing it or exposing keys.
pub fn validate_reference(path: &Path) -> Result<()> {
    reference_bytes(path).map(|_| ())
}

/// Only the manifest bytes are authenticated here. The caller must separately
/// validate the manifest's Files table before accepting or launching a package.
pub fn read(bytes: &[u8], reference: &Path) -> Result<Value> {
    let plaintext = Zeroizing::new(decode(bytes, reference)?);
    serde_json::from_slice(&plaintext).context("Authenticated manifest is not valid JSON")
}

/// Returns authenticated plaintext in memory, never a plaintext file.
pub fn decode(bytes: &[u8], reference: &Path) -> Result<Vec<u8>> {
    let original = reference_bytes(reference)?;
    let keys = provider(&original)?;
    decode_with_keys(bytes, &keys)
}

fn reference_bytes(path: &Path) -> Result<Vec<u8>> {
    let path = path
        .canonicalize()
        .context("Resolve original application reference")?;
    let file = File::options()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .context("Open original Stream Deck application reference")?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file(),
        "Original application reference must be a regular file"
    );
    ensure!(
        metadata.len() <= REFERENCE_LIMIT,
        "Original application reference exceeds 64 MiB"
    );
    let mut bytes = Vec::new();
    file.take(REFERENCE_LIMIT + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= REFERENCE_LIMIT,
        "Original application reference exceeds 64 MiB"
    );
    ensure!(
        format!("{:x}", Sha256::digest(&bytes)) == REFERENCE_HASH,
        "Unsupported original application reference: expected Stream Deck 7.6.0.23012 (SHA256 {REFERENCE_HASH})"
    );
    Ok(bytes)
}

fn slice(bytes: &[u8], offset: usize, length: usize) -> Result<&[u8]> {
    let end = offset
        .checked_add(length)
        .context("Protected asset range overflow")?;
    bytes
        .get(offset..end)
        .context("Protected asset range exceeds input")
}
fn u64_at(bytes: &[u8], offset: usize) -> Result<u64> {
    Ok(u64::from_le_bytes(slice(bytes, offset, 8)?.try_into()?))
}
fn usize_at(bytes: &[u8], offset: usize) -> Result<usize> {
    usize::try_from(u64_at(bytes, offset)?).context("Protected asset length overflow")
}

// PE virtual addresses are usable only after the complete reference hash matches.
fn pe_bytes(bytes: &[u8], address: u64, length: usize) -> Result<&[u8]> {
    let pe = u32::from_le_bytes(slice(bytes, 60, 4)?.try_into()?) as usize;
    ensure!(
        slice(bytes, pe, 4)? == b"PE\0\0",
        "Invalid original PE reference"
    );
    let sections = u16::from_le_bytes(slice(bytes, pe + 6, 2)?.try_into()?) as usize;
    let optional = u16::from_le_bytes(slice(bytes, pe + 20, 2)?.try_into()?) as usize;
    let rva = address
        .checked_sub(0x140000000)
        .context("Invalid reference address")?;
    for index in 0..sections {
        let entry = slice(bytes, pe + 24 + optional + 40 * index, 40)?;
        let start = u32::from_le_bytes(entry[12..16].try_into()?) as u64;
        let size = u32::from_le_bytes(entry[16..20].try_into()?) as u64;
        let offset = u32::from_le_bytes(entry[20..24].try_into()?) as u64;
        if rva >= start && rva + length as u64 <= start + size {
            return slice(bytes, (offset + rva - start).try_into()?, length);
        }
    }
    bail!("Original application reference address is outside a section")
}

fn pcg(state: &mut u64) -> u32 {
    let rotate = *state >> 59;
    let value = ((*state >> 18) ^ *state) >> 27;
    let output = ((value >> rotate) | (value << ((0_u64.wrapping_sub(rotate)) & 31))) as u32;
    *state = state.wrapping_mul(MULTIPLIER).wrapping_add(INCREMENT);
    output
}

fn restore(bytes: &[u8], blocks: [u64; 2], seed_instruction: u64) -> Result<Zeroizing<[u8; 32]>> {
    // These are locations in the required local artifact, not vendor key bytes.
    ensure!(
        pe_bytes(bytes, seed_instruction, 2)? == [0x49, 0xb8],
        "Original key provider instruction mismatch"
    );
    let mut state = u64::from_le_bytes(pe_bytes(bytes, seed_instruction + 2, 8)?.try_into()?);
    let mut table = [0_u8; 256];
    for (i, item) in table.iter_mut().enumerate() {
        *item = i as u8;
    }
    for i in (1..256).rev() {
        let j = pcg(&mut state) as usize % i;
        table.swap(i, j);
    }
    let mut inverse = [0_u8; 256];
    for (i, item) in table.iter().enumerate() {
        inverse[*item as usize] = i as u8;
    }
    let mask = pe_bytes(bytes, 0x1416b1430, 32)?;
    let mut result = Zeroizing::new([0_u8; 32]);
    for (block, address) in blocks.iter().enumerate() {
        for (i, encoded) in pe_bytes(bytes, *address, 16)?.iter().enumerate() {
            result[block * 16 + i] = inverse[*encoded as usize] ^ mask[block * 16 + i];
        }
    }
    Ok(result)
}

fn provider(bytes: &[u8]) -> Result<Keys> {
    let application = *restore(bytes, [0x1416e0040, 0x1416dff60], 0x140690029)?;
    let private = restore(bytes, [0x1416e0120, 0x1416e0230], 0x140690479)?;
    let signer = *restore(bytes, [0x1416dfca0, 0x1416e0130], 0x1406908c9)?;
    ensure!(
        PublicKey::from(&StaticSecret::from(*private)).as_bytes() == &application,
        "Original application key pair does not match"
    );
    Ok(Keys {
        application,
        private,
        signer,
    })
}

fn advance(mut state: u64, mut count: u64) -> u64 {
    let (mut a, mut c, mut multiplier, mut increment) = (1_u64, 0_u64, MULTIPLIER, INCREMENT);
    while count != 0 {
        if count & 1 != 0 {
            a = a.wrapping_mul(multiplier);
            c = c.wrapping_mul(multiplier).wrapping_add(increment);
        }
        increment = increment.wrapping_mul(multiplier.wrapping_add(1));
        multiplier = multiplier.wrapping_mul(multiplier);
        count >>= 1;
    }
    state = a.wrapping_mul(state).wrapping_add(c);
    state
}

fn framing(bytes: &mut [u8], seed: u64, offset: u64) {
    // Native reader bypasses the transform when the header seed is zero.
    if seed == 0 {
        return;
    }
    let folded = seed ^ (seed >> 16);
    let skip = ((folded ^ (folded >> 32)) & 0xffff) + offset;
    let initial = seed
        .wrapping_mul(MULTIPLIER)
        .wrapping_add(0x1a08_ee11_84ba_6d32);
    let mut state = advance(initial, skip);
    for byte in bytes {
        *byte ^= pcg(&mut state) as u8;
    }
}

fn chunks(bytes: &[u8]) -> Result<BTreeMap<[u8; 4], Vec<u8>>> {
    ensure!(
        (32..=LIMIT).contains(&bytes.len()),
        "Protected asset exceeds 32 MiB or has a truncated header"
    );
    ensure!(
        &bytes[..6] == b"ELGATO" && bytes[6..8] == [1, 0],
        "Unsupported protected asset header/version"
    );
    ensure!(
        usize_at(bytes, 8)? == bytes.len(),
        "Protected asset declared size mismatch"
    );
    let base = usize_at(bytes, 16)?;
    ensure!(
        base >= 32
            && base <= bytes.len()
            && (base - 32).is_multiple_of(32)
            && (base - 32) / 32 <= 4096,
        "Invalid protected asset directory size"
    );
    let seed = u64_at(bytes, 24)?;
    let mut directory = bytes[32..base].to_vec();
    framing(&mut directory, seed, 32);
    let mut result = BTreeMap::new();
    let mut ranges = Vec::new();
    for entry in directory.as_chunks::<32>().0 {
        let tag: [u8; 4] = entry[..4].try_into()?;
        if tag == [0; 4] {
            continue;
        }
        ensure!(
            [*b"INFO", *b"META", *b"SIGN", *b"DATA"].contains(&tag),
            "Unsupported protected asset chunk"
        );
        let start = base
            .checked_add(usize_at(entry, 4)?)
            .context("Protected asset chunk overflow")?;
        let length = usize_at(entry, 12)?;
        if tag != *b"DATA" {
            ensure!(length <= MAX_METADATA, "Protected metadata exceeds 1 MiB");
        }
        let mut chunk = slice(bytes, start, length)?.to_vec();
        ensure!(length > 0, "Empty protected asset chunk");
        let end = start + length;
        ensure!(
            ranges.iter().all(|&(a, b)| end <= a || start >= b),
            "Overlapping protected asset chunks"
        );
        ranges.push((start, end));
        framing(&mut chunk, seed, start as u64);
        ensure!(
            result.insert(tag, chunk).is_none(),
            "Duplicate protected asset chunk"
        );
    }
    ensure!(
        result.len() == 4,
        "Protected asset is missing a required chunk"
    );
    Ok(result)
}

enum Field<'a> {
    Text(&'a str),
    Bytes(&'a [u8]),
    Number(u64),
    Array(Vec<&'a [u8]>),
}

// A shallow, definite-length schema avoids generic recursive CBOR allocation.
fn fields<'a>(bytes: &'a [u8], allowed: &[&str]) -> Result<BTreeMap<&'a str, Field<'a>>> {
    let mut decoder = Decoder::new(bytes);
    let count = decoder
        .map()?
        .context("Indefinite protected metadata maps are unsupported")?;
    ensure!(
        count as usize == allowed.len(),
        "Unexpected protected metadata field count"
    );
    let mut fields = BTreeMap::new();
    for _ in 0..count {
        let key = decoder.str()?;
        ensure!(allowed.contains(&key), "Unknown protected metadata field");
        let value = match decoder.datatype()? {
            Type::String => Field::Text(decoder.str()?),
            Type::Bytes => Field::Bytes(decoder.bytes()?),
            Type::U8 | Type::U16 | Type::U32 | Type::U64 => Field::Number(decoder.u64()?),
            Type::Array => {
                let count = decoder
                    .array()?
                    .context("Indefinite protected metadata arrays are unsupported")?;
                ensure!(
                    count <= MAX_BLOCKS as u64,
                    "Protected asset has too many blocks"
                );
                let mut items = Vec::with_capacity(count as usize);
                for _ in 0..count {
                    items.push(decoder.bytes()?);
                }
                Field::Array(items)
            }
            _ => bail!("Unexpected protected metadata value type"),
        };
        ensure!(
            fields.insert(key, value).is_none(),
            "Duplicate protected metadata field"
        );
    }
    ensure!(
        decoder.position() == bytes.len(),
        "Trailing protected metadata bytes"
    );
    Ok(fields)
}

fn byte_field<'a>(fields: &BTreeMap<&str, Field<'a>>, key: &str, size: usize) -> Result<&'a [u8]> {
    match fields.get(key) {
        Some(Field::Bytes(value)) if value.len() == size => Ok(value),
        _ => bail!("Invalid protected metadata {key} length/type"),
    }
}
fn number_field(fields: &BTreeMap<&str, Field<'_>>, key: &str) -> Result<usize> {
    match fields.get(key) {
        Some(Field::Number(value)) => {
            usize::try_from(*value).context("Protected metadata integer overflow")
        }
        _ => bail!("Invalid protected metadata {key} type"),
    }
}
fn array_field<'a, 'b>(
    fields: &'a BTreeMap<&str, Field<'b>>,
    key: &str,
    size: usize,
    count: usize,
) -> Result<&'a Vec<&'b [u8]>> {
    match fields.get(key) {
        Some(Field::Array(value))
            if value.len() == count && value.iter().all(|item| item.len() == size) =>
        {
            Ok(value)
        }
        _ => bail!("Invalid protected metadata {key} count/length/type"),
    }
}

fn decode_with_keys(bytes: &[u8], keys: &Keys) -> Result<Vec<u8>> {
    let chunks = chunks(bytes)?;
    let info = fields(&chunks[b"INFO"], &["cipher", "signing"])?;
    ensure!(
        matches!(
            info.get("cipher"),
            Some(Field::Text("x25519-hkdf-sha256-aes-256-gcm"))
        ) && matches!(info.get("signing"), Some(Field::Text("ed25519"))),
        "Unsupported protected asset cryptographic scheme"
    );
    let signature = fields(&chunks[b"SIGN"], &["pk_hash", "signature", "signed_data"])?;
    ensure!(
        byte_field(&signature, "signed_data", 4)? == b"META",
        "Unsupported protected asset signature coverage"
    );
    ensure!(
        byte_field(&signature, "pk_hash", 32)? == Sha256::digest(keys.signer).as_slice(),
        "Unknown protected asset signer"
    );
    let verifying = VerifyingKey::from_bytes(&keys.signer).context("Invalid trusted signer")?;
    let signature = Signature::from_slice(byte_field(&signature, "signature", 64)?)?;
    verifying
        .verify_strict(&chunks[b"META"], &signature)
        .context("Protected asset signature verification failed")?;

    let metadata = fields(
        &chunks[b"META"],
        &[
            "data_size",
            "block_size",
            "IVs",
            "tags",
            "salt",
            "eph_pk",
            "app_pk_hash",
        ],
    )?;
    ensure!(
        byte_field(&metadata, "app_pk_hash", 32)? == Sha256::digest(keys.application).as_slice(),
        "Wrong protected asset decryption key"
    );
    let size = number_field(&metadata, "data_size")?;
    let block_size = number_field(&metadata, "block_size")?;
    ensure!(
        size <= LIMIT && size == chunks[b"DATA"].len(),
        "Protected asset DATA length mismatch"
    );
    ensure!(
        (1..=LIMIT).contains(&block_size),
        "Invalid protected asset block size"
    );
    let count = size.div_ceil(block_size);
    ensure!(count <= MAX_BLOCKS, "Protected asset has too many blocks");
    let ivs = array_field(&metadata, "IVs", 12, count)?;
    let tags = array_field(&metadata, "tags", 16, count)?;
    let salt = byte_field(&metadata, "salt", 16)?;
    let ephemeral: [u8; 32] = byte_field(&metadata, "eph_pk", 32)?.try_into()?;
    let private = StaticSecret::from(*keys.private);
    let shared = private.diffie_hellman(&PublicKey::from(ephemeral));
    ensure!(
        shared.was_contributory(),
        "Invalid protected asset X25519 peer key"
    );
    let mut key = Zeroizing::new([0_u8; 32]);
    Hkdf::<Sha256>::new(Some(salt), shared.as_bytes())
        .expand(&[], &mut *key)
        .map_err(|_| anyhow::anyhow!("Protected asset key derivation failed"))?;
    let cipher = Aes256Gcm::new_from_slice(&*key)
        .map_err(|_| anyhow::anyhow!("Invalid protected asset cipher key"))?;
    let mut plaintext = Zeroizing::new(Vec::with_capacity(size));
    for (index, block) in chunks[b"DATA"].chunks(block_size).enumerate() {
        let mut ciphertext = block.to_vec();
        ciphertext.extend_from_slice(tags[index]);
        let clear = Zeroizing::new(
            cipher
                .decrypt(Nonce::from_slice(ivs[index]), ciphertext.as_ref())
                .map_err(|_| anyhow::anyhow!("Protected asset block authentication failed"))?,
        );
        plaintext.extend_from_slice(&clear);
    }
    ensure!(
        plaintext.len() == size,
        "Authenticated protected asset length mismatch"
    );
    Ok(std::mem::take(&mut *plaintext))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use minicbor::Encoder;

    const TEST_SEED: u64 = 0x0312_4523_7865;
    type Parts = Vec<([u8; 4], Vec<u8>)>;

    fn test_keys() -> Keys {
        let private = Zeroizing::new([41; 32]);
        Keys {
            application: *PublicKey::from(&StaticSecret::from(*private)).as_bytes(),
            private,
            signer: SigningKey::from_bytes(&[42; 32]).verifying_key().to_bytes(),
        }
    }

    fn signed_metadata(metadata: &[u8], signer: &SigningKey) -> Vec<u8> {
        let mut e = Encoder::new(Vec::new());
        e.map(3)
            .unwrap()
            .str("pk_hash")
            .unwrap()
            .bytes(&Sha256::digest(signer.verifying_key().as_bytes()))
            .unwrap()
            .str("signature")
            .unwrap()
            .bytes(&signer.sign(metadata).to_bytes())
            .unwrap()
            .str("signed_data")
            .unwrap()
            .bytes(b"META")
            .unwrap();
        e.into_writer()
    }

    fn envelope(parts: &[([u8; 4], Vec<u8>)], seed: u64) -> Vec<u8> {
        let base = 32 + (parts.len() + 1) * 32; // Include native padding record.
        let mut result = vec![0_u8; base];
        result[..6].copy_from_slice(b"ELGATO");
        result[6] = 1;
        result[16..24].copy_from_slice(&(base as u64).to_le_bytes());
        result[24..32].copy_from_slice(&seed.to_le_bytes());
        for (index, (tag, bytes)) in parts.iter().enumerate() {
            let at = 32 + index * 32;
            result[at..at + 4].copy_from_slice(tag);
            let relative = (result.len() - base) as u64;
            result[at + 4..at + 12].copy_from_slice(&relative.to_le_bytes());
            result[at + 12..at + 20].copy_from_slice(&(bytes.len() as u64).to_le_bytes());
            result.extend_from_slice(bytes);
        }
        let length = result.len() as u64;
        result[8..16].copy_from_slice(&length.to_le_bytes());
        framing(&mut result[32..], seed, 32);
        result
    }

    fn fixture() -> (Parts, Vec<u8>) {
        let keys = test_keys();
        let ephemeral = StaticSecret::from([43; 32]);
        let shared = ephemeral.diffie_hellman(&PublicKey::from(keys.application));
        let mut key = [0_u8; 32];
        let salt = [44; 16];
        Hkdf::<Sha256>::new(Some(&salt), shared.as_bytes())
            .expand(&[], &mut key)
            .unwrap();
        let cipher = Aes256Gcm::new_from_slice(&key).unwrap();
        let clear: Vec<u8> = (0..1025).map(|i| (i % 251) as u8).collect();
        let mut data = Vec::new();
        let mut ivs = Vec::new();
        let mut tags = Vec::new();
        for (index, block) in clear.chunks(64).enumerate() {
            let mut iv = [0; 12];
            iv[..8].copy_from_slice(&(index as u64).to_le_bytes());
            let encrypted = cipher.encrypt(Nonce::from_slice(&iv), block).unwrap();
            data.extend_from_slice(&encrypted[..block.len()]);
            tags.push(encrypted[block.len()..].to_vec());
            ivs.push(iv);
        }
        let mut e = Encoder::new(Vec::new());
        e.map(7)
            .unwrap()
            .str("data_size")
            .unwrap()
            .u64(clear.len() as u64)
            .unwrap()
            .str("block_size")
            .unwrap()
            .u64(64)
            .unwrap()
            .str("salt")
            .unwrap()
            .bytes(&salt)
            .unwrap()
            .str("eph_pk")
            .unwrap()
            .bytes(PublicKey::from(&ephemeral).as_bytes())
            .unwrap()
            .str("app_pk_hash")
            .unwrap()
            .bytes(&Sha256::digest(keys.application))
            .unwrap()
            .str("IVs")
            .unwrap()
            .array(ivs.len() as u64)
            .unwrap();
        for iv in ivs {
            e.bytes(&iv).unwrap();
        }
        e.str("tags").unwrap().array(tags.len() as u64).unwrap();
        for tag in tags {
            e.bytes(&tag).unwrap();
        }
        let metadata = e.into_writer();
        let signature = signed_metadata(&metadata, &SigningKey::from_bytes(&[42; 32]));
        let mut info = Encoder::new(Vec::new());
        info.map(2)
            .unwrap()
            .str("cipher")
            .unwrap()
            .str("x25519-hkdf-sha256-aes-256-gcm")
            .unwrap()
            .str("signing")
            .unwrap()
            .str("ed25519")
            .unwrap();
        (
            vec![
                (*b"INFO", info.into_writer()),
                (*b"META", metadata),
                (*b"SIGN", signature),
                (*b"DATA", data),
            ],
            clear,
        )
    }

    fn resign(parts: &mut [([u8; 4], Vec<u8>)]) {
        parts[2].1 = signed_metadata(&parts[1].1, &SigningKey::from_bytes(&[42; 32]));
    }

    #[test]
    fn independently_signed_multiblock_data_authenticates() {
        let (parts, clear) = fixture();
        for seed in [0, TEST_SEED, u64::MAX] {
            assert_eq!(
                decode_with_keys(&envelope(&parts, seed), &test_keys()).unwrap(),
                clear
            );
        }
    }

    #[test]
    fn altered_metadata_signature_and_ciphertext_are_rejected() {
        let (parts, _) = fixture();
        for index in [1, 2, 3] {
            let mut changed = parts.clone();
            let last = changed[index].1.len() - 1;
            changed[index].1[last] ^= 1;
            assert!(decode_with_keys(&envelope(&changed, TEST_SEED), &test_keys()).is_err());
        }
        let mut bad_tag = parts;
        let last = bad_tag[1].1.len() - 1;
        bad_tag[1].1[last] ^= 1;
        resign(&mut bad_tag); // A valid signature cannot bypass block authentication.
        assert!(
            decode_with_keys(&envelope(&bad_tag, TEST_SEED), &test_keys())
                .unwrap_err()
                .to_string()
                .contains("block authentication")
        );
    }

    #[test]
    fn changed_ed25519_signature_is_rejected() {
        let (mut parts, _) = fixture();
        let offset = parts[2]
            .1
            .windows(9)
            .position(|w| w == b"signature")
            .unwrap()
            + 9;
        assert_eq!(&parts[2].1[offset..offset + 2], &[0x58, 64]);
        parts[2].1[offset + 2] ^= 1;
        assert!(
            decode_with_keys(&envelope(&parts, TEST_SEED), &test_keys())
                .unwrap_err()
                .to_string()
                .contains("signature verification")
        );
    }

    #[test]
    fn signer_and_application_identity_are_enforced() {
        let (parts, _) = fixture();
        let bytes = envelope(&parts, TEST_SEED);
        let mut keys = test_keys();
        keys.signer = SigningKey::from_bytes(&[99; 32]).verifying_key().to_bytes();
        assert!(
            decode_with_keys(&bytes, &keys)
                .unwrap_err()
                .to_string()
                .contains("Unknown")
        );
        keys = test_keys();
        keys.application = *PublicKey::from(&StaticSecret::from([99; 32])).as_bytes();
        assert!(
            decode_with_keys(&bytes, &keys)
                .unwrap_err()
                .to_string()
                .contains("Wrong")
        );
    }

    #[test]
    fn malformed_lengths_duplicates_and_overlaps_are_rejected() {
        let (parts, _) = fixture();
        let bytes = envelope(&parts, 0);
        for length in [0, 6, 31, 32, bytes.len() - 1] {
            assert!(decode_with_keys(&bytes[..length], &test_keys()).is_err());
        }
        for (offset, value) in [
            (8, u64::MAX),
            (16, u64::MAX),
            (36, u64::MAX),
            (44, u64::MAX),
        ] {
            let mut changed = bytes.clone();
            changed[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
            assert!(chunks(&changed).is_err());
        }
        let mut changed = bytes.clone();
        changed[64..68].copy_from_slice(b"INFO");
        assert!(
            chunks(&changed)
                .unwrap_err()
                .to_string()
                .contains("Duplicate")
        );
        let mut changed = bytes;
        changed[68..76].fill(0);
        assert!(
            chunks(&changed)
                .unwrap_err()
                .to_string()
                .contains("Overlapping")
        );
    }

    #[test]
    fn signed_invalid_block_size_and_unbounded_cbor_are_rejected() {
        let (mut parts, _) = fixture();
        let offset = parts[1]
            .1
            .windows(10)
            .position(|w| w == b"block_size")
            .unwrap()
            + 10;
        assert_eq!(parts[1].1[offset], 0x18);
        parts[1].1[offset + 1] = 0;
        resign(&mut parts);
        assert!(
            decode_with_keys(&envelope(&parts, TEST_SEED), &test_keys())
                .unwrap_err()
                .to_string()
                .contains("block size")
        );
        assert!(fields(&[0xbf, 0xff], &["x"]).is_err());
        assert!(
            fields(
                &[
                    0xa1, 0x61, b'x', 0x9b, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff
                ],
                &["x"]
            )
            .is_err()
        );
        assert!(fields(&[0xa1, 0x61, b'x', 0x81, 0x81], &["x"]).is_err());
    }

    #[test]
    fn wrong_local_reference_is_rejected_before_provider_access() {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), b"not an original vendor executable").unwrap();
        assert!(
            validate_reference(file.path())
                .unwrap_err()
                .to_string()
                .contains("Unsupported original")
        );
    }

    #[test]
    fn fifo_reference_is_rejected_without_waiting_for_writer() {
        use std::{ffi::CString, os::unix::ffi::OsStrExt};
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("reference.pipe");
        let name = CString::new(path.as_os_str().as_bytes()).unwrap();
        // The temporary test-owned FIFO has no writer. A blocking open would hang.
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        assert!(
            validate_reference(&path)
                .unwrap_err()
                .to_string()
                .contains("regular file")
        );
    }

    #[test]
    #[ignore = "requires explicitly supplied local original app and Discord package; no vendor fixtures in repository"]
    fn original_discord_authenticated_manifest() {
        let reference = std::env::var_os("DECKFORGE_ORIGINAL_APP").expect("DECKFORGE_ORIGINAL_APP");
        let package =
            std::env::var_os("DECKFORGE_DISCORD_PACKAGE").expect("DECKFORGE_DISCORD_PACKAGE");
        let file = File::open(package).unwrap();
        let mut zip = zip::ZipArchive::new(file).unwrap();
        let mut bytes = Vec::new();
        zip.by_name("com.elgato.discord.sdPlugin/manifest.json")
            .unwrap()
            .take(LIMIT as u64 + 1)
            .read_to_end(&mut bytes)
            .unwrap();
        let plaintext = Zeroizing::new(decode(&bytes, Path::new(&reference)).unwrap());
        assert_eq!(
            format!("{:x}", Sha256::digest(&plaintext)),
            "038c015493d11a665e7c561b993afc6f6933ad9db0ec926e1d0c6e50c33bae25"
        );
        let manifest: Value = serde_json::from_slice(&plaintext).unwrap();
        assert_eq!(manifest["UUID"], "com.elgato.discord");
        assert_eq!(manifest["Name"], "Discord");
        assert_eq!(manifest["Actions"].as_array().unwrap().len(), 17);
    }
}
