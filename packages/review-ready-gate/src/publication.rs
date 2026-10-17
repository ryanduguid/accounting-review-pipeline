//! Cooperating writers publish three fixed slots using persisted transaction evidence.
//! State replacement gives atomic visibility, without a power-loss durability claim.
use crate::{
    pyjson::{self, Json},
    pypath,
    report::PACK_NAMES,
};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{self, Seek, SeekFrom, Write},
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

pub struct ManagedPublication {
    previous: Option<usize>,
}
impl ManagedPublication {
    pub fn new() -> Self {
        CANCELLED.store(false, Ordering::Relaxed);
        #[cfg(unix)]
        let previous = {
            let mut action = std::mem::MaybeUninit::<libc::sigaction>::zeroed();
            // SAFETY: sigaction fills the supplied structure; no handler is changed here.
            if unsafe { libc::sigaction(libc::SIGTERM, std::ptr::null(), action.as_mut_ptr()) } == 0
            {
                // SAFETY: the successful call initialised action.
                let action = unsafe { action.assume_init() };
                if action.sa_sigaction == libc::SIG_DFL {
                    // SAFETY: the handler only stores an atomic flag and has static lifetime.
                    let old = unsafe {
                        libc::signal(libc::SIGTERM, request_cancel as *const () as usize)
                    };
                    (old != libc::SIG_ERR).then_some(old)
                } else {
                    None
                }
            } else {
                None
            }
        };
        #[cfg(windows)]
        let previous = {
            // Windows process termination remains abrupt; C signal delivery is managed.
            // SAFETY: the handler only stores an atomic flag and has static lifetime.
            let old = unsafe { win_signal(15, request_cancel as *const () as usize) };
            if old == 0 {
                Some(old)
            } else {
                if old != usize::MAX {
                    unsafe {
                        win_signal(15, old);
                    }
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
        use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
        use windows_sys::Win32::Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
        };
        let file = OpenOptions::new().access_mode(0).open(_path)?;
        let mut info = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::zeroed();
        // SAFETY: file owns the handle and info is correctly sized writable storage.
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), info.as_mut_ptr()) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the successful call initialised info.
        Ok(unsafe { info.assume_init() }.nNumberOfLinks == 1)
    }
}
fn regular(path: &Path, absent: bool) -> io::Result<bool> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if absent && e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e),
    };
    if !single_link(path, &metadata)? {
        return Err(error(&format!(
            "publication refuses a non-regular or linked file: {}",
            pypath::decode_os(path.file_name().unwrap_or_default())
        )));
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
fn read_state(control: &Path) -> io::Result<State> {
    directory(control)?;
    regular(&control.join("state.json"), false)?;
    let bytes = fs::read(control.join("state.json"))?;
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
    match fs::symlink_metadata(&transaction) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Err(error(
                "referenced publication transaction is missing; retain control for review",
            ));
        }
        Err(e) => return Err(e),
        Ok(_) => {}
    }
    directory(&transaction)?;
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
            previous[i] = Some(crate::sha256(&fs::read(backup)?));
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
            if &crate::sha256(&fs::read(&backup)?) != hash {
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
                held = Some(
                    OpenOptions::new().read(true).share_mode(0).open(
                        control
                            .join(format!("tx-{}", state.transaction.unwrap()))
                            .join("backup-0"),
                    )?,
                );
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
    fn corrupt_backup_keeps_unresolved_recovery_evidence() {
        let tmp = TempDir::new();
        publish(&tmp.0, &rendered("old")).unwrap();
        let result = publish_observed(&tmp.0, &rendered("new"), &mut |point| {
            if point == "install-0" {
                let control = tmp.join(CONTROL_NAME);
                let state = read_state(&control)?;
                fs::write(
                    control
                        .join(format!("tx-{}", state.transaction.unwrap()))
                        .join("backup-0"),
                    "corrupt",
                )?;
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
