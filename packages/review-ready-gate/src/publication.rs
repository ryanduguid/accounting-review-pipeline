// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! Cooperating writers publish three fixed slots using persisted transaction evidence.
//! State replacement gives atomic visibility, without a power-loss durability claim.
use crate::{
    pyjson::{self, Json},
    pypath,
    report::PACK_NAMES,
};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

pub const CONTROL_NAME: &str = ".reviewready";
static CANCELLED: AtomicBool = AtomicBool::new(false);
static MANAGED: AtomicBool = AtomicBool::new(false);
fn error(message: &str) -> io::Error {
    io::Error::other(message)
}
fn cancellation() -> io::Result<()> {
    if MANAGED.load(Ordering::Relaxed) && CANCELLED.load(Ordering::Relaxed) {
        Err(error("pack publication cancelled by SIGTERM"))
    } else {
        Ok(())
    }
}
extern "C" fn request_cancel(_: i32) {
    CANCELLED.store(true, Ordering::Relaxed);
}

#[cfg(unix)]
fn install_unix_handler(signal: i32) -> Option<usize> {
    let mut action = std::mem::MaybeUninit::<libc::sigaction>::zeroed();
    // SAFETY: initializes writable storage without changing a signal handler.
    // nosemgrep: rust.lang.security.unsafe-usage.unsafe-usage
    if unsafe { libc::sigaction(signal, std::ptr::null(), action.as_mut_ptr()) } != 0 {
        return None;
    }
    // SAFETY: the successful query initialized action.
    // nosemgrep: rust.lang.security.unsafe-usage.unsafe-usage
    let action = unsafe { action.assume_init() };
    if action.sa_sigaction != libc::SIG_DFL {
        return None;
    }
    let handler = request_cancel as *const () as usize;
    // SAFETY: the handler has static lifetime and only stores an atomic flag.
    // nosemgrep: rust.lang.security.unsafe-usage.unsafe-usage
    let previous = unsafe { libc::signal(signal, handler) };
    if previous == libc::SIG_ERR {
        return None;
    }
    Some(previous)
}

pub struct ManagedPublication {
    previous: Option<usize>,
}
impl ManagedPublication {
    pub fn new() -> Self {
        CANCELLED.store(false, Ordering::Relaxed);
        #[cfg(unix)]
        let previous = install_unix_handler(libc::SIGTERM);
        #[cfg(windows)]
        let previous = {
            // Windows process termination remains abrupt; C signal delivery is managed.
            // SAFETY: the handler only stores an atomic flag and has static lifetime.
            let old = unsafe { win_signal(15, request_cancel as *const () as usize) };
            if old == 0 {
                Some(old)
            } else {
                if old != usize::MAX {
                    // SAFETY: puts back the handler signal() just returned.
                    // nosemgrep: rust.lang.security.unsafe-usage.unsafe-usage
                    unsafe { win_signal(15, old) };
                }
                None
            }
        };
        MANAGED.store(previous.is_some(), Ordering::Relaxed);
        Self { previous }
    }
}
impl Default for ManagedPublication {
    fn default() -> Self {
        Self::new()
    }
}
impl Drop for ManagedPublication {
    fn drop(&mut self) {
        MANAGED.store(false, Ordering::Relaxed);
        if let Some(previous) = self.previous {
            #[cfg(unix)]
            // SAFETY: previous came from signal() and is restored before this guard ends.
            unsafe {
                libc::signal(libc::SIGTERM, previous);
            }
            #[cfg(windows)]
            // SAFETY: previous came from signal() and is restored before this guard ends.
            unsafe {
                win_signal(15, previous);
            }
        }
    }
}
#[cfg(windows)]
unsafe extern "C" {
    #[link_name = "signal"]
    fn win_signal(signal: i32, handler: usize) -> usize;
}

pub fn single_link(_path: &Path, metadata: &fs::Metadata) -> io::Result<bool> {
    if !metadata.is_file() {
        return Ok(false);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(metadata.nlink() == 1)
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        let file = OpenOptions::new().access_mode(0).open(_path)?;
        single_link_handle(&file)
    }
}

#[cfg(windows)]
fn single_link_handle(file: &File) -> io::Result<bool> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

    let info = pypath::file_information_by_handle(file.as_raw_handle())?;
    Ok(info.nNumberOfLinks == 1 && info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT == 0)
}

pub(crate) fn read_single_link(path: &Path) -> io::Result<Option<Vec<u8>>> {
    if !fs::symlink_metadata(path)?.is_file() {
        return Ok(None);
    }
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
        use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    // Validate the same file we read, even if the directory entry has changed.
    let mut file = options.open(path)?;
    let metadata = file.metadata()?;
    #[cfg(unix)]
    let allowed = single_link(path, &metadata)?;
    #[cfg(windows)]
    let allowed = metadata.is_file() && single_link_handle(&file)?;
    if !allowed {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(Some(bytes))
}

fn non_regular_file(path: &Path) -> io::Error {
    error(&format!(
        "publication refuses a non-regular or linked file: {}",
        pypath::decode_os(path.file_name().unwrap_or_default())
    ))
}

fn regular(path: &Path, absent: bool) -> io::Result<bool> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if absent && e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e),
    };
    if !single_link(path, &metadata)? {
        return Err(non_regular_file(path));
    }
    Ok(true)
}
fn directory(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    #[cfg(windows)]
    let reparse = {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    };
    #[cfg(not(windows))]
    let reparse = false;
    if !metadata.is_dir() || reparse {
        return Err(error(&format!(
            "publication refuses a non-directory or link: {}",
            pypath::decode_os(path.file_name().unwrap_or_default())
        )));
    }
    Ok(())
}
fn identity() -> io::Result<String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|e| error(&e.to_string()))?;
    bytes[6] = (bytes[6] & 15) | 64;
    bytes[8] = (bytes[8] & 63) | 128;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
fn hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
#[derive(Clone, Debug)]
struct State {
    status: String,
    revision: String,
    transaction: Option<String>,
    previous: [Option<String>; 3],
}
impl State {
    fn json(&self) -> Json {
        Json::object([
            ("format", Json::int(1)),
            ("status", Json::str(&self.status)),
            ("revision", Json::str(&self.revision)),
            (
                "transaction",
                self.transaction
                    .as_ref()
                    .map_or(Json::Null, |s| Json::str(s)),
            ),
            (
                "previous",
                Json::Array(
                    self.previous
                        .iter()
                        .map(|s| s.as_ref().map_or(Json::Null, |v| Json::str(v)))
                        .collect(),
                ),
            ),
        ])
    }
}
#[expect(
    clippy::too_many_lines,
    reason = "reads and validates the publication state"
)]
fn read_state(control: &Path) -> io::Result<State> {
    directory(control)?;
    let path = control.join("state.json");
    let bytes = read_single_link(&path)?.ok_or_else(|| non_regular_file(&path))?;
    let malformed = || error("publication state is malformed; retain recovery evidence");
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&bytes);
    let text = std::str::from_utf8(bytes).map_err(|_| malformed())?;
    let state = pyjson::loads(text).map_err(|_| malformed())?;
    let unsupported = || error("publication state is unsupported; retain recovery evidence");
    if !state.has_exact_keys(&["format", "status", "revision", "transaction", "previous"])
        || state
            .at("format")
            .as_plain_int()
            .is_none_or(|v| pyjson::int_cmp(v, 1) != std::cmp::Ordering::Equal)
    {
        return Err(unsupported());
    }
    let text = |key| crate::loader::json_text(state.at(key)).ok_or_else(unsupported);
    let status = text("status")?;
    let revision = text("revision")?;
    if !["settled", "in_progress"].contains(&status.as_str()) || !hex(&revision, 32) {
        return Err(unsupported());
    }
    let transaction = if matches!(state.at("transaction"), Json::Null) {
        None
    } else {
        let id = text("transaction")?;
        if !hex(&id, 32) {
            return Err(unsupported());
        }
        Some(id)
    };
    if status == "in_progress" && transaction.is_none() {
        return Err(unsupported());
    }
    let Json::Array(previous) = state.at("previous") else {
        return Err(unsupported());
    };
    if previous.len() != 3 {
        return Err(unsupported());
    }
    let mut saved = [None, None, None];
    for (i, v) in previous.iter().enumerate() {
        if !matches!(v, Json::Null) {
            let hash = crate::loader::json_text(v).ok_or_else(unsupported)?;
            if !hex(&hash, 64) {
                return Err(unsupported());
            }
            saved[i] = Some(hash);
        }
    }
    Ok(State {
        status,
        revision,
        transaction,
        previous: saved,
    })
}
fn write_state(control: &Path, state: &State) -> io::Result<()> {
    let temporary = control.join("state.next");
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?
        .write_all((pyjson::dumps(&state.json(), true) + "\n").as_bytes())?;
    fs::rename(temporary, control.join("state.json"))
}
pub fn snapshot_revision(output: &Path) -> io::Result<Option<String>> {
    let control = output.join(CONTROL_NAME);
    match fs::symlink_metadata(&control) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
        Ok(_) => {}
    }
    let state = read_state(&control)?;
    if state.status != "settled" {
        return Err(error(
            "pack publication is unresolved; retain recovery evidence",
        ));
    }
    Ok(Some(state.revision))
}

struct Admission {
    file: File,
}
impl Admission {
    fn acquire(control: &Path) -> io::Result<Self> {
        match fs::create_dir(control) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
        directory(control)?;
        let lock = control.join("writer.lock");
        regular(&lock, true)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let mut file = options.open(lock)?;
        if file.metadata()?.len() == 0 {
            file.write_all(b"1")?;
        }
        file.seek(SeekFrom::Start(0))?;
        lock_file(&file, true).map_err(|_| error("another pack writer holds admission"))?;
        Ok(Self { file })
    }
}
impl Drop for Admission {
    fn drop(&mut self) {
        let _ = lock_file(&self.file, false);
    }
}
#[cfg(unix)]
fn lock_file(file: &File, acquire: bool) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    // SAFETY: file owns the descriptor throughout the nonblocking lock operation.
    if unsafe {
        libc::flock(
            file.as_raw_fd(),
            if acquire {
                libc::LOCK_EX | libc::LOCK_NB
            } else {
                libc::LOCK_UN
            },
        )
    } == 0
    {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}
#[cfg(windows)]
fn lock_file(file: &File, acquire: bool) -> io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::{
        Storage::FileSystem::{
            LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY, LockFileEx, UnlockFileEx,
        },
        System::IO::OVERLAPPED,
    };
    let mut overlap: OVERLAPPED = unsafe { std::mem::zeroed() };
    // SAFETY: the synchronous file handle remains open and overlap describes byte zero.
    let ok = unsafe {
        if acquire {
            LockFileEx(
                file.as_raw_handle(),
                LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
                0,
                1,
                0,
                &mut overlap,
            )
        } else {
            UnlockFileEx(file.as_raw_handle(), 0, 1, 0, &mut overlap)
        }
    };
    if ok != 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}
fn entries(path: &Path) -> io::Result<Vec<PathBuf>> {
    fs::read_dir(path)?.map(|p| p.map(|p| p.path())).collect()
}
fn names(paths: &[PathBuf]) -> BTreeSet<String> {
    paths
        .iter()
        .map(|p| pypath::decode_os(p.file_name().unwrap_or_default()))
        .collect()
}
fn validate_control(control: &Path, state: &State) -> io::Result<()> {
    let mut allowed = BTreeSet::from(["writer.lock".into(), "state.json".into()]);
    if let Some(tx) = &state.transaction {
        allowed.insert(format!("tx-{tx}"));
    }
    if !names(&entries(control)?).is_subset(&allowed) {
        return Err(error(
            "unknown publication control entries; retain recovery evidence",
        ));
    }
    regular(&control.join("writer.lock"), false)?;
    Ok(())
}
type Observer<'a> = &'a mut dyn FnMut(&str) -> io::Result<()>;
fn cleanup(control: &Path, state: &State, observer: Observer<'_>) -> io::Result<()> {
    validate_control(control, state)?;
    let Some(id) = &state.transaction else {
        return Ok(());
    };
    let transaction = control.join(format!("tx-{id}"));
    directory(&transaction).map_err(|e| {
        if e.kind() == io::ErrorKind::NotFound {
            error("referenced publication transaction is missing; retain control for review")
        } else {
            e
        }
    })?;
    let paths = entries(&transaction)?;
    let owned: BTreeSet<_> = ["stage", "backup", "restore"]
        .iter()
        .flat_map(|kind| (0..3).map(move |i| format!("{kind}-{i}")))
        .collect();
    if !names(&paths).is_subset(&owned) {
        return Err(error(
            "unknown transaction entries; retain recovery evidence",
        ));
    }
    for path in &paths {
        regular(path, false)?;
    }
    for path in paths {
        fs::remove_file(path)?;
    }
    fs::remove_dir(transaction)?;
    let mut cleared = state.clone();
    cleared.transaction = None;
    cleared.previous = [None, None, None];
    observer("cleanup-state")
        .and_then(|()| write_state(control, &cleared))
        .map_err(|_| error("cleanup state uncertain; retain control for review"))
}
fn initialise(control: &Path) -> io::Result<()> {
    if !control.join("state.json").exists() {
        if names(&entries(control)?) != BTreeSet::from(["writer.lock".into()]) {
            return Err(error(
                "uninitialised publication control; retain recovery evidence",
            ));
        }
        write_state(
            control,
            &State {
                status: "settled".into(),
                revision: identity()?,
                transaction: None,
                previous: [None, None, None],
            },
        )?;
    }
    Ok(())
}
fn copy_new(source: &Path, destination: &Path) -> io::Result<()> {
    let mut source = File::open(source)?;
    let mut sink = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    io::copy(&mut source, &mut sink)?;
    Ok(())
}
fn backup_digest(mut reader: impl Read) -> io::Result<String> {
    let mut digest = Sha256::new();
    let mut buffer = [0; 16 * 1024];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => return Ok(crate::sha256_hex(&digest.finalize())),
            Ok(bytes) => digest.update(&buffer[..bytes]),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
}
fn checkpoint(observer: Observer<'_>, name: &str) -> io::Result<()> {
    observer(name)?;
    cancellation()
}
fn backup_slots(
    output: &Path,
    tx: &Path,
    observer: Observer<'_>,
) -> io::Result<[Option<String>; 3]> {
    let mut previous = [None, None, None];
    for (i, name) in PACK_NAMES.iter().enumerate() {
        let destination = output.join(name);
        if regular(&destination, true)? {
            let backup = tx.join(format!("backup-{i}"));
            copy_new(&destination, &backup)?;
            previous[i] = Some(backup_digest(File::open(&backup)?)?);
        }
        checkpoint(observer, &format!("backup-{i}"))?;
    }
    Ok(previous)
}
fn stage_slots(tx: &Path, rendered: &[Vec<u8>; 3], observer: Observer<'_>) -> io::Result<()> {
    for (i, bytes) in rendered.iter().enumerate() {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(tx.join(format!("stage-{i}")))?
            .write_all(bytes)?;
        checkpoint(observer, &format!("stage-{i}"))?;
    }
    Ok(())
}
fn restore_slots(
    output: &Path,
    tx: &Path,
    state: &State,
    observer: Observer<'_>,
) -> io::Result<()> {
    for (i, name) in PACK_NAMES.iter().enumerate() {
        let destination = output.join(name);
        if let Some(hash) = &state.previous[i] {
            let backup = tx.join(format!("backup-{i}"));
            regular(&backup, false)?;
            if &backup_digest(File::open(&backup)?)? != hash {
                return Err(error("recovery backup differs; retain recovery evidence"));
            }
            if regular(&destination, true)? && &crate::sha256(&fs::read(&destination)?) == hash {
                continue;
            }
            let restore = tx.join(format!("restore-{i}"));
            copy_new(&backup, &restore)?;
            fs::rename(restore, destination)?;
        } else if regular(&destination, true)? {
            fs::remove_file(destination)?;
        }
        observer(&format!("restore-{i}"))?;
    }
    Ok(())
}
fn recover(
    control: &Path,
    output: &Path,
    tx: &Path,
    state: &State,
    backups_complete: bool,
    commit: &str,
    observer: Observer<'_>,
) -> io::Result<()> {
    let observed = read_state(control)
        .map_err(|_| error("publication state uncertain; retain recovery evidence"))?;
    if observed.status == "settled" && observed.revision == commit {
        return Err(error(
            "pack published; cleanup incomplete, inspect retained control before retry",
        ));
    }
    if observed.transaction != state.transaction {
        return match fs::remove_dir(tx) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(error(
                "unrecorded transaction remains; retain control for review",
            )),
        };
    }
    let restore = || -> io::Result<()> {
        if backups_complete {
            restore_slots(output, tx, state, observer)?;
        }
        let mut settled = state.clone();
        settled.status = "settled".into();
        settled.revision = identity()?;
        write_state(control, &settled)?;
        cleanup(control, &settled, observer)
    };
    let mut restore = restore;
    restore().map_err(|_| {
        error("publication or recovery incomplete; retain control and backups for review")
    })
}
pub fn publish(output: &Path, rendered: &[Vec<u8>; 3]) -> io::Result<()> {
    publish_observed(output, rendered, &mut |_| Ok(()))
}
#[expect(
    clippy::too_many_lines,
    reason = "checks the observed publication in one place"
)]
fn publish_observed(
    output: &Path,
    rendered: &[Vec<u8>; 3],
    observer: Observer<'_>,
) -> io::Result<()> {
    for name in PACK_NAMES {
        regular(&output.join(name), true)?;
    }
    let control = output.join(CONTROL_NAME);
    let _admission = Admission::acquire(&control)?;
    initialise(&control)?;
    checkpoint(observer, "initialised")?;
    let mut state = read_state(&control)?;
    if state.status != "settled" {
        return Err(error(
            "unresolved pack publication; retain recovery evidence for review",
        ));
    }
    cleanup(&control, &state, observer)?;
    checkpoint(observer, "previous-cleanup")?;
    let id = identity()?;
    state.status = "in_progress".into();
    state.transaction = Some(id.clone());
    state.previous = [None, None, None];
    let tx = control.join(format!("tx-{id}"));
    let commit = identity()?;
    let mut backups_complete = false;
    let publish_result = (|| -> io::Result<()> {
        fs::create_dir(&tx)?;
        observer("transaction-created")?;
        write_state(&control, &state)?;
        checkpoint(observer, "in-progress")?;
        let previous = backup_slots(output, &tx, observer)?;
        stage_slots(&tx, rendered, observer)?;
        state.previous = previous;
        write_state(&control, &state)?;
        backups_complete = true;
        checkpoint(observer, "prepared")?;
        for (i, name) in PACK_NAMES.iter().enumerate() {
            fs::rename(tx.join(format!("stage-{i}")), output.join(name))?;
            checkpoint(observer, &format!("install-{i}"))?;
        }
        let mut settled = state.clone();
        settled.status = "settled".into();
        settled.revision = commit.clone();
        write_state(&control, &settled)?;
        checkpoint(observer, "committed")
    })();
    if let Err(failure) = publish_result {
        recover(
            &control,
            output,
            &tx,
            &state,
            backups_complete,
            &commit,
            observer,
        )?;
        return Err(failure);
    }
    let settled = read_state(&control)?;
    if let Err(e) = cleanup(&control, &settled, observer) {
        return Err(error(
            if e.to_string() == "cleanup state uncertain; retain control for review" {
                "pack published; cleanup state uncertain, requires review"
            } else {
                "pack published; cleanup incomplete, inspect retained control before retry"
            },
        ));
    }
    checkpoint(observer, "finished").map_err(|_| error("pack published; cancellation after commit"))
}

/// Feature-gated process synchronisation for publication death and signal checks.
#[cfg(feature = "probe")]
pub fn process_probe() -> i32 {
    use std::io::{BufRead, Write};
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 3 {
        eprintln!("usage: review-ready-publication-probe OUTPUT CHECKPOINT PREFIX");
        return 1;
    }
    let output = PathBuf::from(&args[0]);
    let point = args[1].to_string_lossy();
    let prefix = args[2].to_string_lossy();
    let output = match crate::report::require_output_outside_repository(&output) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    if let Err(e) = fs::create_dir_all(&output) {
        eprintln!("{e}");
        return 1;
    }
    let rendered = std::array::from_fn(|i| format!("{prefix}-{i}").into_bytes());
    let _managed = ManagedPublication::new();
    let result = publish_observed(&output, &rendered, &mut |observed| {
        if observed == point {
            println!("checkpoint {observed}");
            io::stdout().flush()?;
            let mut line = String::new();
            io::stdin().lock().read_line(&mut line)?;
            if line.trim() == "cancel" {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
                while !CANCELLED.load(Ordering::Relaxed) {
                    if std::time::Instant::now() >= deadline {
                        return Err(error("signal checkpoint timed out"));
                    }
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
            }
        }
        Ok(())
    });
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    #[cfg(unix)]
    #[test]
    fn failed_signal_query_or_registration_keeps_no_handler() {
        assert!(install_unix_handler(-1).is_none());
        assert!(install_unix_handler(libc::SIGKILL).is_none());
    }

    #[test]
    fn managed_publication_keeps_the_process_existing_signal_handler() {
        const CHILD: &str = "ARP_TEST_MANAGED_PUBLICATION_SIGNAL_CHILD";
        if std::env::var_os(CHILD).is_none() {
            // This test runs its own test binary with fixed arguments.
            // nosemgrep: rust.lang.security.current-exe.current-exe
            let output = crate::child_profile::command(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "publication::tests::managed_publication_keeps_the_process_existing_signal_handler",
                ])
                .env(CHILD, "1")
                .output()
                .unwrap();
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(
                output.status.success() && stdout.contains("1 passed"),
                "{stdout}{stderr}"
            );
            return;
        }
        #[cfg(unix)]
        let (set_signal, term, ignored) = (libc::signal, libc::SIGTERM, libc::SIG_IGN);
        #[cfg(windows)]
        let (set_signal, term, ignored) = (win_signal, 15, 1);

        // SAFETY: change SIGTERM in this one-test child without sending a signal.
        // nosemgrep: rust.lang.security.unsafe-usage.unsafe-usage
        let previous = unsafe { set_signal(term, ignored) };
        assert_ne!(previous, usize::MAX);
        {
            let managed = ManagedPublication::new();
            assert!(managed.previous.is_none());
        }
        // SAFETY: restore the handler returned by the successful initial call.
        // nosemgrep: rust.lang.security.unsafe-usage.unsafe-usage
        let restored = unsafe { set_signal(term, previous) };
        assert_eq!(restored, ignored);
    }

    #[cfg(windows)]
    #[test]
    fn windows_file_information_preserves_the_kernel_error() {
        use windows_sys::Win32::Foundation::ERROR_INVALID_HANDLE;

        let error = pypath::file_information_by_handle(std::ptr::null_mut())
            .err()
            .expect("Windows must refuse an invalid file handle");
        assert_eq!(error.raw_os_error(), Some(ERROR_INVALID_HANDLE as i32));
    }
    fn rendered(prefix: &str) -> [Vec<u8>; 3] {
        std::array::from_fn(|i| format!("{prefix}-{i}").into_bytes())
    }
    fn payloads(dir: &Path) -> [Option<Vec<u8>>; 3] {
        PACK_NAMES.map(|name| fs::read(dir.join(name)).ok())
    }
    fn assert_settled(dir: &Path) {
        let state = read_state(&dir.join(CONTROL_NAME)).unwrap();
        assert_eq!(state.status, "settled");
        assert!(state.transaction.is_none());
        assert_eq!(state.previous, [None, None, None]);
        assert_eq!(
            names(&entries(&dir.join(CONTROL_NAME)).unwrap()),
            BTreeSet::from(["state.json".into(), "writer.lock".into()])
        );
    }
    #[test]
    fn missing_output_parent_refuses_publication_until_created() {
        let tmp = TempDir::new();
        let sentinel = tmp.join("sentinel.txt");
        fs::write(&sentinel, b"fabricated neighbouring artifact\n").unwrap();
        let output = tmp.join("missing/pack");
        assert_eq!(
            publish(&output, &rendered("new")).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        assert!(!tmp.join("missing").exists());
        assert_eq!(
            fs::read(&sentinel).unwrap(),
            b"fabricated neighbouring artifact\n"
        );
        fs::create_dir_all(&output).unwrap();
        publish(&output, &rendered("retry")).unwrap();
        assert_eq!(payloads(&output), rendered("retry").map(Some));
        assert_settled(&output);
        assert_eq!(
            fs::read(&sentinel).unwrap(),
            b"fabricated neighbouring artifact\n"
        );
    }

    #[test]
    fn oversized_state_versions_preserve_a_settled_generation_for_recovery() {
        for version in ["9223372036854775808", "-9223372036854775809"] {
            let tmp = TempDir::new();
            publish(&tmp.0, &rendered("old")).unwrap();
            let control = tmp.join(CONTROL_NAME);
            let path = control.join("state.json");
            let original = fs::read_to_string(&path).unwrap();
            let field = "\"format\": 1";
            assert_eq!(original.matches(field).count(), 1);
            let unsupported = original.replacen(field, &format!("\"format\": {version}"), 1);
            fs::write(&path, &unsupported).unwrap();
            let modified = fs::metadata(&path).unwrap().modified().unwrap();
            assert_eq!(
                publish(&tmp.0, &rendered("new")).unwrap_err().to_string(),
                "publication state is unsupported; retain recovery evidence"
            );
            assert_eq!(fs::read_to_string(&path).unwrap(), unsupported);
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
            assert_eq!(payloads(&tmp.0), rendered("old").map(Some));
            assert_eq!(
                names(&entries(&control).unwrap()),
                BTreeSet::from(["state.json".into(), "writer.lock".into()])
            );
            fs::write(&path, original).unwrap();
            publish(&tmp.0, &rendered("retry")).unwrap();
            assert_eq!(payloads(&tmp.0), rendered("retry").map(Some));
            assert_settled(&tmp.0);
        }
    }

    #[cfg(unix)]
    #[test]
    fn initial_state_permission_denial_preserves_legacy_payloads_for_retry() {
        use std::os::unix::fs::PermissionsExt;
        struct RestorePermissions<'a>(&'a Path, fs::Permissions);
        impl Drop for RestorePermissions<'_> {
            fn drop(&mut self) {
                let _ = fs::set_permissions(self.0, self.1.clone());
            }
        }
        let tmp = TempDir::new();
        for (name, bytes) in PACK_NAMES.into_iter().zip(rendered("old")) {
            fs::write(tmp.join(name), bytes).unwrap();
        }
        let control = tmp.join(CONTROL_NAME);
        drop(Admission::acquire(&control).unwrap());
        let restore = RestorePermissions(&control, fs::metadata(&control).unwrap().permissions());
        fs::set_permissions(&control, fs::Permissions::from_mode(0o500)).unwrap();
        let result = publish(&tmp.0, &rendered("new"));
        drop(restore);
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(payloads(&tmp.0), rendered("old").map(Some));
        assert_eq!(
            names(&entries(&control).unwrap()),
            BTreeSet::from(["writer.lock".into()])
        );
        assert_eq!(fs::read(control.join("writer.lock")).unwrap(), b"1");
        publish(&tmp.0, &rendered("retry")).unwrap();
        assert_eq!(payloads(&tmp.0), rendered("retry").map(Some));
        assert_settled(&tmp.0);
    }

    #[test]
    fn handled_precommit_failures_restore_every_existing_and_absent_slot() {
        for mask in 0..8 {
            for point in [
                "transaction-created",
                "in-progress",
                "backup-0",
                "backup-1",
                "backup-2",
                "stage-0",
                "stage-1",
                "stage-2",
                "prepared",
                "install-0",
                "install-1",
                "install-2",
            ] {
                let tmp = TempDir::new();
                for (i, name) in PACK_NAMES.iter().enumerate() {
                    if mask & (1 << i) != 0 {
                        fs::write(tmp.join(name), format!("old-{i}")).unwrap();
                    }
                }
                let before = payloads(&tmp.0);
                let mut failed = false;
                let result = publish_observed(&tmp.0, &rendered("new"), &mut |observed| {
                    if observed == point && !failed {
                        failed = true;
                        Err(error("injected failure"))
                    } else {
                        Ok(())
                    }
                });
                assert!(result.is_err(), "{mask}:{point}");
                assert!(failed, "{mask}:{point}");
                assert_eq!(before, payloads(&tmp.0), "{mask}:{point}");
                assert_settled(&tmp.0);
            }
        }
    }
    #[test]
    fn commit_observed_failure_retains_new_generation_and_readable_state() {
        let tmp = TempDir::new();
        publish(&tmp.0, &rendered("old")).unwrap();
        let before = snapshot_revision(&tmp.0).unwrap();
        let result = publish_observed(&tmp.0, &rendered("new"), &mut |point| {
            if point == "committed" {
                Err(error("after commit"))
            } else {
                Ok(())
            }
        });
        assert!(
            result
                .unwrap_err()
                .to_string()
                .starts_with("pack published;")
        );
        assert_eq!(payloads(&tmp.0), rendered("new").map(Some));
        assert_ne!(snapshot_revision(&tmp.0).unwrap(), before);
        publish(&tmp.0, &rendered("next")).unwrap();
        assert_settled(&tmp.0);
    }
    #[test]
    fn rollback_changes_revision_even_when_original_bytes_return() {
        let tmp = TempDir::new();
        publish(&tmp.0, &rendered("old")).unwrap();
        let before = snapshot_revision(&tmp.0).unwrap();
        let mut failed = false;
        let result = publish_observed(&tmp.0, &rendered("new"), &mut |point| {
            if point == "install-1" && !failed {
                failed = true;
                Err(error("failure"))
            } else {
                Ok(())
            }
        });
        assert!(result.is_err());
        assert_eq!(payloads(&tmp.0), rendered("old").map(Some));
        assert_ne!(snapshot_revision(&tmp.0).unwrap(), before);
        assert_settled(&tmp.0);
    }
    #[test]
    fn unknown_control_and_transaction_entries_survive_refusal() {
        for transaction in [false, true] {
            let tmp = TempDir::new();
            publish(&tmp.0, &rendered("old")).unwrap();
            let control = tmp.join(CONTROL_NAME);
            let mut state = read_state(&control).unwrap();
            let location = if transaction {
                let id = identity().unwrap();
                let path = control.join(format!("tx-{id}"));
                fs::create_dir(&path).unwrap();
                state.transaction = Some(id);
                write_state(&control, &state).unwrap();
                path
            } else {
                control
            };
            fs::write(location.join("unknown"), "retain me").unwrap();
            assert!(
                publish(&tmp.0, &rendered("new"))
                    .unwrap_err()
                    .to_string()
                    .contains("unknown")
            );
            assert_eq!(fs::read(location.join("unknown")).unwrap(), b"retain me");
            assert_eq!(payloads(&tmp.0), rendered("old").map(Some));
        }
    }
    #[test]
    fn cleanup_state_failure_keeps_commit_but_blocks_retry() {
        let tmp = TempDir::new();
        let result = publish_observed(&tmp.0, &rendered("new"), &mut |point| {
            if point == "cleanup-state" {
                Err(error("cannot clear state"))
            } else {
                Ok(())
            }
        });
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("cleanup state uncertain")
        );
        assert!(snapshot_revision(&tmp.0).unwrap().is_some());
        assert_eq!(payloads(&tmp.0), rendered("new").map(Some));
        assert!(
            publish(&tmp.0, &rendered("retry"))
                .unwrap_err()
                .to_string()
                .contains("transaction is missing")
        );
        let control = tmp.join(CONTROL_NAME);
        let id = read_state(&control).unwrap().transaction.unwrap();
        let transaction = control.join(format!("tx-{id}"));
        fs::write(&transaction, "retain me").unwrap();
        assert!(
            publish(&tmp.0, &rendered("retry"))
                .unwrap_err()
                .to_string()
                .contains("publication refuses a non-directory or link")
        );
        assert_eq!(fs::read(transaction).unwrap(), b"retain me");
        assert_eq!(payloads(&tmp.0), rendered("new").map(Some));
    }
    #[test]
    fn empty_and_lock_only_control_are_safely_initialised() {
        for lock_only in [false, true] {
            let tmp = TempDir::new();
            let control = tmp.join(CONTROL_NAME);
            fs::create_dir(&control).unwrap();
            if lock_only {
                fs::write(control.join("writer.lock"), b"").unwrap();
            }
            publish(&tmp.0, &rendered("new")).unwrap();
            assert!(snapshot_revision(&tmp.0).unwrap().is_some());
            assert_eq!(payloads(&tmp.0), rendered("new").map(Some));
        }
    }
    #[cfg(unix)]
    #[test]
    fn an_output_file_propagates_directory_errors_without_replacing_it() {
        let tmp = TempDir::new();
        let output = tmp.join("occupied");
        fs::write(&output, b"retain this file").unwrap();
        assert_eq!(
            publish(&output, &rendered("new")).unwrap_err().kind(),
            io::ErrorKind::NotADirectory
        );
        assert_eq!(
            snapshot_revision(&output).unwrap_err().kind(),
            io::ErrorKind::NotADirectory
        );
        assert_eq!(fs::read(&output).unwrap(), b"retain this file");
        assert_eq!(fs::read_dir(&tmp.0).unwrap().count(), 1);
    }
    #[test]
    fn uninitialised_control_preserves_unknown_evidence_and_payloads() {
        let tmp = TempDir::new();
        for (name, bytes) in PACK_NAMES.iter().zip(rendered("old")) {
            fs::write(tmp.join(name), bytes).unwrap();
        }
        let control = tmp.join(CONTROL_NAME);
        fs::create_dir(&control).unwrap();
        fs::write(control.join("writer.lock"), b"").unwrap();
        fs::write(control.join("unknown"), b"retain this evidence").unwrap();
        assert_eq!(
            publish(&tmp.0, &rendered("new")).unwrap_err().to_string(),
            "uninitialised publication control; retain recovery evidence"
        );
        assert_eq!(payloads(&tmp.0), rendered("old").map(Some));
        assert_eq!(
            fs::read(control.join("unknown")).unwrap(),
            b"retain this evidence"
        );
        assert_eq!(
            names(&entries(&control).unwrap()),
            BTreeSet::from(["unknown".into(), "writer.lock".into()])
        );
    }
    #[cfg(windows)]
    #[test]
    fn post_commit_cleanup_denial_is_retryable_after_the_handle_closes() {
        use std::os::windows::fs::OpenOptionsExt;
        let tmp = TempDir::new();
        publish(&tmp.0, &rendered("old")).unwrap();
        let mut held = None;
        let result = publish_observed(&tmp.0, &rendered("new"), &mut |point| {
            if point == "committed" {
                let control = tmp.join(CONTROL_NAME);
                let state = read_state(&control)?;
                let backup = control
                    .join(format!("tx-{}", state.transaction.unwrap()))
                    .join("backup-0");
                held = Some(OpenOptions::new().read(true).share_mode(0).open(backup)?);
            }
            Ok(())
        });
        assert!(result.is_err());
        assert!(snapshot_revision(&tmp.0).unwrap().is_some());
        assert_eq!(payloads(&tmp.0), rendered("new").map(Some));
        drop(held);
        publish(&tmp.0, &rendered("retry")).unwrap();
        assert_eq!(payloads(&tmp.0), rendered("retry").map(Some));
    }
    #[test]
    fn backup_digest_retries_interrupted_reads_and_preserves_other_errors() {
        struct ReadError(Option<io::Error>);
        impl Read for ReadError {
            fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
                self.0.take().map_or(Ok(0), Err)
            }
        }

        let interrupted = ReadError(Some(io::ErrorKind::Interrupted.into()));
        let input = b"earlier "
            .as_slice()
            .chain(interrupted)
            .chain(&b"pack"[..]);
        assert_eq!(
            backup_digest(input).unwrap(),
            crate::sha256(b"earlier pack")
        );

        let failed = ReadError(Some(io::Error::new(
            io::ErrorKind::InvalidData,
            "could not read remaining backup bytes",
        )));
        let error = backup_digest(b"earlier ".as_slice().chain(failed)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(error.to_string(), "could not read remaining backup bytes");
    }
    #[test]
    fn rollback_hashes_and_restores_the_entire_earlier_payload() {
        let tmp = TempDir::new();
        let mut old = rendered("old");
        old[0] = [b'a'; 16 * 1024].repeat(2);
        old[0].extend_from_slice(b"the tail must survive rollback");
        publish(&tmp.0, &old).unwrap();
        let expected = crate::sha256(&old[0]);
        let result = publish_observed(&tmp.0, &rendered("new"), &mut |point| {
            if point == "prepared" {
                let state = read_state(&tmp.join(CONTROL_NAME))?;
                assert_eq!(state.previous[0].as_deref(), Some(expected.as_str()));
            }
            if point == "install-1" {
                return Err(error("publication stopped after installing 2 files"));
            }
            Ok(())
        });
        assert_eq!(
            result.unwrap_err().to_string(),
            "publication stopped after installing 2 files"
        );
        assert_eq!(payloads(&tmp.0), old.map(Some));
        assert_settled(&tmp.0);
    }
    #[test]
    fn corrupt_backup_keeps_unresolved_recovery_evidence() {
        let tmp = TempDir::new();
        publish(&tmp.0, &rendered("old")).unwrap();
        let result = publish_observed(&tmp.0, &rendered("new"), &mut |point| {
            if point == "install-0" {
                let control = tmp.join(CONTROL_NAME);
                let state = read_state(&control)?;
                let backup = control
                    .join(format!("tx-{}", state.transaction.unwrap()))
                    .join("backup-0");
                fs::write(backup, "corrupt")?;
                Err(error("failure"))
            } else {
                Ok(())
            }
        });
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("recovery incomplete")
        );
        assert!(snapshot_revision(&tmp.0).is_err());
        assert!(publish(&tmp.0, &rendered("retry")).is_err());
        let state = read_state(&tmp.join(CONTROL_NAME)).unwrap();
        assert_eq!(state.status, "in_progress");
        assert!(
            tmp.join(CONTROL_NAME)
                .join(format!("tx-{}", state.transaction.unwrap()))
                .join("backup-0")
                .exists()
        );
    }
    #[cfg(unix)]
    #[test]
    fn post_commit_cleanup_permission_denial_is_retryable() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = TempDir::new();
        publish(&tmp.0, &rendered("old")).unwrap();
        let mut transaction = None;
        let result = publish_observed(&tmp.0, &rendered("new"), &mut |point| {
            if point == "committed" {
                let control = tmp.join(CONTROL_NAME);
                let state = read_state(&control)?;
                let path = control.join(format!("tx-{}", state.transaction.unwrap()));
                fs::set_permissions(&path, fs::Permissions::from_mode(0o500))?;
                transaction = Some(path);
            }
            Ok(())
        });
        fs::set_permissions(transaction.unwrap(), fs::Permissions::from_mode(0o700)).unwrap();
        assert!(result.is_err());
        assert!(snapshot_revision(&tmp.0).unwrap().is_some());
        assert_eq!(payloads(&tmp.0), rendered("new").map(Some));
        publish(&tmp.0, &rendered("retry")).unwrap();
        assert_eq!(payloads(&tmp.0), rendered("retry").map(Some));
    }
    #[test]
    fn malformed_state_and_orphan_next_are_preserved() {
        for state in [
            "[]",
            "{\"format\":true}",
            "{\"format\":1,\"format\":1}",
            "not JSON",
        ] {
            let tmp = TempDir::new();
            publish(&tmp.0, &rendered("old")).unwrap();
            let path = tmp.join(CONTROL_NAME).join("state.json");
            fs::write(&path, state).unwrap();
            assert!(publish(&tmp.0, &rendered("new")).is_err());
            assert_eq!(fs::read(path).unwrap(), state.as_bytes());
            assert_eq!(payloads(&tmp.0), rendered("old").map(Some));
        }
        let tmp = TempDir::new();
        publish(&tmp.0, &rendered("old")).unwrap();
        let next = tmp.join(CONTROL_NAME).join("state.next");
        fs::write(&next, "retain").unwrap();
        assert!(publish(&tmp.0, &rendered("new")).is_err());
        assert_eq!(fs::read(next).unwrap(), b"retain");
    }
    #[test]
    fn nonblocking_admission_and_single_link_destinations() {
        let tmp = TempDir::new();
        let control = tmp.join(CONTROL_NAME);
        let admission = Admission::acquire(&control).unwrap();
        assert!(Admission::acquire(&control).is_err());
        drop(admission);
        let admission = Admission::acquire(&control).unwrap();
        drop(admission);
        let path = tmp.join(PACK_NAMES[0]);
        fs::write(&path, "old").unwrap();
        fs::hard_link(&path, tmp.join("alias")).unwrap();
        assert!(
            publish(&tmp.0, &rendered("new"))
                .unwrap_err()
                .to_string()
                .contains("linked file")
        );
        assert_eq!(fs::read(path).unwrap(), b"old");
    }
    #[cfg(windows)]
    #[test]
    fn denied_windows_replacement_rolls_back_slots_installed_before_it() {
        use std::os::windows::fs::OpenOptionsExt;
        let tmp = TempDir::new();
        publish(&tmp.0, &rendered("old")).unwrap();
        let _held = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(tmp.join(PACK_NAMES[1]))
            .unwrap();
        assert!(publish(&tmp.0, &rendered("new")).is_err());
        assert_eq!(payloads(&tmp.0), rendered("old").map(Some));
        assert_settled(&tmp.0);
    }
    #[cfg(unix)]
    #[test]
    fn linked_directories_and_fifo_slots_are_refused() {
        use std::os::unix::fs::symlink;
        let tmp = TempDir::new();
        let other = tmp.join("other");
        fs::create_dir(&other).unwrap();
        symlink(&other, tmp.join(CONTROL_NAME)).unwrap();
        assert!(publish(&tmp.0, &rendered("new")).is_err());
        assert_eq!(fs::read_dir(other).unwrap().count(), 0);
        fs::remove_file(tmp.join(CONTROL_NAME)).unwrap();
        use std::os::unix::ffi::OsStrExt;
        let path = std::ffi::CString::new(tmp.join(PACK_NAMES[0]).as_os_str().as_bytes()).unwrap();
        // SAFETY: path is a NUL-terminated name in this test's owned directory.
        assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
        assert!(publish(&tmp.0, &rendered("new")).is_err());
    }
}
