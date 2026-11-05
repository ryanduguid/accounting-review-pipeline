// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

//! The README says no network client exists in this package. This holds the
//! shipped dependency tree and `src` to that. It reads names, so it cannot see a
//! socket reached through a foreign function; `git` is the one program evatt runs.
use std::path::{Path, PathBuf};
use std::process::Command;

/// HTTP, TLS, DNS, socket and mail clients, and the async runtimes that carry them.
const NETWORK_CRATES: [&str; 37] = [
    "actix-web",
    "async-net",
    "async-std",
    "attohttpc",
    "awc",
    "axum",
    "curl",
    "curl-sys",
    "git2",
    "h2",
    "h3",
    "hickory-resolver",
    "http-client",
    "hyper",
    "hyper-rustls",
    "hyper-tls",
    "hyper-util",
    "isahc",
    "lettre",
    "libgit2-sys",
    "mio",
    "native-tls",
    "openssl",
    "openssl-sys",
    "quinn",
    "reqwest",
    "rustls",
    "smol",
    "socket2",
    "ssh2",
    "surf",
    "tokio",
    "tokio-tungstenite",
    "tonic",
    "trust-dns-resolver",
    "tungstenite",
    "ureq",
];

/// Spellings of the standard library's sockets. The type names catch a grouped
/// import, such as `use std::{io, net::TcpStream}`, that hides `std::net`.
const SOCKET_NAMES: [&str; 6] = [
    "std::net",
    "core::net",
    "os::unix::net",
    "TcpListener",
    "TcpStream",
    "UdpSocket",
];

fn network_crates(tree: &str) -> Vec<&str> {
    let mut found: Vec<_> = tree
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .filter(|name| NETWORK_CRATES.contains(name))
        .collect();
    found.sort_unstable();
    found.dedup();
    found
}

fn socket_names(source: &str) -> Vec<&'static str> {
    SOCKET_NAMES.into_iter().filter(|name| source.contains(name)).collect()
}

fn rust_files(directory: &Path, found: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, found);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            found.push(path);
        }
    }
}

/// Every normal dependency on every target, with every feature on.
fn dependency_tree() -> String {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let output = Command::new(cargo)
        .args(["tree", "--locked", "-e", "normal", "--prefix", "none"])
        .args(["--target", "all", "--all-features"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn the_normal_dependency_tree_lists_no_network_crate() {
    let tree = dependency_tree();
    // An empty or misread tree would otherwise pass.
    assert!(tree.contains("fancy-regex v"), "{tree}");
    assert!(tree.contains("windows-sys v"), "{tree}");
    assert_eq!(network_crates(&tree), Vec::<&str>::new());
}

#[test]
fn src_names_no_socket_api() {
    let mut files = Vec::new();
    rust_files(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut files);
    assert!(files.len() > 20, "{files:?}");
    for file in files {
        let source = std::fs::read_to_string(&file).unwrap();
        assert_eq!(socket_names(&source), Vec::<&str>::new(), "{file:?}");
    }
}

#[test]
fn the_checks_catch_a_planted_crate_and_a_planted_socket() {
    let tree = "evatt v1.0.0 (/x)\nfancy-regex v0.20.0\nhyper v1.0.0\nhyper v1.0.0 (*)\nmy-hyper v1.0.0\n";
    assert_eq!(network_crates(tree), ["hyper"]);
    assert_eq!(socket_names("use std::net::TcpStream;"), ["std::net", "TcpStream"]);
    assert_eq!(socket_names("use std::{io, net::UdpSocket};"), ["UdpSocket"]);
    assert!(socket_names("use std::{io, fs};").is_empty());
}
