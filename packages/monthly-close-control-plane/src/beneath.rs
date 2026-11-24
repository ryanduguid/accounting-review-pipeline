// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! Reading files beneath one directory opened once. Every name is opened
//! relative to the handle of the directory before it, so nothing outside the
//! directory can be reached by any spelling, and a link or other reparse
//! point at any step is refused rather than followed.
use cap_std::{ambient_authority, fs::Dir};
use std::{
    cell::RefCell,
    io::{Error, ErrorKind, Read},
    path::{Component, Path, PathBuf},
    rc::Rc,
};

/// The directory the tools may read, as given and as an open handle.
#[derive(Clone, Debug)]
pub struct Root {
    path: PathBuf,
    dir: Rc<Dir>,
}

thread_local! {
    static ACTIVE: RefCell<Option<Root>> = const { RefCell::new(None) };
}

fn refused(message: &str) -> Error {
    Error::new(ErrorKind::PermissionDenied, message)
}

impl Root {
    /// Open `path` once; later reads never look it up by name again.
    pub fn open(path: &Path) -> crate::Result<Self> {
        let failed = |error: Error| format!("directory {}: {error}", path.display());
        let absolute = std::path::absolute(path).map_err(failed)?;
        let dir = Dir::open_ambient_dir(&absolute, ambient_authority()).map_err(failed)?;
        Ok(Self {
            path: absolute,
            dir: Rc::new(dir),
        })
    }

    /// The path beneath the root that `raw` names: plain names separated
    /// by `/`, with no empty name, `.`, `..`, `\`, `:` or NUL, so it can
    /// be neither absolute nor a drive, share, device or stream.
    pub fn join(&self, raw: &str) -> crate::Result<PathBuf> {
        let mut path = self.path.clone();
        for name in raw.split('/') {
            if name.is_empty() || name == "." || name == ".." || name.contains(['\\', ':', '\0']) {
                return Err(format!(
                    "{raw}: give a path relative to the server's directory, \
                     with names separated by / and no ., .., \\ or :."
                ));
            }
            path.push(name);
        }
        Ok(path)
    }

    /// The names of `path` below the root, or an error when it is not
    /// below it.
    fn names<'a>(&self, path: &'a Path) -> std::io::Result<Vec<&'a std::ffi::OsStr>> {
        let outside = || refused("outside the directory this server may read");
        path.strip_prefix(&self.path)
            .map_err(|_| outside())?
            .components()
            .map(|component| match component {
                Component::Normal(name) => Ok(name),
                _ => Err(outside()),
            })
            .collect()
    }

    /// The directory holding the last name of `path`, opened a name at a
    /// time without following any link, and that last name.
    fn parent<'a>(&self, path: &'a Path) -> std::io::Result<(Dir, &'a std::ffi::OsStr)> {
        let names = self.names(path)?;
        let (last, parents) = names.split_last().ok_or_else(|| refused("names no file"))?;
        let mut dir = self.dir.try_clone()?;
        for name in parents {
            dir = open_dir(&dir, name)?;
        }
        Ok((dir, last))
    }

    /// Whether `path` is a directory beneath the root, reached without a link.
    pub fn directory(&self, path: &Path) -> std::io::Result<()> {
        let (dir, name) = self.parent(path)?;
        open_dir(&dir, name).map(drop)
    }

    /// Whether `path` is a regular file beneath the root, reached without a
    /// link: not a directory, pipe or device.
    pub fn file(&self, path: &Path) -> std::io::Result<()> {
        let (dir, name) = self.parent(path)?;
        let metadata = dir.symlink_metadata(name)?;
        if metadata.is_symlink() {
            Err(refused("is a link; links are not followed"))
        } else if metadata.is_file() {
            Ok(())
        } else {
            Err(refused("is not a regular file"))
        }
    }

    /// The bytes of the regular file `path`, reached without a link.
    pub fn read(&self, path: &Path) -> std::io::Result<Vec<u8>> {
        let (dir, name) = self.parent(path)?;
        not_a_link(&dir, name)?;
        if dir.symlink_metadata(name)?.is_dir() {
            return Err(Error::new(ErrorKind::IsADirectory, "is a directory"));
        }
        let mut options = cap_std::fs::OpenOptions::new();
        options.read(true);
        // A pipe opened for reading would wait for a writer; this returns at
        // once and the check below refuses it.
        #[cfg(unix)]
        cap_std::fs::OpenOptionsExt::custom_flags(&mut options, libc::O_NONBLOCK);
        let mut file = dir.open_with(name, &options)?;
        if !file.metadata()?.is_file() {
            return Err(refused("is not a regular file"));
        }
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        Ok(bytes)
    }

    /// Run `body` with every [`read`] confined to this root.
    pub fn within<T>(&self, body: impl FnOnce() -> T) -> T {
        let previous = ACTIVE.with(|active| active.replace(Some(self.clone())));
        let value = body();
        ACTIVE.with(|active| active.replace(previous));
        value
    }
}

fn not_a_link(dir: &Dir, name: &std::ffi::OsStr) -> std::io::Result<()> {
    if dir.symlink_metadata(name)?.is_symlink() {
        Err(refused("is a link; links are not followed"))
    } else {
        Ok(())
    }
}

fn open_dir(dir: &Dir, name: &std::ffi::OsStr) -> std::io::Result<Dir> {
    not_a_link(dir, name)?;
    dir.open_dir(name)
}

/// The bytes of `path`: through the active root when a tool call is
/// running, otherwise straight from the file system.
pub fn read(path: &Path) -> std::io::Result<Vec<u8>> {
    match ACTIVE.with(|active| active.borrow().clone()) {
        Some(root) => root.read(path),
        None => std::fs::read(path),
    }
}
