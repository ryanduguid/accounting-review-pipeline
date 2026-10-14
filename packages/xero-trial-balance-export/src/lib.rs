pub mod amount;
pub mod auth;
pub mod cache_json;
pub mod cli;
pub mod config;
pub mod dpapi;
pub mod export;
pub mod http;
mod http_date;
#[cfg(any(test, feature = "tools"))]
pub mod offline;
pub mod paths;
pub mod publication;
#[cfg(any(test, feature = "tools"))]
pub mod pycsv;
pub mod pyjson;
pub mod pypath;
pub mod pytext;
pub mod report;
pub mod stdio;
pub mod tables;
pub mod termsize;
pub mod textwrap;
pub mod tokens;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Failure {
    pub fn message(message: impl Into<String>) -> Self {
        Self {
            code: 1,
            stdout: String::new(),
            stderr: format!("{}\n", message.into()),
        }
    }
}

pub type Result<T> = std::result::Result<T, Failure>;

#[cfg(any(test, feature = "probe"))]
pub mod compat_probe;

#[cfg(test)]
mod testing {
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicUsize, Ordering},
    };
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    pub struct Scratch(pub PathBuf);
    impl Scratch {
        pub fn new() -> Self {
            loop {
                let path = std::env::temp_dir().join(format!(
                    "xero-rust-test-{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                ));
                match fs::create_dir(&path) {
                    Ok(()) => return Self(path),
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(e) => panic!("scratch directory: {e}"),
                }
            }
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).expect("remove owned test directory");
        }
    }
}
