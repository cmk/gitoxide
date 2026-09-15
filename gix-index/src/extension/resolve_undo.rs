use bstr::BString;
use gix_hash::ObjectId;

use crate::{extension::Signature, util::split_at_byte_exclusive};

pub type Paths = Vec<ResolvePath>;

#[derive(Clone)]
#[cfg_attr(test, derive(Debug, PartialEq, Eq))]
pub struct ResolvePath {
    /// relative to the root of the repository, or what would be stored in the index
    name: BString,

    /// 0 = ancestor/common, 1 = ours, 2 = theirs
    stages: [Option<Stage>; 3],
}

#[derive(Clone, Copy)]
#[cfg_attr(test, derive(Debug, PartialEq, Eq))]
pub struct Stage {
    mode: u32,
    id: ObjectId,
}

pub const SIGNATURE: Signature = *b"REUC";

pub fn decode(mut data: &[u8], object_hash: gix_hash::Kind) -> Option<Paths> {
    let hash_len = object_hash.len_in_bytes();
    let mut out = Vec::new();

    while !data.is_empty() {
        let (path, rest) = split_at_byte_exclusive(data, 0)?;
        data = rest;

        let mut modes = [0u32; 3];
        for mode in &mut modes {
            let (mode_ascii, rest) = split_at_byte_exclusive(data, 0)?;
            data = rest;
            *mode = u32::from_str_radix(std::str::from_utf8(mode_ascii).ok()?, 8).ok()?;
        }

        let mut stages = [None, None, None];
        for (mode, stage) in modes.iter().zip(stages.iter_mut()) {
            if *mode == 0 {
                continue;
            }
            let (hash, rest) = data.split_at_checked(hash_len)?;
            data = rest;
            *stage = Some(Stage {
                mode: *mode,
                id: ObjectId::from_bytes_or_panic(hash),
            });
        }

        out.push(ResolvePath {
            name: path.into(),
            stages,
        });
    }
    out.into()
}

pub(crate) fn write_to(paths: &[ResolvePath], mut out: impl std::io::Write) -> std::io::Result<()> {
    use std::io::Write;

    let mut entries = Vec::new();
    for path in paths {
        entries.write_all(&path.name)?;
        entries.write_all(b"\0")?;
        for stage in &path.stages {
            write!(entries, "{:o}\0", stage.map_or(0, |stage| stage.mode))?;
        }
        for stage in path.stages.iter().flatten() {
            entries.write_all(stage.id.as_bytes())?;
        }
    }
    let size = u32::try_from(entries.len())
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "resolve-undo extension exceeds 4GB"))?;
    out.write_all(&SIGNATURE)?;
    out.write_all(&size.to_be_bytes())?;
    out.write_all(&entries)
}

#[cfg(test)]
mod tests;
