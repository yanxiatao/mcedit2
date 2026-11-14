//! 旧 MCEdit2 的 NBT.load 会识别 Region 解压结果中的一层 GZip。
//! 显式识别编码标记并验证 CRC，失败直接报错，不尝试其他解码器。
use crate::{decompress_limited, invalid, WorldError};
use flate2::{read::GzDecoder, Compression, GzBuilder, GzHeader};
use std::io::Write;

#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) enum NbtEnvelope {
    #[default]
    Plain,
    Gzip(GzHeader),
}
impl NbtEnvelope {
    pub(crate) fn decode(bytes: Vec<u8>) -> Result<(Self, Vec<u8>), WorldError> {
        Self::decode_limited(bytes, crate::MAX_NBT)
    }
    pub(crate) fn decode_limited(
        bytes: Vec<u8>,
        limit: u64,
    ) -> Result<(Self, Vec<u8>), WorldError> {
        if bytes.len() as u64 > crate::MAX_NBT {
            return Err(invalid("NBT 编码超过 64 MiB"));
        }
        if !bytes.starts_with(&[0x1f, 0x8b]) {
            if bytes.len() as u64 > limit {
                return Err(invalid("NBT 数据超过当前解码预算"));
            }
            return Ok((Self::Plain, bytes));
        }
        let mut decoder = GzDecoder::new(bytes.as_slice());
        let nbt = decompress_limited(&mut decoder, limit)?;
        let header = decoder
            .header()
            .ok_or_else(|| invalid("内层 NBT GZip 缺少有效头部"))?
            .clone();
        if nbt.first() != Some(&10) {
            return Err(invalid("内层 GZip 必须恰好包含一层 Compound NBT"));
        }
        Ok((Self::Gzip(header), nbt))
    }
    pub(crate) fn encode(&self, nbt: &[u8]) -> Result<Vec<u8>, WorldError> {
        match self {
            Self::Plain => Ok(nbt.to_vec()),
            Self::Gzip(header) => {
                let mut builder = GzBuilder::new()
                    .mtime(header.mtime())
                    .operating_system(header.operating_system());
                if let Some(value) = header.extra() {
                    builder = builder.extra(value);
                }
                if let Some(value) = header.filename() {
                    builder = builder.filename(value);
                }
                if let Some(value) = header.comment() {
                    builder = builder.comment(value);
                }
                let mut encoder = builder.write(Vec::new(), Compression::default());
                encoder.write_all(nbt)?;
                let bytes = encoder.finish()?;
                if bytes.len() as u64 > crate::MAX_NBT {
                    return Err(invalid("内层 GZip 超过 NBT 编码预算"));
                }
                Ok(bytes)
            }
        }
    }
}

#[cfg(test)]
pub(crate) fn run_test_category(category: crate::test_categories::Category) {
    tests::run_test_category(category);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inner_gzip_retains_source_header_metadata_and_rejects_invalid_encoding() {
        let mut encoder = GzBuilder::new()
            .mtime(123)
            .operating_system(3)
            .filename("chunk.nbt")
            .comment("原始测试")
            .extra([1, 2, 3])
            .write(Vec::new(), Compression::default());
        let raw = [10, 0, 0, 0];
        encoder.write_all(&raw).unwrap();
        let (envelope, decoded) = NbtEnvelope::decode(encoder.finish().unwrap()).unwrap();
        assert_eq!(decoded, raw);
        let (roundtrip, decoded) = NbtEnvelope::decode(envelope.encode(&raw).unwrap()).unwrap();
        assert_eq!(roundtrip, envelope);
        assert_eq!(decoded, raw);
        assert!(NbtEnvelope::decode(vec![0x1f, 0x8b, 8]).is_err());
        let nested = envelope.encode(&envelope.encode(&raw).unwrap()).unwrap();
        assert!(NbtEnvelope::decode(nested).is_err());
    }

    #[cfg(test)]
    pub(crate) fn run_test_category(category: crate::test_categories::Category) {
        if category == crate::test_categories::Category::Formats {
            inner_gzip_retains_source_header_metadata_and_rejects_invalid_encoding();
        }
    }
}
