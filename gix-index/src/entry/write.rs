use crate::{Entry, State, entry};

impl Entry {
    /// Serialize ourselves in V2/V3 form to `out` with path access via `state`, without padding.
    pub fn write_to(&self, out: impl std::io::Write, state: &State) -> std::io::Result<()> {
        self.write_with_previous_path(out, state, None)
    }

    pub(crate) fn write_with_previous_path(
        &self,
        mut out: impl std::io::Write,
        state: &State,
        previous: Option<&[u8]>,
    ) -> std::io::Result<()> {
        let stat = self.stat;
        out.write_all(&stat.ctime.secs.to_be_bytes())?;
        out.write_all(&stat.ctime.nsecs.to_be_bytes())?;
        out.write_all(&stat.mtime.secs.to_be_bytes())?;
        out.write_all(&stat.mtime.nsecs.to_be_bytes())?;
        out.write_all(&stat.dev.to_be_bytes())?;
        out.write_all(&stat.ino.to_be_bytes())?;
        out.write_all(&self.mode.bits().to_be_bytes())?;
        out.write_all(&stat.uid.to_be_bytes())?;
        out.write_all(&stat.gid.to_be_bytes())?;
        out.write_all(&stat.size.to_be_bytes())?;
        out.write_all(self.id.as_bytes())?;
        let path = self.path(state);
        let path_len: u16 = if path.len() >= entry::Flags::PATH_LEN.bits() as usize {
            entry::Flags::PATH_LEN.bits() as u16
        } else {
            path.len()
                .try_into()
                .expect("we just checked that the length is smaller than 0xfff")
        };
        out.write_all(&(self.flags.to_storage().bits() | path_len).to_be_bytes())?;
        if self.flags.contains(entry::Flags::EXTENDED) {
            out.write_all(
                &entry::at_rest::FlagsExtended::from_flags(self.flags)
                    .bits()
                    .to_be_bytes(),
            )?;
        }
        if let Some(previous) = previous {
            let common = previous.iter().zip(path.iter()).take_while(|(a, b)| a == b).count();
            let mut strip = previous.len() - common;
            let mut encoded = [0u8; 10];
            let mut start = encoded.len() - 1;
            encoded[start] = (strip & 0x7f) as u8;
            while strip > 0x7f {
                strip = (strip >> 7) - 1;
                start -= 1;
                encoded[start] = 0x80 | (strip & 0x7f) as u8;
            }
            out.write_all(&encoded[start..])?;
            out.write_all(&path[common..])?;
        } else {
            out.write_all(path)?;
        }
        out.write_all(b"\0")
    }
}
