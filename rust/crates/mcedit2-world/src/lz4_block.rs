//! Minecraft Java compression method 4: lz4-java LZ4Block streams.
//! NOT the regular LZ4 frame encoding. Read-only and strict: bounded output,
//! per-block xxHash32 verification, exact EOF marker, no silent trailing data.
use crate::WorldError;
const MAGIC: &[u8; 8] = b"LZ4Block";
const HEADER: usize = 21;
const MAX_NBT: usize = 64 * 1024 * 1024;
const XXH_SEED: u32 = 0x9747_b28c;

fn refuse(reason: &str) -> WorldError {
    WorldError::Invalid(reason.into())
}
fn number(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes.try_into().unwrap())
}
pub(crate) fn decode(stream: &[u8]) -> Result<Vec<u8>, WorldError> {
    let mut cursor = 0usize;
    let mut output = Vec::new();
    loop {
        let end = cursor
            .checked_add(HEADER)
            .ok_or_else(|| refuse("LZ4Block header position overflow"))?;
        let head = stream
            .get(cursor..end)
            .ok_or_else(|| refuse("Truncated original Java LZ4Block header/terminator"))?;
        if &head[..8] != MAGIC {
            return Err(refuse("Original LZ4 stream lacks LZ4Block magic"));
        }
        let token = head[8];
        let method = token & 0xf0;
        if !matches!(method, 0x10 | 0x20) {
            return Err(refuse("Unknown Java LZ4Block compression method"));
        }
        let level = (token & 0x0f) as usize + 10;
        let max_chunk = 1usize
            .checked_shl(level as u32)
            .ok_or_else(|| refuse("Unsupported LZ4Block size level"))?;
        let compressed = number(&head[9..13]) as usize;
        let decoded = number(&head[13..17]) as usize;
        let checksum = number(&head[17..21]);
        cursor = end;
        if compressed == 0 && decoded == 0 {
            if checksum != 0 || cursor != stream.len() {
                return Err(refuse(
                    "Invalid LZ4Block terminal checksum or trailing bytes",
                ));
            }
            return Ok(output);
        }
        if compressed == 0
            || decoded == 0
            || decoded > max_chunk
            || decoded > MAX_NBT.saturating_sub(output.len())
        {
            return Err(refuse("Java LZ4Block expanded size exceeds safe limits"));
        }
        let next = cursor
            .checked_add(compressed)
            .ok_or_else(|| refuse("LZ4Block compressed length overflow"))?;
        let bytes = stream
            .get(cursor..next)
            .ok_or_else(|| refuse("Truncated LZ4Block compressed source"))?;
        cursor = next;
        let raw = match method {
            0x10 => {
                if compressed != decoded {
                    return Err(refuse("Raw LZ4Block compressed/decompressed sizes differ"));
                }
                bytes.to_vec()
            }
            0x20 => {
                // Use the bounded caller-supplied buffer API: the alloc
                // decoder can panic when a corrupt stream expands beyond
                // its claimed size. Here that becomes a recoverable error.
                let mut target = vec![0u8; decoded];
                let written = lz4_flex::block::decompress_into(bytes, &mut target)
                    .map_err(|_| refuse("Malformed compressed Java LZ4 block"))?;
                target.truncate(written);
                target
            }
            _ => unreachable!(),
        };
        if raw.len() != decoded {
            return Err(refuse(
                "LZ4Block expanded length differs from declared length",
            ));
        }
        // The lz4-java block stream masks off the top 4 checksum bits.
        if xxhash_rust::xxh32::xxh32(&raw, XXH_SEED) & 0x0fff_ffff != checksum {
            return Err(refuse("Original Java LZ4Block xxHash32 mismatch"));
        }
        output.extend_from_slice(&raw);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn block(raw: &[u8], compressed: bool) -> Vec<u8> {
        let data = if compressed {
            lz4_flex::block::compress(raw)
        } else {
            raw.to_vec()
        };
        let mut out = MAGIC.to_vec();
        out.push(if compressed { 0x26 } else { 0x16 });
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(raw.len() as u32).to_le_bytes());
        out.extend_from_slice(
            &(xxhash_rust::xxh32::xxh32(raw, XXH_SEED) & 0x0fff_ffff).to_le_bytes(),
        );
        out.extend(data);
        out
    }
    fn terminal() -> Vec<u8> {
        let mut end = MAGIC.to_vec();
        end.push(0x16);
        end.extend_from_slice(&[0u8; 12]);
        end
    }
    #[test]
    fn java_block_stream_supports_raw_and_lz4_blocks_and_exact_end() {
        let data = vec![0x45; 4096];
        let mut stream = block(&data, false);
        stream.extend(block(&data, true));
        stream.extend(terminal());
        assert_eq!(decode(&stream).unwrap().len(), 8192);
        let mut trailing = stream.clone();
        trailing.push(0);
        assert!(decode(&trailing).is_err());
        stream.truncate(stream.len() - HEADER);
        assert!(decode(&stream).is_err());
    }
    #[test]
    fn malformed_checksum_and_lengths_cannot_be_silently_loaded() {
        let mut bytes = block(&vec![0x55; 128], false);
        bytes.extend(terminal());
        bytes[17] ^= 1;
        assert!(decode(&bytes).is_err());
        let mut lengths = block(&vec![0x55; 128], false);
        lengths[13..17].copy_from_slice(&((MAX_NBT as u32) + 1).to_le_bytes());
        lengths.extend(terminal());
        assert!(decode(&lengths).is_err());
        let mut unknown = block(&[1, 2, 3], false);
        unknown[8] = 0xf6;
        unknown.extend(terminal());
        assert!(decode(&unknown).is_err());
    }
}
