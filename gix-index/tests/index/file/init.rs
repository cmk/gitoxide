mod truncation {
    use proptest::prelude::*;

    fn hash_kind() -> impl Strategy<Value = gix_hash::Kind> {
        prop_oneof![Just(gix_hash::Kind::Sha1), Just(gix_hash::Kind::Sha256)]
    }

    fn prefix_length(full: usize, hash: usize) -> impl Strategy<Value = usize> {
        prop_oneof![
            4 => Just(0), 4 => Just(12), 4 => Just(13),
            4 => Just(hash - 1), 4 => Just(hash),
            4 => Just(full - 1), 4 => Just(full),
            8 => 0..=full,
        ]
    }

    #[test]
    fn thirteen_byte_index_returns_error() -> gix_testtools::Result {
        let tmp = gix_testtools::tempfile::TempDir::new()?;
        let path = tmp.path().join("index");
        std::fs::write(&path, b"DIRC\0\0\0\x02\0\0\0\0\0")?;
        assert!(gix_index::File::at(&path, gix_hash::Kind::Sha1, false, Default::default()).is_err());
        Ok(())
    }

    proptest! {
        #[test]
        fn populated_index_prefixes_do_not_unwind_or_modify_competing_lock(
            skip_hash in any::<bool>(),
            v4 in any::<bool>(),
            cut in prop_oneof![4 => Just(1usize), 4 => Just(12), 4 => Just(20), 8 => 1usize..256],
        ) {
            let fixture = crate::Fixture::Generated(if v4 { "v4_more_files_IEOT" } else { "v2" });
            let kind = gix_testtools::object_hash();
            let bytes = std::fs::read(fixture.to_path())?;
            let tmp = gix_testtools::tempfile::TempDir::new()?;
            let path = tmp.path().join("index");
            let lock = tmp.path().join("index.lock");
            std::fs::write(&path, &bytes)?;
            prop_assert!(gix_index::File::at(&path, kind, skip_hash, Default::default()).is_ok());
            let prefix = bytes.len().saturating_sub(cut);
            std::fs::write(&path, &bytes[..prefix])?;
            std::fs::write(&lock, b"other owner")?;
            let result = gix_index::File::at(&path, kind, skip_hash, Default::default());
            // Without checksum validation a prefix can itself be a valid
            // index with fewer optional extensions. It must still not unwind.
            if !skip_hash || prefix < 12 + kind.len_in_bytes() {
                prop_assert!(result.is_err());
            }
            prop_assert_eq!(std::fs::read(&path)?, &bytes[..prefix]);
            prop_assert_eq!(std::fs::read(&lock)?, b"other owner");
        }

        #[test]
        fn valid_index_prefixes_return_errors_without_modifying_bytes(
            (kind, skip_hash, prefix) in (hash_kind(), any::<bool>()).prop_flat_map(|(kind, skip)| {
                prefix_length(12 + kind.len_in_bytes(), kind.len_in_bytes()).prop_map(move |prefix| (kind, skip, prefix))
            })
        ) {
            let tmp = gix_testtools::tempfile::TempDir::new()?;
            let path = tmp.path().join("index");
            let mut index = gix_index::File::from_state(gix_index::State::new(kind), path.clone());
            index.write(Default::default())?;
            let bytes = std::fs::read(&path)?;
            prop_assert_eq!(bytes.len(), 12 + kind.len_in_bytes());
            std::fs::write(&path, &bytes[..prefix])?;
            let result = gix_index::File::at(&path, kind, skip_hash, gix_index::decode::Options {
                thread_limit: Some(1), ..Default::default()
            });
            prop_assert_eq!(result.is_ok(), prefix == bytes.len());
            prop_assert_eq!(std::fs::read(&path)?, &bytes[..prefix]);
            prop_assert!(!tmp.path().join("index.lock").exists());
        }
    }
}

mod at_or_new {
    use crate::Fixture::Generated;

    #[test]
    fn opens_existing() {
        gix_index::File::at_or_default(
            Generated("v4_more_files_IEOT").to_path(),
            gix_testtools::object_hash(),
            false,
            Default::default(),
        )
        .expect("file exists and can be opened");
    }

    #[test]
    fn create_empty_in_memory_state_if_file_does_not_exist() {
        let index = gix_index::File::at_or_default(
            "__definitely no file that exists ever__",
            gix_testtools::object_hash(),
            false,
            Default::default(),
        )
        .expect("file is defaulting to a new one");
        assert!(!index.path().is_file(), "the file wasn't created yet");
        assert_eq!(
            index.object_hash(),
            gix_testtools::object_hash(),
            "object hash is respected"
        );
        assert_eq!(index.entries().len(), 0, "index is empty");
    }
}

mod from_state {
    use gix_index::Version::{V2, V3, V4};

    use crate::Fixture::*;

    #[test]
    fn writes_data_to_disk_and_is_a_valid_index() -> gix_testtools::Result {
        let fixtures = [
            (Loose("extended-flags"), V3),
            (Generated("v2"), V2),
            (Generated("v2_empty"), V2),
            (Generated("v2_more_files"), V2),
            (Generated("v2_all_file_kinds"), V2),
            (Generated("v4_more_files_IEOT"), V4),
        ];

        for (fixture, expected_version) in fixtures {
            // Loose fixtures are pre-created and only exist as SHA-1 variants.
            if gix_testtools::object_hash() != gix_hash::Kind::Sha1 && matches!(fixture, Loose(_)) {
                continue;
            }

            let tmp = gix_testtools::tempfile::TempDir::new()?;
            let new_index_path = tmp.path().join(fixture.to_name());
            assert!(!new_index_path.exists());

            let index = gix_index::File::at(
                fixture.to_path(),
                gix_testtools::object_hash(),
                false,
                Default::default(),
            )?;
            let mut index = gix_index::File::from_state(index.into(), new_index_path.clone());
            assert!(index.checksum().is_none());
            assert_eq!(index.path(), new_index_path);

            index.write(gix_index::write::Options::default())?;
            assert!(index.checksum().is_some(), "checksum is adjusted after writing");
            assert!(index.path().is_file());
            assert_eq!(index.version(), expected_version);

            index.verify_integrity()?;
        }
        Ok(())
    }
}
