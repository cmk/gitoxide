use proptest::prelude::*;

use crate::{File, State, Version, decode, entry};

fn entry_count() -> impl Strategy<Value = usize> {
    prop_oneof![8 => prop::sample::select(vec![0, 1, 2, 127, 128, 255]), 2 => 0usize..512]
}

proptest! {
    #[test]
    fn decoded_entry_budget_is_independent_of_other_allocations(count in entry_count()) {
        for hash in [gix_hash::Kind::Sha1, gix_hash::Kind::Sha256] {
            for version in [Version::V2, Version::V3, Version::V4] {
                let mut state = State::new(hash);
                state.version = version;
                let flags = if version == Version::V3 { entry::Flags::EXTENDED | entry::Flags::SKIP_WORKTREE }
                    else { entry::Flags::empty() };
                for _ in 0..count {
                    state.dangerously_push_entry(Default::default(), hash.null(), flags,
                        entry::Mode::FILE, b"abc".as_slice().into());
                }
                let file = File::from_state(state, "unused-index");
                let mut bytes = Vec::new();
                let (written, _) = file.write_to(&mut bytes, Default::default())?;
                let required = count * std::mem::size_of::<crate::Entry>();
                let allowance = decode::max_entries_possible(bytes.len(), None, hash, written)
                    * std::mem::size_of::<crate::Entry>();
                prop_assert!(allowance >= required);
                let mut options = decode::Options {
                    alloc_limit_bytes: Some(bytes.len()),
                    entry_alloc_limit_bytes: Some(allowance),
                    thread_limit: Some(1),
                    ..Default::default()
                };
                let read = |options| State::from_bytes(&bytes, file.timestamp(), hash, options);
                prop_assert_eq!(read(options)?.0.entries().len(), count);
                if count > 0 {
                    options.entry_alloc_limit_bytes = Some(required - 1);
                    prop_assert!(matches!(read(options), Err(decode::Error::OutOfMemory)));
                    options.entry_alloc_limit_bytes = Some(allowance);
                    options.alloc_limit_bytes = Some(0);
                    prop_assert!(matches!(read(options), Err(decode::Error::OutOfMemory)));
                }
            }
        }
    }
}
