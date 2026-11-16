//! The argument buffers of the suite's cases, as the Mac's runner fills
//! them without an argument encoder (`encode_arguments` in `runner.m`):
//! the Tier 2 layout, each entry's GPU address or resource ID written at
//! its offset in a shared buffer.
use super::suite::{Held, Runner};
use serde_json::Value;
use stela_driver_gpu::metal::error::{Error, Result};

/// Whether this runner fills every argument buffer of `case`.
pub fn supported(case: &Value) -> bool {
    const ARGUMENTS: [&str; 2] = ["buffer", "entries"];
    const ENTRY: [&str; 4] = ["kind", "offset", "resource", "resource_offset"];
    let only = |value: &Value, keys: &[&str]| {
        value
            .as_object()
            .is_some_and(|object| object.keys().all(|key| keys.contains(&key.as_str())))
    };
    case["arguments"].as_array().into_iter().flatten().all(|a| {
        only(a, &ARGUMENTS)
            && a["entries"].as_array().into_iter().flatten().all(|e| {
                only(e, &ENTRY)
                    && matches!(e["kind"].as_str(), Some("buffer" | "texture" | "sampler"))
            })
    })
}

impl Runner<'_> {
    /// Writes the case's argument buffers.
    pub(crate) fn encode_arguments(&mut self, case: &Value, held: &Held) -> Result<()> {
        for a in case["arguments"].as_array().into_iter().flatten() {
            let target = *held
                .named
                .get(a["buffer"].as_str().unwrap_or_default())
                .ok_or(Error::Invalid("an argument buffer is no buffer"))?;
            for e in a["entries"].as_array().into_iter().flatten() {
                let name = e["resource"].as_str().unwrap_or_default();
                let word = match e["kind"].as_str() {
                    Some("buffer") => {
                        let at = *held
                            .named
                            .get(name)
                            .ok_or(Error::Invalid("an argument names no buffer"))?;
                        let reference = self.device.buffer(held.buffers[at].id)?;
                        reference.address + e["resource_offset"].as_u64().unwrap_or(0)
                    }
                    Some("texture") => self.device.texture_resource_id(held.texture(name)?)?.0,
                    Some("sampler") => held.sampler(name)?.0,
                    _ => return Err(Error::Unsupported("an argument kind")),
                };
                let offset = e["offset"].as_u64().unwrap_or(0);
                let buffer = &held.buffers[target];
                if offset + 8 > buffer.length {
                    return Err(Error::Invalid("an argument lies past its buffer"));
                }
                // SAFETY: within the buffer's mapping, checked above.
                unsafe {
                    buffer
                        .pointer
                        .as_ptr()
                        .add(offset as usize)
                        .cast::<u64>()
                        .write_unaligned(word);
                }
            }
        }
        Ok(())
    }
}
