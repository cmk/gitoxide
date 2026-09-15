use proptest::prelude::*;

use super::{Paths, ResolvePath, Stage};
use crate::{File, State, write};

fn stage(hash: gix_hash::Kind) -> impl Strategy<Value = Option<Stage>> {
    let mode = prop_oneof![
        5 => prop::sample::select(vec![0o100644, 0o100755, 0o120000, 0o160000]),
        1 => Just(u32::MAX),
        1 => 1u32..=u32::MAX,
    ];
    let id = prop_oneof![
        1 => Just(vec![0; hash.len_in_bytes()]),
        1 => Just(vec![255; hash.len_in_bytes()]),
        6 => prop::collection::vec(any::<u8>(), hash.len_in_bytes()),
    ];
    prop_oneof![
        2 => Just(None),
        5 => (mode, id).prop_map(|(mode, id)| Some(Stage {
            mode,
            id: gix_hash::ObjectId::from_bytes_or_panic(&id),
        })),
    ]
}

fn paths(hash: gix_hash::Kind) -> impl Strategy<Value = Paths> {
    let name = prop_oneof![
        1 => Just(b"nested/space tab\tline\n\xff".to_vec()),
        1 => Just(vec![b'x'; 4096]),
        6 => prop::collection::vec(1u8..=255, 1..65),
    ];
    let path = (name, prop::array::uniform3(stage(hash)))
        .prop_map(|(name, stages)| ResolvePath {
            name: name.into(),
            stages,
        })
        .boxed();
    prop_oneof![
        1 => Just(Vec::new()),
        3 => prop::collection::vec(path.clone(), 1),
        4 => prop::collection::vec(path, 2..9),
    ]
}

proptest! {
    #[test]
    fn resolve_undo_roundtrips_through_index(
        paths in prop_oneof![Just(gix_hash::Kind::Sha1), Just(gix_hash::Kind::Sha256)]
            .prop_flat_map(|hash| paths(hash).prop_map(move |paths| (hash, paths))),
    ) {
        let (hash, paths) = paths;
        for extensions in [write::Extensions::All, write::Extensions::None,
            write::Extensions::Given { tree_cache: true, end_of_index_entry: true }]
        {
            let mut state = State::new(hash);
            state.resolve_undo = Some(paths.clone());
            let file = File::from_state(state, "unused-index");
            let mut bytes = Vec::new();
            file.write_to(&mut bytes, write::Options { extensions, skip_hash: false })?;
            let (actual, _) = State::from_bytes(&bytes, filetime::FileTime::now(), hash, Default::default())?;
            let expected = matches!(extensions, write::Extensions::All).then_some(&paths);
            prop_assert_eq!(actual.resolve_undo(), expected);
        }
    }
}
