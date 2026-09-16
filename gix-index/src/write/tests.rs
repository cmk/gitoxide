use proptest::prelude::*;

use crate::{File, State, Version, entry};

fn v4_paths() -> impl Strategy<Value = Vec<Vec<u8>>> {
    (
        prop_oneof![
            8 => prop::sample::select(vec![0, 1, 127, 128, 4095, 4096, 16511, 16512]),
            2 => 0usize..20000,
        ],
        prop::collection::vec(1u8..=255, 1..16),
    )
        .prop_map(|(len, suffix)| {
            let mut path = vec![b'a'; len];
            path.extend(suffix);
            vec![path.clone(), b"removed".to_vec(), path, b"z".to_vec()]
        })
}

proptest! {
    #[test]
    fn v4_roundtrip_preserves_paths_flags_and_compression(
        paths in v4_paths(),
        hash in prop_oneof![Just(gix_hash::Kind::Sha1), Just(gix_hash::Kind::Sha256)],
        all_removed in prop_oneof![1 => Just(true), 9 => Just(false)],
    ) {
        let mut state = State::new(hash);
        state.version = Version::V4;
        for (position, path) in paths.iter().enumerate() {
            let mut flags = match position {
                0 => entry::Flags::from_stage(entry::Stage::Base),
                1 => entry::Flags::REMOVE,
                2 => entry::Flags::EXTENDED | entry::Flags::SKIP_WORKTREE
                    | entry::Flags::from_stage(entry::Stage::Theirs),
                _ => entry::Flags::empty(),
            };
            flags.set(entry::Flags::REMOVE, all_removed || position == 1);
            state.dangerously_push_entry(Default::default(), hash.null(), flags, entry::Mode::FILE, path.as_slice().into());
        }
        let file = File::from_state(state, "unused-index");
        let mut bytes = Vec::new();
        let (version, _) = file.write_to(&mut bytes, Default::default())?;
        prop_assert_eq!(version, Version::V4);
        let (decoded, _) = State::from_bytes(&bytes, file.timestamp(), hash, Default::default())?;
        let expected = file.entries().iter().filter(|e| !e.flags.contains(entry::Flags::REMOVE))
            .map(|e| (e.path(&file).to_owned(), e.flags, e.mode, e.id, e.stat)).collect::<Vec<_>>();
        let actual = decoded.entries().iter()
            .map(|e| (e.path(&decoded).to_owned(), e.flags, e.mode, e.id, e.stat)).collect::<Vec<_>>();
        prop_assert_eq!(actual, expected);
    }
}
