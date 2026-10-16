use crate::{Result, pypath};
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

fn enclosing_repository(
    directory: &Path,
    configured_work_tree: Option<&std::ffi::OsStr>,
    identity: &mut impl FnMut(&Path) -> io::Result<(u64, u64)>,
) -> std::io::Result<Option<(PathBuf, String)>> {
    for candidate in directory.ancestors() {
        for marker in [".git", ".hg", ".svn", ".bzr"] {
            match fs::symlink_metadata(candidate.join(marker)) {
                Ok(_) => {
                    return Ok(Some((
                        candidate.to_owned(),
                        format!("which holds {marker}"),
                    )));
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                    ) => {}
                Err(e) => return Err(e),
            }
        }
    }
    // This is the single non-secret Git setting the original output guard reads.
    if let Some(value) = configured_work_tree.filter(|s| !s.is_empty()) {
        let work_tree = pypath::resolve(Path::new(&value), false)
            .map_err(|_| std::io::Error::other("cannot resolve GIT_WORK_TREE"))?;
        for candidate in directory.ancestors() {
            if pypath::same_path(candidate, &work_tree) {
                return Ok(Some((
                    work_tree,
                    "which the GIT_WORK_TREE environment variable names".into(),
                )));
            }
            match (identity(candidate), identity(&work_tree)) {
                (Ok(a), Ok(b)) if a == b => {
                    return Ok(Some((
                        work_tree,
                        "which the GIT_WORK_TREE environment variable names".into(),
                    )));
                }
                (Err(e), _) | (_, Err(e))
                    if !matches!(
                        e.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                    ) =>
                {
                    return Err(e);
                }
                _ => {}
            }
        }
    }
    Ok(None)
}
pub fn require_output_outside_repository(output: &Path) -> Result<PathBuf> {
    let inspect = || -> std::io::Result<(PathBuf, Option<(PathBuf, String)>)> {
        let resolved = pypath::resolve(output, false)
            .map_err(|_| std::io::Error::other("cannot resolve output path"))?;
        for candidate in resolved.ancestors() {
            match fs::metadata(candidate) {
                Ok(_) => break,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e),
            }
        }
        let work_tree = std::env::var_os("GIT_WORK_TREE");
        let checkout = enclosing_repository(&resolved, work_tree.as_deref(), &mut pypath::file_id)?;
        Ok((resolved, checkout))
    };
    let (resolved, checkout) = inspect().map_err(|e| format!("{} cannot be examined for an enclosing version-control checkout: {e}. An output this run cannot inspect is refused rather than written to, because a review pack names a client's accounts and balances. Point --output somewhere readable.", pypath::text(output)))?;
    if let Some((repo, reason)) = checkout {
        return Err(format!(
            "{} is inside the version-control checkout at {}, {reason}. A review pack names a client's accounts and balances, so it belongs in an access-controlled directory outside version control. Point --output somewhere outside that checkout.",
            pypath::text(output),
            pypath::text(&repo)
        ));
    }
    Ok(resolved)
}

fn partial(destination: &Path) -> io::Result<PathBuf> {
    loop {
        let mut random = [0u8; 6];
        getrandom::fill(&mut random).map_err(io::Error::other)?;
        let suffix: String = random.iter().map(|b| format!("{b:02x}")).collect();
        let mut name = destination.file_name().unwrap().to_os_string();
        name.push(format!(".{suffix}.partial"));
        let candidate = destination.with_file_name(name);
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(_) => return Ok(candidate),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
}

fn remove_quietly(path: &Path) {
    let _ = fs::remove_file(path);
}

fn publish_with(
    output: &Path,
    rendered: &[Vec<u8>; 4],
    write: &mut impl FnMut(&Path, &[u8]) -> io::Result<()>,
    replace: &mut impl FnMut(&Path, &Path) -> io::Result<()>,
) -> io::Result<[PathBuf; 4]> {
    let destinations = crate::report::PACK_FILE_NAMES.map(|name| output.join(name));
    let mut staged = Vec::new();
    let stage = (|| -> io::Result<()> {
        for (destination, bytes) in destinations.iter().zip(rendered) {
            let path = partial(destination)?;
            staged.push(path.clone());
            write(&path, bytes)?;
        }
        Ok(())
    })();
    if let Err(error) = stage {
        for path in &staged {
            remove_quietly(path);
        }
        return Err(error);
    }
    let mut replaced: Vec<(PathBuf, Option<PathBuf>)> = Vec::new();
    let swaps = (|| -> io::Result<()> {
        for (staged, destination) in staged.iter().zip(&destinations) {
            let parked = if destination.is_file() {
                let parked = partial(destination)?;
                if let Err(e) = replace(destination, &parked) {
                    remove_quietly(&parked);
                    return Err(e);
                }
                Some(parked)
            } else {
                None
            };
            if let Err(error) = replace(staged, destination) {
                if let Some(parked) = &parked {
                    let _ = replace(parked, destination);
                }
                return Err(error);
            }
            replaced.push((destination.clone(), parked));
        }
        Ok(())
    })();
    if let Err(error) = swaps {
        for (destination, parked) in replaced.iter().rev() {
            if let Some(parked) = parked {
                let _ = replace(parked, destination);
            } else {
                remove_quietly(destination);
            }
        }
        for path in &staged {
            remove_quietly(path);
        }
        return Err(error);
    }
    for (_, parked) in replaced {
        if let Some(path) = parked {
            remove_quietly(&path);
        }
    }
    Ok(destinations)
}

pub fn write_review_pack(
    pack: &crate::engine::CloseReviewPack,
    output: &Path,
) -> Result<[PathBuf; 4]> {
    write_review_pack_with(output, || crate::report::rendered(pack))
}
fn write_review_pack_with(
    output: &Path,
    render: impl FnOnce() -> Result<[Vec<u8>; 4]>,
) -> Result<[PathBuf; 4]> {
    let output = require_output_outside_repository(output)?;
    fs::create_dir_all(&output).map_err(|e| e.to_string())?;
    let rendered = render()?;
    publish_with(
        &output,
        &rendered,
        &mut |path, bytes| fs::File::create(path)?.write_all(bytes),
        &mut |a, b| fs::rename(a, b),
    )
    .map_err(|e| e.to_string())
}

fn write_new_with(
    output: &Path,
    render: impl FnOnce() -> Result<Vec<(String, Vec<u8>)>>,
    write: &mut impl FnMut(&mut fs::File, &[u8]) -> io::Result<()>,
) -> Result<PathBuf> {
    let output = require_output_outside_repository(output)?;
    if output.exists() {
        return Err(
            "Output already exists. Choose a new directory to preserve the previous run.".into(),
        );
    }
    let files = render()?;
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::create_dir(&output).map_err(|e| e.to_string())?;
    let mut created = Vec::new();
    let result = (|| -> io::Result<()> {
        for (name, bytes) in files {
            let path = output.join(name);
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)?;
            created.push(path);
            write(&mut file, &bytes)?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        for path in created.iter().rev() {
            match fs::remove_file(path) {
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.to_string()),
                Ok(()) => {}
            }
        }
        fs::remove_dir(&output).map_err(|e| e.to_string())?;
        return Err(error.to_string());
    }
    Ok(output)
}
pub fn write_new_files(
    output: &Path,
    render: impl FnOnce() -> Result<Vec<(String, Vec<u8>)>>,
) -> Result<PathBuf> {
    write_new_with(output, render, &mut |file, bytes| file.write_all(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;
    fn data(label: &str) -> [Vec<u8>; 4] {
        std::array::from_fn(|i| format!("{label}-{i}").into_bytes())
    }
    fn inventory(dir: &Path) -> Vec<String> {
        let mut names: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }
    fn write(path: &Path, bytes: &[u8]) -> io::Result<()> {
        fs::write(path, bytes)
    }
    fn replace(a: &Path, b: &Path) -> io::Result<()> {
        fs::rename(a, b)
    }
    #[test]
    fn configured_work_tree_checks_identity_and_refuses_incomplete_inspection() {
        let temp = TempDir::new();
        let tree = temp.join("client-files");
        fs::create_dir(&tree).unwrap();
        let output = tree.join("packs/july");
        let found = enclosing_repository(&output, Some(tree.as_os_str()), &mut pypath::file_id)
            .unwrap()
            .unwrap();
        assert!(found.1.contains("GIT_WORK_TREE"));
        assert!(
            enclosing_repository(
                &temp.join("outside/july"),
                Some(tree.as_os_str()),
                &mut pypath::file_id
            )
            .unwrap()
            .is_none()
        );
        let error = enclosing_repository(
            &temp.join("outside/july"),
            Some(tree.as_os_str()),
            &mut |_| {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "injected identity refusal",
                ))
            },
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        let alias = temp.join("CLIENT-FILES");
        if cfg!(windows) {
            assert!(
                enclosing_repository(
                    &alias.join("packs/july"),
                    Some(tree.as_os_str()),
                    &mut pypath::file_id
                )
                .unwrap()
                .is_some()
            );
        }
        let aliased = enclosing_repository(
            &temp.join("alias/packs/july"),
            Some(tree.as_os_str()),
            &mut |_| Ok((1, 2)),
        )
        .unwrap()
        .unwrap();
        assert!(aliased.1.contains("GIT_WORK_TREE"));
    }
    #[test]
    fn redirecting_the_supplied_symlink_after_validation_keeps_writes_at_the_approved_directory() {
        let temp = TempDir::new();
        let safe = temp.join("safe");
        let checkout = temp.join("checkout");
        let link = temp.join("link");
        fs::create_dir(&safe).unwrap();
        fs::create_dir(&checkout).unwrap();
        fs::write(checkout.join(".git"), b"marker").unwrap();
        let approved = pypath::resolve(&safe, true).unwrap();
        #[cfg(unix)]
        let symlink = std::os::unix::fs::symlink;
        #[cfg(windows)]
        let symlink = std::os::windows::fs::symlink_dir;
        symlink(&safe, &link).unwrap();
        let paths = write_review_pack_with(&link, || {
            #[cfg(unix)]
            fs::remove_file(&link).unwrap();
            #[cfg(windows)]
            fs::remove_dir(&link).unwrap();
            symlink(&checkout, &link).unwrap();
            Ok(data("new"))
        })
        .unwrap();
        for (index, path) in paths.iter().enumerate() {
            assert_eq!(path.parent(), Some(approved.as_path()));
            assert_eq!(fs::read(path).unwrap(), data("new")[index]);
        }
        assert_eq!(inventory(&checkout), vec![".git"]);
    }
    #[test]
    fn a_looping_output_link_is_refused_before_source_reads() {
        let temp = TempDir::new();
        let link = temp.join("loop");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&link, &link).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(&link, &link).unwrap();
        let output = link.join("pack");
        assert!(
            require_output_outside_repository(&output)
                .unwrap_err()
                .contains("cannot be examined")
        );
        let (code, _, error) = crate::stdio::capture(|| {
            crate::cli::main(vec![
                "review".into(),
                "--current=missing-current".into(),
                "--prior=missing-prior".into(),
                "--output".into(),
                output.into_os_string(),
            ])
        });
        assert_eq!(code, 1);
        assert!(error.contains("cannot be examined"));
        assert!(!error.contains("does not exist"));
    }
    #[cfg(unix)]
    #[test]
    fn unreadable_marker_parent_refuses_incomplete_repository_inspection() {
        use std::os::unix::fs::PermissionsExt;
        let temp = TempDir::new();
        let directory = temp.join("private");
        fs::create_dir(&directory).unwrap();
        fs::write(directory.join(".git"), b"marker").unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o0)).unwrap();
        let result = enclosing_repository(&directory, None, &mut pypath::file_id);
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
    }
    #[test]
    fn replaces_all_members_and_preserves_unowned_files() {
        let temp = TempDir::new();
        fs::write(temp.0.join("unowned.partial"), b"unowned").unwrap();
        let paths = publish_with(&temp.0, &data("old"), &mut write, &mut replace).unwrap();
        publish_with(&temp.0, &data("new"), &mut write, &mut replace).unwrap();
        for (i, path) in paths.iter().enumerate() {
            assert_eq!(fs::read(path).unwrap(), data("new")[i]);
        }
        assert_eq!(inventory(&temp.0).len(), 5);
        assert_eq!(
            fs::read(temp.0.join("unowned.partial")).unwrap(),
            b"unowned"
        );
    }
    #[test]
    fn staging_failures_clean_partial_content_without_changing_previous_pack() {
        for failed in 0..4 {
            let temp = TempDir::new();
            let paths = publish_with(&temp.0, &data("old"), &mut write, &mut replace).unwrap();
            let mut count = 0;
            let error = publish_with(
                &temp.0,
                &data("new"),
                &mut |path, bytes| {
                    let current = count;
                    count += 1;
                    if current == failed {
                        fs::write(path, b"truncated")?;
                        return Err(io::Error::other("injected write failure"));
                    }
                    write(path, bytes)
                },
                &mut replace,
            )
            .unwrap_err();
            assert!(error.to_string().contains("injected"));
            for (i, path) in paths.iter().enumerate() {
                assert_eq!(fs::read(path).unwrap(), data("old")[i]);
            }
            assert_eq!(inventory(&temp.0).len(), 4);
        }
    }
    #[test]
    fn each_failed_park_or_swap_restores_previous_members_in_reverse_order() {
        for failed in 0..8 {
            let temp = TempDir::new();
            let paths = publish_with(&temp.0, &data("old"), &mut write, &mut replace).unwrap();
            let mut count = 0;
            let mut restored = Vec::new();
            publish_with(&temp.0, &data("new"), &mut write, &mut |a, b| {
                let current = count;
                count += 1;
                if current == failed {
                    return Err(io::Error::other("injected replacement failure"));
                }
                if current > failed && b.extension().is_some_and(|s| s != "partial") {
                    restored.push(b.to_owned());
                }
                replace(a, b)
            })
            .unwrap_err();
            for (i, path) in paths.iter().enumerate() {
                assert_eq!(fs::read(path).unwrap(), data("old")[i]);
            }
            assert_eq!(inventory(&temp.0).len(), 4);
            let mut indices: Vec<_> = restored
                .iter()
                .map(|path| paths.iter().position(|p| p == path).unwrap())
                .collect();
            let observed = indices.clone();
            indices.sort_by(|a, b| b.cmp(a));
            assert_eq!(observed, indices);
        }
    }
    #[test]
    fn failed_new_pack_removes_only_members_it_installed() {
        let temp = TempDir::new();
        let mut count = 0;
        publish_with(&temp.0, &data("new"), &mut write, &mut |a, b| {
            count += 1;
            if count == 3 {
                Err(io::Error::other("injected"))
            } else {
                replace(a, b)
            }
        })
        .unwrap_err();
        assert!(inventory(&temp.0).is_empty());
    }
    #[test]
    fn destination_directory_is_preserved_and_prior_pack_rolled_back() {
        let temp = TempDir::new();
        let paths = publish_with(&temp.0, &data("old"), &mut write, &mut replace).unwrap();
        fs::remove_file(&paths[2]).unwrap();
        fs::create_dir(&paths[2]).unwrap();
        fs::write(paths[2].join("owned-by-someone-else"), b"keep").unwrap();
        assert!(publish_with(&temp.0, &data("new"), &mut write, &mut replace).is_err());
        assert_eq!(fs::read(&paths[0]).unwrap(), data("old")[0]);
        assert_eq!(fs::read(&paths[1]).unwrap(), data("old")[1]);
        assert_eq!(fs::read(&paths[3]).unwrap(), data("old")[3]);
        assert_eq!(
            fs::read(paths[2].join("owned-by-someone-else")).unwrap(),
            b"keep"
        );
        assert_eq!(inventory(&temp.0).len(), 4);
    }
    #[test]
    fn checkout_pointer_markers_are_refused_without_creating_output() {
        for marker in [".git", ".hg", ".svn", ".bzr"] {
            let temp = TempDir::new();
            fs::write(temp.0.join(marker), b"pointer").unwrap();
            let output = temp.0.join("nested/output");
            assert!(
                require_output_outside_repository(&output)
                    .unwrap_err()
                    .contains(&format!("which holds {marker}"))
            );
            assert!(!output.exists());
        }
    }
    #[test]
    fn new_directory_exports_encode_before_creation_and_refuse_existing_runs() {
        let temp = TempDir::new();
        let output = temp.0.join("not-created/pack");
        assert!(write_new_files(&output, || Err("injected encoding failure".into())).is_err());
        assert!(!temp.0.join("not-created").exists());
        let output = temp.0.join("pack");
        fs::create_dir(&output).unwrap();
        fs::write(output.join("keep"), b"previous").unwrap();
        let mut rendered = false;
        assert!(
            write_new_files(&output, || {
                rendered = true;
                Ok(Vec::new())
            })
            .unwrap_err()
            .starts_with("Output already exists.")
        );
        assert!(!rendered);
        assert_eq!(fs::read(output.join("keep")).unwrap(), b"previous");
    }
    #[test]
    fn new_directory_write_failure_removes_only_created_members() {
        let temp = TempDir::new();
        let output = temp.0.join("pack");
        let mut writes = 0;
        let result = write_new_with(
            &output,
            || {
                Ok(vec![
                    ("first".into(), b"first".to_vec()),
                    ("second".into(), b"second".to_vec()),
                ])
            },
            &mut |file, bytes| {
                writes += 1;
                file.write_all(bytes)?;
                if writes == 2 {
                    Err(io::Error::other("injected write failure"))
                } else {
                    Ok(())
                }
            },
        );
        assert!(result.unwrap_err().contains("injected write failure"));
        assert!(!output.exists());
        let output = temp.0.join("pack-with-unowned-file");
        let result = write_new_with(
            &output,
            || {
                Ok(vec![
                    ("first".into(), b"first".to_vec()),
                    ("second".into(), b"second".to_vec()),
                ])
            },
            &mut |file, bytes| {
                file.write_all(bytes)?;
                fs::write(output.join("second"), b"unowned")?;
                Ok(())
            },
        );
        assert!(result.is_err());
        assert!(!output.join("first").exists());
        assert_eq!(fs::read(output.join("second")).unwrap(), b"unowned");
    }
    #[cfg(unix)]
    #[test]
    fn dangling_checkout_markers_and_symlinked_outputs_are_refused() {
        let temp = TempDir::new();
        fs::create_dir(temp.0.join("checkout")).unwrap();
        std::os::unix::fs::symlink("missing", temp.0.join("checkout/.git")).unwrap();
        std::os::unix::fs::symlink(temp.0.join("checkout"), temp.0.join("link")).unwrap();
        assert!(
            require_output_outside_repository(&temp.0.join("link/output"))
                .unwrap_err()
                .contains("which holds .git")
        );
    }
}
