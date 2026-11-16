//! Function constant values, as the Mac's runner gives them (`constants_of`
//! in `runner.m`): a value converted to the constant's type, or base64
//! bytes, at an index, by name, or over a range of indices.
use super::suite::named;
use serde_json::Value;
use stela_driver_gpu::metal::{
    abi::DataType,
    device::pipelines::{ConstantKey, ConstantValue},
    error::{Error, Result},
};

/// The case's constants, none for a case without.
pub fn values(constants: &Value) -> Result<Vec<ConstantValue>> {
    let mut out = Vec::new();
    for spec in constants.as_array().into_iter().flatten() {
        let data_type = named(DataType::ALL, spec["type"].as_str().unwrap_or_default())
            .ok_or(Error::Unsupported("a function constant type"))?;
        let bytes = match spec["bytes"].as_str() {
            Some(text) => base64(text).ok_or(Error::Invalid("constant bytes"))?,
            None => value_bytes(data_type, &spec["value"]),
        };
        if let Some(range) = spec["range"].as_array() {
            let start = range.first().and_then(Value::as_u64).unwrap_or(0) as u32;
            let count = range.get(1).and_then(Value::as_u64).unwrap_or(0) as usize;
            let size = bytes.len().checked_div(count).unwrap_or(0);
            if size == 0 {
                return Err(Error::Invalid("a constant range"));
            }
            for (at, chunk) in bytes.chunks(size).take(count).enumerate() {
                let key = ConstantKey::Index(start + at as u32);
                out.retain(|value: &ConstantValue| value.key != key);
                out.push(ConstantValue {
                    key,
                    data_type,
                    value: chunk.to_vec(),
                });
            }
            continue;
        }
        let key = match spec["name"].as_str() {
            Some(name) => ConstantKey::Name(name.to_owned()),
            None => ConstantKey::Index(spec["index"].as_u64().unwrap_or(0) as u32),
        };
        // A constant set again takes its last value, as
        // MTLFunctionConstantValues keeps.
        out.retain(|value: &ConstantValue| value.key != key);
        out.push(ConstantValue {
            key,
            data_type,
            value: bytes,
        });
    }
    Ok(out)
}

/// A value's bytes as the runner's union holds them.
fn value_bytes(data_type: DataType, value: &Value) -> Vec<u8> {
    let real = value.as_f64().unwrap_or(0.0);
    let integer = value.as_i64().unwrap_or(real as i64);
    match data_type {
        DataType::Bool | DataType::Char | DataType::UChar => vec![integer as u8],
        DataType::Short | DataType::UShort => (integer as u16).to_le_bytes().to_vec(),
        DataType::Half => half(real as f32).to_le_bytes().to_vec(),
        DataType::Float => (real as f32).to_le_bytes().to_vec(),
        DataType::Long | DataType::ULong => (integer as u64).to_le_bytes().to_vec(),
        _ => (integer as u32).to_le_bytes().to_vec(),
    }
}

/// The nearest half, ties to even.
fn half(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exponent = ((bits >> 23) & 0xff) as i32;
    let mantissa = bits & 0x7f_ffff;
    if exponent == 0xff {
        return sign | 0x7c00 | if mantissa != 0 { 0x200 } else { 0 };
    }
    let unbiased = exponent - 127 + 15;
    if unbiased >= 0x1f {
        return sign | 0x7c00;
    }
    if unbiased <= 0 {
        if unbiased < -10 {
            return sign;
        }
        let full = mantissa | 0x80_0000;
        let shift = (14 - unbiased) as u32;
        let rounded = (full >> shift)
            + u32::from(
                (full >> (shift - 1)) & 1 != 0
                    && ((full & ((1 << (shift - 1)) - 1)) != 0 || (full >> shift) & 1 != 0),
            );
        return sign | rounded as u16;
    }
    let rounded = (unbiased as u32) << 10 | mantissa >> 13;
    let remainder = mantissa & 0x1fff;
    let up = remainder > 0x1000 || (remainder == 0x1000 && rounded & 1 != 0);
    sign | (rounded + u32::from(up)) as u16
}

fn base64(text: &str) -> Option<Vec<u8>> {
    let value = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    };
    let digits: Vec<u8> = text
        .bytes()
        .filter(|c| *c != b'=' && !c.is_ascii_whitespace())
        .collect();
    let mut out = Vec::new();
    for chunk in digits.chunks(4) {
        let mut word = 0;
        for (at, &c) in chunk.iter().enumerate() {
            word |= value(c)? << (18 - 6 * at);
        }
        let bytes = word.to_be_bytes();
        out.extend_from_slice(&bytes[1..chunk.len()]);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn halves_and_base64() {
        assert_eq!(half(1.0), 0x3c00);
        assert_eq!(half(-2.0), 0xc000);
        assert_eq!(half(0.5), 0x3800);
        assert_eq!(half(65504.0), 0x7bff);
        assert_eq!(base64("AQID").unwrap(), [1, 2, 3]);
        assert_eq!(base64("AQI=").unwrap(), [1, 2]);
    }
}
