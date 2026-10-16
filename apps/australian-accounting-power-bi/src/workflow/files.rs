use super::Result;
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

pub(super) fn linked(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    false
}

fn local_prefix(text: &str) -> bool {
    !text
        .as_bytes()
        .get(..2)
        .is_some_and(|prefix| prefix.iter().all(|byte| b"/\\".contains(byte)) || prefix.eq_ignore_ascii_case(b"Z:"))
}

pub fn ordinary(path: &Path) -> Result<PathBuf> {
    if !local_prefix(&path.as_os_str().to_string_lossy()) {
        return Err("Use a local directory.".into());
    }
    let absolute = std::path::absolute(path).map_err(|error| error.to_string())?;
    if !local_prefix(&absolute.as_os_str().to_string_lossy()) {
        return Err("Use a local directory.".into());
    }
    let (mut current, mut missing_depth) = (PathBuf::new(), 0usize);
    for component in absolute.components() {
        if component == Component::ParentDir {
            current.pop();
            missing_depth = missing_depth.saturating_sub(1);
            continue;
        }
        if component == Component::CurDir {
            continue;
        }
        current.push(component.as_os_str());
        if missing_depth > 0 {
            missing_depth += 1;
            continue;
        }
        match fs::symlink_metadata(&current) {
            Ok(metadata) if linked(&metadata) => return Err("Linked paths are not admitted.".into()),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => missing_depth = 1,
            Err(error) => return Err(error.to_string()),
        }
    }
    resolve_existing_prefix(current)
}

fn resolve_existing_prefix(mut path: PathBuf) -> Result<PathBuf> {
    let mut missing = Vec::new();
    loop {
        match fs::canonicalize(&path) {
            Ok(mut resolved) => {
                #[cfg(windows)]
                {
                    use std::os::windows::ffi::{OsStrExt, OsStringExt};
                    let wide: Vec<_> = resolved.as_os_str().encode_wide().collect();
                    if wide.starts_with(&[92, 92, 63, 92]) {
                        resolved = std::ffi::OsString::from_wide(&wide[4..]).into();
                    }
                }
                for component in missing.into_iter().rev() {
                    resolved.push(component);
                }
                return Ok(resolved);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                missing.push(path.file_name().ok_or("Cannot resolve the local path.")?.to_owned());
                if !path.pop() {
                    return Err("Cannot resolve the local path.".into());
                }
            }
            Err(error) => return Err(error.to_string()),
        }
    }
}

pub fn new_output(path: &Path) -> Result<PathBuf> {
    let path = ordinary(path)?;
    if path.try_exists().map_err(|error| error.to_string())? {
        return Err("Output must be new.".into());
    }
    if path.ancestors().skip(1).any(|parent| {
        [".git", ".hg", ".svn", ".bzr"]
            .iter()
            .any(|marker| parent.join(marker).exists())
    }) {
        return Err("Generated reviews must be outside version control.".into());
    }
    Ok(path)
}

pub fn open_regular(path: &Path) -> Result<File> {
    let path = ordinary(path)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x0020_0000);
    }
    let file = options.open(path).map_err(|error| error.to_string())?;
    if !file.metadata().map_err(|error| error.to_string())?.is_file() {
        return Err("Input must be a regular file.".into());
    }
    Ok(file)
}

pub fn read(path: &Path) -> Result<Vec<u8>> {
    let mut data = Vec::new();
    open_regular(path)?
        .read_to_end(&mut data)
        .map_err(|error| error.to_string())?;
    Ok(data)
}

pub fn bounded_read(path: &Path, maximum: usize) -> Result<Vec<u8>> {
    let mut data = Vec::new();
    open_regular(path)?
        .take(maximum as u64 + 1)
        .read_to_end(&mut data)
        .map_err(|error| error.to_string())?;
    if data.len() > maximum {
        return Err("Export archive exceeds the size limit.".into());
    }
    Ok(data)
}

#[derive(Debug, PartialEq, Eq)]
struct Identity(u64, u64);

#[cfg(unix)]
fn identity(file: &File) -> Result<Identity> {
    use std::os::unix::fs::MetadataExt;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    Ok(Identity(metadata.dev(), metadata.ino()))
}

#[cfg(windows)]
fn information(file: &File) -> Result<windows_sys::Win32::Storage::FileSystem::BY_HANDLE_FILE_INFORMATION> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle};
    let mut information = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::zeroed();
    // SAFETY: the file owns its handle and the output has the required size.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), information.as_mut_ptr()) } == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    // SAFETY: the successful call initialised information.
    Ok(unsafe { information.assume_init() })
}

#[cfg(windows)]
fn identity(file: &File) -> Result<Identity> {
    let metadata = information(file)?;
    Ok(Identity(
        u64::from(metadata.dwVolumeSerialNumber),
        (u64::from(metadata.nFileIndexHigh) << 32) | u64::from(metadata.nFileIndexLow),
    ))
}

pub fn single_link(file: &File) -> Result<bool> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(file.metadata().map_err(|error| error.to_string())?.nlink() == 1)
    }
    #[cfg(windows)]
    {
        Ok(information(file)?.nNumberOfLinks == 1)
    }
}

pub fn write_new(path: &Path, data: &[u8]) -> Result<()> {
    write_new_with(path, |file| {
        file.write_all(data)?;
        file.flush()
    })
}

fn write_new_with(path: &Path, write: impl FnOnce(&mut File) -> std::io::Result<()>) -> Result<()> {
    write_identified(path, write, identity)
}

fn write_identified(
    path: &Path,
    write: impl FnOnce(&mut File) -> std::io::Result<()>,
    mut identify: impl FnMut(&File) -> Result<Identity>,
) -> Result<()> {
    let path = ordinary(path)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|error| error.to_string())?;
    let created = match identify(&file) {
        Ok(created) => created,
        Err(error) => {
            let recovered = identify(&file).ok();
            drop(file);
            remove_created(&path, recovered)?;
            return Err(error);
        }
    };
    let result = write(&mut file);
    drop(file);
    if result.is_err() {
        remove_created(&path, Some(created))?;
    }
    result.map_err(|error| error.to_string())
}

fn remove_created(path: &Path, created: Option<Identity>) -> Result<()> {
    if created.is_some() && open_regular(path).and_then(|file| identity(&file)).ok() == created {
        fs::remove_file(path).map_err(|error| error.to_string())?;
    }
    Ok(())
}

pub struct Temporary(pub PathBuf);

impl Temporary {
    pub fn new(prefix: &str) -> Result<Self> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos();
        let base = ordinary(&std::env::temp_dir())?;
        for _ in 0..100 {
            let sequence = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = base.join(format!("{prefix}{}-{stamp}-{sequence}", std::process::id()));
            let builder = fs::DirBuilder::new();
            #[cfg(unix)]
            let builder = {
                use std::os::unix::fs::DirBuilderExt;
                let mut builder = builder;
                builder.mode(0o700);
                builder
            };
            match builder.create(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.to_string()),
            }
        }
        Err("Could not create a temporary review directory.".into())
    }
}

impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_refuses_existing_and_versioned_paths() {
        let temporary = Temporary::new("pbi-files-test-").unwrap();
        let output = temporary.0.join("output");
        write_new(&output, b"first").unwrap();
        assert!(write_new(&output, b"second").is_err());
        assert_eq!(read(&output).unwrap(), b"first");
        fs::write(temporary.0.join(".git"), b"fixture marker").unwrap();
        assert!(new_output(&temporary.0.join("missing")).is_err());
        assert!(ordinary(Path::new("Z:/private")).is_err());
        assert!(ordinary(Path::new("\\\\server\\private")).is_err());
        for path in [
            "//server/share/private",
            "\\/server/share/private",
            "/\\server/share/private",
            "//?/C:/private",
            "z:relative",
        ] {
            assert!(!local_prefix(path), "{path}");
        }
        for path in ["/tmp/local", "C:/local", "relative/local"] {
            assert!(local_prefix(path), "{path}");
        }
    }

    #[test]
    fn failed_write_removes_only_its_own_file() {
        let temporary = Temporary::new("pbi-files-test-").unwrap();
        let output = temporary.0.join("output");
        let result = write_new_with(&output, |file| {
            file.write_all(b"partial")?;
            Err(std::io::Error::other("fixture failure"))
        });
        assert!(result.is_err());
        assert!(!output.exists());
        let result = write_new_with(&output, |_| {
            fs::rename(&output, temporary.0.join("displaced"))?;
            fs::write(&output, b"replacement")?;
            Err(std::io::Error::other("fixture replacement"))
        });
        assert!(result.is_err());
        assert_eq!(read(&output).unwrap(), b"replacement");
    }

    #[test]
    fn identity_failure_cleans_only_when_the_owned_descriptor_can_still_identify_it() {
        let temporary = Temporary::new("pbi-identity-failure-").unwrap();
        let output = temporary.0.join("output");
        let mut calls = 0;
        let result = write_identified(
            &output,
            |_| panic!("write follows successful identity"),
            |file| {
                calls += 1;
                if calls == 1 {
                    Err("fixture initial identity failure".into())
                } else {
                    identity(file)
                }
            },
        );
        assert_eq!(result.unwrap_err(), "fixture initial identity failure");
        assert!(!output.exists());
        let result = write_identified(
            &output,
            |_| panic!("write follows successful identity"),
            |_| {
                if !temporary.0.join("displaced").exists() {
                    fs::rename(&output, temporary.0.join("displaced")).unwrap();
                    fs::write(&output, b"another writer").unwrap();
                }
                Err("fixture unavailable descriptor identity".into())
            },
        );
        assert!(result.is_err());
        assert_eq!(read(&output).unwrap(), b"another writer");
    }

    #[test]
    fn linked_ancestors_are_refused_even_after_a_missing_subtree() {
        let temporary = Temporary::new("pbi-linked-path-").unwrap();
        let target = Temporary::new("pbi-link-target-").unwrap();
        let link = temporary.0.join("linked");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target.0, &link).unwrap();
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            let output = std::process::Command::new("cmd")
                .args(["/c", "mklink", "/J"])
                .arg(&link)
                .arg(&target.0)
                .creation_flags(0x0800_0000)
                .output()
                .unwrap();
            assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        }
        for path in [link.join("child"), temporary.0.join("missing/../linked/child")] {
            assert!(ordinary(&path).unwrap_err().contains("Linked paths"));
            assert!(read(&path).is_err());
        }
        assert_eq!(
            ordinary(&temporary.0.join("missing/../local-output")).unwrap(),
            temporary.0.join("local-output")
        );
    }

    #[cfg(unix)]
    #[test]
    fn fifo_inputs_are_refused_without_waiting_for_a_writer() {
        use std::os::unix::ffi::OsStrExt;
        let temporary = Temporary::new("pbi-fifo-").unwrap();
        let path = temporary.0.join("pipe");
        let name = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        // SAFETY: name is terminated and remains live for this call.
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        let started = std::time::Instant::now();
        assert!(read(&path).unwrap_err().contains("regular file"));
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
    }

    #[cfg(windows)]
    #[test]
    fn ordinary_resolves_windows_spelling_before_appending_missing_output_names() {
        use std::os::windows::ffi::{OsStrExt, OsStringExt};
        use windows_sys::Win32::Storage::FileSystem::GetShortPathNameW;
        let temporary = Temporary::new("pbi-path-spelling-").unwrap();
        let wide: Vec<_> = temporary.0.as_os_str().encode_wide().chain(Some(0)).collect();
        // SAFETY: the input is terminated; the first call requests the output length.
        let size = unsafe { GetShortPathNameW(wide.as_ptr(), std::ptr::null_mut(), 0) };
        assert_ne!(size, 0);
        let mut short = vec![0; size as usize];
        // SAFETY: both buffers have their declared sizes and remain live.
        let length = unsafe { GetShortPathNameW(wide.as_ptr(), short.as_mut_ptr(), size) };
        assert!(length > 0 && length < size);
        let alias = PathBuf::from(std::ffi::OsString::from_wide(&short[..length as usize]));
        assert_eq!(
            ordinary(&alias.join("new-output")).unwrap(),
            temporary.0.join("new-output")
        );
        let lower = PathBuf::from(temporary.0.to_str().unwrap().to_ascii_lowercase());
        assert_eq!(
            ordinary(&lower.join("new-output")).unwrap(),
            temporary.0.join("new-output")
        );
    }
}
