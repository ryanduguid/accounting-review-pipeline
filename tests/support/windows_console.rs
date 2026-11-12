// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

//! Observe the actual Windows console without capturing the CLI's streams.

use std::{
    ffi::c_void,
    fs, io,
    os::windows::process::CommandExt,
    path::PathBuf,
    process::{Child, Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const WIDTH: usize = 80;
const NO_WINDOW: u32 = 0x0800_0000;
const LAUNCH: &str = r#"
$ErrorActionPreference = 'Stop'
$owned = $null
try {
    $owned = Start-Process -FilePath $env:ARP_CONSOLE_TEST_EXE -ArgumentList @('--exact', $env:ARP_CONSOLE_TEST_NAME, '--nocapture') -WindowStyle Hidden -PassThru
    if (-not $owned.WaitForExit(20000)) { throw 'Owned console test exceeded 20 seconds' }
    exit $owned.ExitCode
} finally {
    if ($null -ne $owned -and -not $owned.HasExited) {
        $owned.Kill($true)
        if (-not $owned.WaitForExit(3000)) { throw 'Owned console test did not stop' }
    }
}
"#;

/// Re-enter this one test in an owned, hidden console, then run the real CLI.
pub fn check(name: &str, binary: &str, help: &str, usage: &str, error_code: i32) {
    if std::env::var("ARP_CONSOLE_TEST_NAME").as_deref() == Ok(name) {
        observe(binary, help, usage, error_code);
        return;
    }
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let directory = std::env::temp_dir().join(format!("arp-console-{}-{nonce}", std::process::id()));
    fs::create_dir(&directory).unwrap();
    let mut launcher = crate::child_profile::command("pwsh");
    launcher
        .args(["-NoProfile", "-NonInteractive", "-Command", LAUNCH])
        .env("ARP_CONSOLE_TEST_EXE", std::env::current_exe().unwrap())
        .env("ARP_CONSOLE_TEST_NAME", name)
        .env("ARP_CONSOLE_RECEIPTS", &directory)
        .creation_flags(NO_WINDOW)
        .stdout(Stdio::null());
    let status = wait(&mut launcher, Duration::from_secs(30));
    assert!(
        status.success(),
        "Console test failed; receipts: {}",
        directory.display()
    );
    assert_eq!(fs::read(directory.join("passed")).unwrap(), b"passed\n");
    for file in ["help.txt", "usage.txt", "passed"] {
        fs::remove_file(directory.join(file)).unwrap();
    }
    fs::remove_dir(directory).unwrap();
}

fn wait(command: &mut Command, timeout: Duration) -> ExitStatus {
    let mut child = command.spawn().expect("start owned console process");
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        if Instant::now() >= deadline {
            kill_tree(&mut child);
            panic!("owned console process exceeded {timeout:?}");
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn kill_tree(child: &mut Child) {
    if let Ok(mut killer) = Command::new("taskkill")
        .args(["/PID", &child.id().to_string(), "/T", "/F"])
        .creation_flags(NO_WINDOW)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        let deadline = Instant::now() + Duration::from_secs(3);
        while killer.try_wait().unwrap().is_none() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        stop(&mut killer);
    }
    stop(child);
}

fn stop(child: &mut Child) {
    let _ = child.kill();
    let deadline = Instant::now() + Duration::from_secs(3);
    while child.try_wait().unwrap().is_none() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    assert!(
        child.try_wait().unwrap().is_some(),
        "owned console process did not stop"
    );
}

fn observe(binary: &str, help: &str, usage: &str, error_code: i32) {
    let directory = PathBuf::from(std::env::var_os("ARP_CONSOLE_RECEIPTS").unwrap());
    let height = help.lines().count().max(usage.lines().count()).max(24) + 2;
    let console = Console::new(height);
    for (label, args, expected, code) in [("help", vec!["--help"], help, 0), ("usage", vec![], usage, error_code)] {
        console.clear();
        let mut command = crate::child_profile::command(binary);
        command.args(args).stdin(Stdio::null());
        for name in ["PYTHON_COLORS", "NO_COLOR", "FORCE_COLOR", "TERM", "COLUMNS", "LINES"] {
            command.env_remove(name);
        }
        let status = wait(&mut command, Duration::from_secs(5));
        let (screen, attributes) = console.read();
        fs::write(directory.join(format!("{label}.txt")), &screen).unwrap();
        assert_eq!(status.code(), Some(code), "{label}");
        assert_eq!(screen, physical_rows(expected), "{label}");
        assert!(
            attributes.iter().any(|&value| value != 7),
            "{label}: colour roles missing"
        );
    }
    fs::write(directory.join("passed"), b"passed\n").unwrap();
}

fn physical_rows(text: &str) -> String {
    assert!(text.is_ascii(), "console fixture must have unambiguous cell widths");
    let mut rows = Vec::new();
    for line in text.lines() {
        if line.is_empty() {
            rows.push(String::new());
        } else {
            rows.extend(
                line.as_bytes()
                    .chunks(WIDTH)
                    .map(|chunk| String::from_utf8(chunk.to_vec()).unwrap()),
            );
        }
    }
    rows.join("\n").trim_end().to_owned() + "\n"
}

type Handle = *mut c_void;

#[derive(Clone, Copy, Default)]
#[repr(C)]
struct Coord {
    x: i16,
    y: i16,
}

#[derive(Default)]
#[repr(C)]
struct Rect {
    left: i16,
    top: i16,
    right: i16,
    bottom: i16,
}

#[derive(Default)]
#[repr(C)]
struct BufferInfo {
    size: Coord,
    cursor: Coord,
    attributes: u16,
    window: Rect,
    maximum: Coord,
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetStdHandle(which: u32) -> Handle;
    fn GetConsoleMode(handle: Handle, mode: *mut u32) -> i32;
    fn SetConsoleMode(handle: Handle, mode: u32) -> i32;
    fn GetConsoleScreenBufferInfo(handle: Handle, info: *mut BufferInfo) -> i32;
    fn SetConsoleScreenBufferSize(handle: Handle, size: Coord) -> i32;
    fn SetConsoleWindowInfo(handle: Handle, absolute: i32, window: *const Rect) -> i32;
    fn SetConsoleCursorPosition(handle: Handle, position: Coord) -> i32;
    fn FillConsoleOutputCharacterW(
        handle: Handle,
        character: u16,
        count: u32,
        position: Coord,
        written: *mut u32,
    ) -> i32;
    fn FillConsoleOutputAttribute(
        handle: Handle,
        attributes: u16,
        count: u32,
        position: Coord,
        written: *mut u32,
    ) -> i32;
    fn ReadConsoleOutputCharacterW(handle: Handle, text: *mut u16, count: u32, position: Coord, read: *mut u32) -> i32;
    fn ReadConsoleOutputAttribute(
        handle: Handle,
        attributes: *mut u16,
        count: u32,
        position: Coord,
        read: *mut u32,
    ) -> i32;
}

struct Console {
    handle: Handle,
    cells: u32,
}

fn succeeded(result: i32) {
    assert_ne!(result, 0, "Windows console: {}", io::Error::last_os_error());
}

impl Console {
    fn new(height: usize) -> Self {
        let mut info = BufferInfo::default();
        let height = i16::try_from(height).unwrap();
        // SAFETY: fixed Win32 handles and valid, correctly laid out buffers.
        unsafe {
            let handle = GetStdHandle(-11i32 as u32);
            succeeded(GetConsoleScreenBufferInfo(handle, &mut info));
            let larger = Coord {
                x: info.size.x.max(80),
                y: info.size.y.max(height),
            };
            succeeded(SetConsoleScreenBufferSize(handle, larger));
            let window = Rect {
                left: 0,
                top: 0,
                right: 79,
                bottom: 23,
            };
            succeeded(SetConsoleWindowInfo(handle, 1, &window));
            succeeded(SetConsoleScreenBufferSize(handle, Coord { x: 80, y: height }));
            for stream in [-11i32, -12] {
                let mut mode = 0;
                let output = GetStdHandle(stream as u32);
                succeeded(GetConsoleMode(output, &mut mode));
                succeeded(SetConsoleMode(output, mode | 4));
            }
            Self {
                handle,
                cells: WIDTH as u32 * height as u32,
            }
        }
    }

    fn clear(&self) {
        let mut written = 0;
        // SAFETY: the owned console contains exactly `cells` valid cells.
        unsafe {
            succeeded(FillConsoleOutputCharacterW(
                self.handle,
                b' ' as u16,
                self.cells,
                Coord::default(),
                &mut written,
            ));
            assert_eq!(written, self.cells);
            succeeded(FillConsoleOutputAttribute(
                self.handle,
                7,
                self.cells,
                Coord::default(),
                &mut written,
            ));
            assert_eq!(written, self.cells);
            succeeded(SetConsoleCursorPosition(self.handle, Coord::default()));
        }
    }

    fn read(&self) -> (String, Vec<u16>) {
        let mut text = vec![0u16; self.cells as usize];
        let mut attributes = vec![0u16; self.cells as usize];
        let mut read = 0;
        // SAFETY: both output buffers contain `cells` writable elements.
        unsafe {
            succeeded(ReadConsoleOutputCharacterW(
                self.handle,
                text.as_mut_ptr(),
                self.cells,
                Coord::default(),
                &mut read,
            ));
            assert_eq!(read, self.cells);
            succeeded(ReadConsoleOutputAttribute(
                self.handle,
                attributes.as_mut_ptr(),
                self.cells,
                Coord::default(),
                &mut read,
            ));
            assert_eq!(read, self.cells);
        }
        let rows = text
            .chunks(WIDTH)
            .map(|row| String::from_utf16(row).unwrap().trim_end().to_owned())
            .collect::<Vec<_>>();
        (rows.join("\n").trim_end().to_owned() + "\n", attributes)
    }
}
