use std::{
    fs, io,
    path::{Path, PathBuf},
};

use gix_odb::alternate;

pub fn alternate(
    objects_at: impl Into<PathBuf>,
    objects_to: impl Into<PathBuf>,
) -> Result<(PathBuf, PathBuf), io::Error> {
    alternate_with(objects_at, objects_to, None)
}

fn alternate_with(
    objects_at: impl Into<PathBuf>,
    objects_to: impl Into<PathBuf>,
    content_before_to: Option<&str>,
) -> Result<(PathBuf, PathBuf), io::Error> {
    let objects_to = objects_to.into();
    alternate_with_content(
        objects_at,
        objects_to.clone(),
        objects_to.to_str().expect("valid UTF-8").as_bytes().to_owned(),
        content_before_to,
    )
}

fn alternate_with_content(
    objects_at: impl Into<PathBuf>,
    objects_to: impl Into<PathBuf>,
    to_content: Vec<u8>,
    content_before_to: Option<&str>,
) -> Result<(PathBuf, PathBuf), io::Error> {
    let at = objects_at.into();
    let to = objects_to.into();
    let at_info = at.join("info");
    fs::create_dir_all(&at_info)?;
    fs::create_dir_all(&to)?;
    let contents = if let Some(content) = content_before_to {
        let mut c = vec![b'\n'];
        c.extend(content.as_bytes());
        c.extend(to_content);
        c
    } else {
        to_content
    };
    fs::write(at_info.join("alternates"), contents)?;
    Ok((at, to))
}

#[test]
fn nested_relative_alternates_resolve_from_the_declaring_object_directory() -> crate::Result {
    use gix_object::{Find as _, Write as _};

    let tmp = gix_testtools::tempfile::TempDir::new()?;
    let root = tmp.path();
    let (a, b) = alternate(root.join("a/objects"), root.join("storage/b/objects"))?;
    let c = root.join("storage/c/objects");
    alternate_with_content(&b, &c, b"../../c/objects\n".to_vec(), None)?;
    assert!(
        !a.join("../../c/objects").exists(),
        "the path relative to A must not accidentally provide C's objects"
    );

    let payload = b"available only in the nested alternate\n";
    let id = gix_odb::at(&c)?.write_buf(gix_object::Kind::Blob, payload)?;
    let actual_paths = alternate::resolve(a.clone(), &std::env::current_dir()?)?
        .into_iter()
        .map(fs::canonicalize)
        .collect::<Result<Vec<_>, _>>()?;
    assert_eq!(actual_paths, vec![fs::canonicalize(b)?, fs::canonicalize(c)?]);

    let mut buffer = Vec::new();
    let store = gix_odb::at(a)?;
    let object = store.try_find(&id, &mut buffer)?.expect("C owns the requested blob");
    assert_eq!(object.kind, gix_object::Kind::Blob);
    assert_eq!(object.data, payload);
    Ok(())
}

#[test]
fn circular_alternates_are_detected_with_relative_paths() -> crate::Result {
    let tmp = gix_testtools::tempfile::TempDir::new()?;
    let tmp = tmp.path().join("sub-dir");
    std::fs::create_dir(&tmp)?;
    let (from, _) = alternate(tmp.join("a"), tmp.join("b"))?;
    alternate_with_content(
        tmp.join("b"),
        tmp.join("..").join("a"),
        Path::new("..")
            .join("a")
            .to_str()
            .expect("valid UTF-8")
            .as_bytes()
            .to_owned(),
        None,
    )?;

    match alternate::resolve(from, &std::env::current_dir()?) {
        Err(alternate::Error::Cycle(chain)) => {
            assert_eq!(
                chain
                    .into_iter()
                    .map(|p| p.file_name().expect("non-root").to_str().expect("utf8").to_owned())
                    .collect::<Vec<_>>(),
                vec!["a", "b"]
            );
        }
        res => unreachable!("should be a specific kind of error: {:?}", res),
    }
    Ok(())
}

#[test]
fn alternates_reachable_on_multiple_paths_are_not_a_cycle() -> crate::Result {
    let tmp = gix_testtools::tempfile::TempDir::new()?;
    let tmp = tmp.path();
    let (a, shared) = alternate(tmp.join("a"), tmp.join("shared"))?;
    let (b, _) = alternate(tmp.join("b"), tmp.join("shared"))?;
    let (from, _) = alternate_with_content(
        tmp.join("from"),
        &a,
        [a.to_str().expect("valid UTF-8"), b.to_str().expect("valid UTF-8")]
            .join("\n")
            .into_bytes(),
        None,
    )?;

    let mut alternates = alternate::resolve(from, &std::env::current_dir()?)?;
    alternates.sort();
    assert_eq!(
        alternates,
        vec![a, b, shared],
        "each object directory is listed once, even though 'shared' is reachable via both 'a' and 'b'"
    );
    Ok(())
}

#[test]
fn cycles_between_alternates_also_listed_by_the_root_are_detected() -> crate::Result {
    let tmp = gix_testtools::tempfile::TempDir::new()?;
    let tmp = tmp.path();
    let (a, b) = alternate(tmp.join("a"), tmp.join("b"))?;
    alternate(&b, &a)?;
    let (from, _) = alternate_with_content(
        tmp.join("from"),
        &a,
        [a.to_str().expect("valid UTF-8"), b.to_str().expect("valid UTF-8")]
            .join("\n")
            .into_bytes(),
        None,
    )?;

    match alternate::resolve(from, &std::env::current_dir()?) {
        Err(alternate::Error::Cycle(chain)) => assert_eq!(
            chain
                .into_iter()
                .map(|p| p.file_name().expect("non-root").to_str().expect("utf8").to_owned())
                .collect::<Vec<_>>(),
            vec!["a", "b"],
            "the traversed A -> B -> A edges form a cycle even when A and B were first seen as siblings"
        ),
        res => unreachable!("should be a cycle error: {res:?}"),
    }
    Ok(())
}

#[test]
fn single_link_with_comment_before_path_and_ansi_c_escape() -> crate::Result {
    let tmp = gix_testtools::tempfile::TempDir::new()?;
    let non_alternate = tmp.path().join("actual");

    let (from, to) = alternate_with(tmp.path().join("a"), non_alternate, Some("# comment\n"))?;
    let alternates = alternate::resolve(from, &std::env::current_dir()?)?;
    assert_eq!(alternates.len(), 1);
    assert_eq!(alternates[0], to);
    Ok(())
}

#[test]
fn no_alternate_in_first_objects_dir() -> crate::Result {
    let tmp = gix_testtools::tempfile::TempDir::new()?;
    assert!(alternate::resolve(tmp.path().to_owned(), &std::env::current_dir()?)?.is_empty());
    Ok(())
}
