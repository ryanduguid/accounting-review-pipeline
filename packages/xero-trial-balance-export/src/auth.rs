//! OAuth loopback consent with state checks and bounded accepted connections.
use crate::{
    Failure, Result,
    http::{self, Credentials, Http},
    tokens::{self, TokenSession},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use std::{
    io::{Read, Write},
    net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream},
    time::{Duration, Instant},
};

pub const SCOPES: &str = "offline_access accounting.reports.trialbalance.read";
pub const AUTHORIZE_URL: &str = "https://login.xero.com/identity/connect/authorize";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallbackConfig {
    pub host: String,
    pub port: u16,
    pub path: String,
}

pub fn callback_config(uri: &str) -> Result<CallbackConfig> {
    let normalised: String = uri
        .trim_start_matches(|ch: char| ch as u32 <= 32)
        .chars()
        .filter(|&ch| !matches!(ch, '\t' | '\r' | '\n'))
        .collect();
    let uri = normalised.as_str();
    let scheme = uri
        .split_once(":")
        .map(|(s, _)| s.to_ascii_lowercase())
        .unwrap_or_default();
    if scheme != "http" {
        return Err(Failure::message(
            "XERO_REDIRECT_URI must use http for this local callback server",
        ));
    }
    let authority = uri
        .split_once("://")
        .map(|(_, rest)| rest.split(['/', '?', '#']).next().unwrap_or(""))
        .unwrap_or("");
    if authority.is_empty() {
        return Err(Failure::message("XERO_REDIRECT_URI must include a host"));
    }
    let hostport = authority.rsplit('@').next().unwrap();
    let raw_host = if let Some(bracketed) = hostport.strip_prefix('[') {
        bracketed.split(']').next().unwrap_or("")
    } else {
        hostport.split(':').next().unwrap_or("")
    };
    if raw_host.is_empty() {
        return Err(Failure::message("XERO_REDIRECT_URI must include a host"));
    }
    if !raw_host.eq_ignore_ascii_case("localhost") {
        return Err(Failure::message(
            "XERO_REDIRECT_URI must use localhost for this local callback server",
        ));
    }
    let parsed = reqwest::Url::parse(uri)
        .map_err(|_| Failure::message("XERO_REDIRECT_URI has an invalid port"))?;
    let host = parsed
        .host_str()
        .ok_or_else(|| Failure::message("XERO_REDIRECT_URI must include a host"))?;
    if host != "localhost" {
        return Err(Failure::message(
            "XERO_REDIRECT_URI must use localhost for this local callback server",
        ));
    }
    let port = parsed.port_or_known_default().unwrap_or(80);
    if port == 0 {
        return Err(Failure::message(
            "XERO_REDIRECT_URI port must be between 1 and 65535",
        ));
    }
    let after_authority = uri.split_once("://").map(|(_, rest)| rest).unwrap_or("");
    let after_authority = after_authority.split(['?', '#']).next().unwrap_or("");
    let path_start = after_authority.find('/');
    let path = path_start
        .map(|i| after_authority[i..].split(['?', '#']).next().unwrap_or("/"))
        .unwrap_or("/");
    Ok(CallbackConfig {
        host: host.into(),
        port,
        path: without_parameters(path).into(),
    })
}

fn without_parameters(path: &str) -> &str {
    path.rfind(';')
        .filter(|&index| path.rfind('/').is_none_or(|slash| index > slash))
        .map_or(path, |index| &path[..index])
}

pub fn bind(config: &CallbackConfig) -> Result<TcpListener> {
    let socket = socket2::Socket::new(
        socket2::Domain::IPV4,
        socket2::Type::STREAM,
        Some(socket2::Protocol::TCP),
    )
    .map_err(|e| bind_error(config, e))?;
    socket
        .set_reuse_address(false)
        .map_err(|e| bind_error(config, e))?;
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawSocket;
        use windows_sys::Win32::Networking::WinSock::{
            SO_EXCLUSIVEADDRUSE, SOCKET_ERROR, SOL_SOCKET, WSAGetLastError, setsockopt,
        };
        let enabled = 1i32;
        // SAFETY: socket is live, and enabled points to the four-byte option value.
        if unsafe {
            setsockopt(
                socket.as_raw_socket() as usize,
                SOL_SOCKET,
                SO_EXCLUSIVEADDRUSE,
                (&enabled as *const i32).cast(),
                4,
            )
        } == SOCKET_ERROR
        {
            // SAFETY: reads this thread's error immediately after setsockopt failed.
            return Err(bind_error(
                config,
                std::io::Error::from_raw_os_error(unsafe { WSAGetLastError() }),
            ));
        }
    }
    socket
        .bind(&SocketAddrV4::new(Ipv4Addr::LOCALHOST, config.port).into())
        .map_err(|e| bind_error(config, e))?;
    socket.listen(128).map_err(|e| bind_error(config, e))?;
    let listener: TcpListener = socket.into();
    listener
        .set_nonblocking(true)
        .map_err(|e| bind_error(config, e))?;
    Ok(listener)
}

fn bind_error(config: &CallbackConfig, error: std::io::Error) -> Failure {
    if error.kind() == std::io::ErrorKind::PermissionDenied {
        Failure::message(format!(
            "error: not permitted to listen on {}:{} for the OAuth callback ({error}). Ports below 1024 usually need elevated rights, and a redirect URI with no port uses 80. Point XERO_REDIRECT_URI at a port such as 8400 and add the same URI to the app at developer.xero.com.",
            config.host, config.port
        ))
    } else {
        Failure::message(format!(
            "error: cannot listen on {}:{} for the OAuth callback ({error}). Something else holds that port. Close it, or point XERO_REDIRECT_URI at a free port and add the same URI to the app at developer.xero.com.",
            config.host, config.port
        ))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Callback {
    pub code: Option<String>,
    pub error: Option<String>,
    pub state: Option<String>,
}

fn receive(
    stream: &mut TcpStream,
    config: &CallbackConfig,
    read_budget: Duration,
    connection_budget: Duration,
) -> Option<Callback> {
    let deadline = Instant::now() + connection_budget;
    let mut bytes = Vec::new();
    let mut byte = [0u8; 1];
    let (mut line_length, mut lines) = (0, 0);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return None;
        }
        stream
            .set_read_timeout(Some(read_budget.min(remaining)))
            .ok()?;
        match stream.read(&mut byte) {
            Ok(1) => {
                bytes.push(byte[0]);
                line_length += 1;
            }
            _ => return None,
        }
        if line_length > 65536 {
            return None;
        }
        if byte[0] == b'\n' {
            lines += 1;
            line_length = 0;
            if lines == 1 {
                let line: String = bytes.iter().map(|&b| char::from(b)).collect();
                if line.split_whitespace().count() == 2 {
                    break;
                }
            }
            if lines > 101 {
                return None;
            }
        }
        if bytes.ends_with(b"\r\n\r\n") || bytes.ends_with(b"\n\n") {
            break;
        }
    }
    let line = bytes.split(|&b| b == b'\n').next()?;
    let line: String = line.iter().map(|&b| char::from(b)).collect();
    let parts: Vec<_> = line.split_whitespace().collect();
    if !matches!(parts.len(), 2 | 3) {
        return None;
    }
    if parts.len() == 3 {
        let version = parts[2].strip_prefix("HTTP/")?;
        let (major, minor) = version.split_once('.')?;
        if [major, minor]
            .iter()
            .any(|v| v.is_empty() || v.len() > 10 || !v.bytes().all(|b| b.is_ascii_digit()))
            || major.parse::<u64>().ok()? >= 2
        {
            return None;
        }
    }
    let method = parts[0];
    let target = parts[1];
    if method != "GET" {
        respond(stream, 501, "");
        return None;
    }
    let target = target.split('#').next().unwrap();
    let target = if target.starts_with("//") {
        format!("/{}", target.trim_start_matches('/'))
    } else {
        target.into()
    };
    let target = target.as_str();
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let path = without_parameters(path);
    if path != config.path {
        respond(stream, 404, "");
        return None;
    }
    let parsed = reqwest::Url::parse(&format!("http://localhost/?{query}")).ok()?;
    let mut callback = Callback::default();
    for (key, value) in parsed.query_pairs().filter(|(_, value)| !value.is_empty()) {
        let slot = match key.as_ref() {
            "code" => &mut callback.code,
            "error" => &mut callback.error,
            "state" => &mut callback.state,
            _ => continue,
        };
        if slot.is_none() {
            *slot = Some(value.into_owned());
        }
    }
    let body = if callback.error.is_some() {
        "<h3>Authorisation was not completed. You can close this tab.</h3>"
    } else {
        "<h3>Authorised. You can close this tab.</h3>"
    };
    respond(stream, 200, body);
    Some(callback)
}

fn respond(stream: &mut TcpStream, status: u16, body: &str) {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        _ => "Not Implemented",
    };
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    let _ = write!(
        stream,
        "HTTP/1.0 {status} {reason}\r\nContent-Type: text/html\r\n\r\n{body}"
    );
}

pub fn wait_for_callback(
    listener: &TcpListener,
    config: &CallbackConfig,
    budget: Duration,
) -> Result<Callback> {
    wait_with_budgets(
        listener,
        config,
        budget,
        Duration::from_secs(2),
        Duration::from_secs(10),
    )
}

pub fn wait_with_budgets(
    listener: &TcpListener,
    config: &CallbackConfig,
    budget: Duration,
    read_budget: Duration,
    connection_budget: Duration,
) -> Result<Callback> {
    let deadline = Instant::now() + budget;
    loop {
        if Instant::now() >= deadline {
            return Err(Failure::message(format!(
                "error: no Xero callback arrived within {} seconds. Nothing was saved. Run again and complete the consent in the browser.",
                budget.as_secs()
            )));
        }
        match listener.accept() {
            Ok((mut stream, _)) => {
                if let Some(callback) = receive(&mut stream, config, read_budget, connection_budget)
                    && (callback.code.is_some() || callback.error.is_some())
                {
                    return Ok(callback);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(10))
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => {
                return Err(Failure::message(format!(
                    "error: OAuth callback listener failed ({e}); nothing was saved."
                )));
            }
        }
    }
}

pub fn verify_callback<'a>(callback: &'a Callback, expected_state: &str) -> Result<&'a str> {
    let state = callback.state.as_deref().unwrap_or("");
    let same = state.len() == expected_state.len()
        && state
            .bytes()
            .zip(expected_state.bytes())
            .fold(0u8, |bits, (a, b)| bits | (a ^ b))
            == 0;
    if !same {
        return Err(Failure::message(
            "State mismatch - possible CSRF or stale callback. Run again.",
        ));
    }
    if let Some(error) = &callback.error {
        let message = if !error.is_empty()
            && error.len() <= 64
            && error
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            format!("Xero returned '{error}' - consent was denied or cancelled. Run again.")
        } else {
            "Xero returned an error code this script could not read - consent was denied or cancelled. Run again.".into()
        };
        return Err(Failure::message(message));
    }
    callback.code.as_deref().ok_or_else(|| {
        Failure::message(
            "error: OAuth callback contained no authorisation code. Nothing was saved.",
        )
    })
}

pub fn authorise(
    http: &impl Http,
    session: &TokenSession,
    credentials: &Credentials,
    redirect: &str,
) -> Result<()> {
    let config = callback_config(redirect)
        .map_err(|e| Failure::message(format!("Invalid XERO_REDIRECT_URI: {}", e.stderr.trim())))?;
    let mut state = [0u8; 16];
    getrandom::fill(&mut state)
        .map_err(|_| Failure::message("error: secure random bytes unavailable"))?;
    let state = URL_SAFE_NO_PAD.encode(state);
    let mut url = reqwest::Url::parse(AUTHORIZE_URL).expect("constant OAuth URL");
    url.query_pairs_mut().extend_pairs([
        ("response_type", "code"),
        ("client_id", &credentials.id),
        ("redirect_uri", redirect),
        ("scope", SCOPES),
        ("state", &state),
    ]);
    let listener = bind(&config)?;
    if open_browser(url.as_str()) {
        crate::outln!(
            "Opening browser for Xero consent...\nIf the browser did not open, paste this URL into one:"
        );
    } else {
        crate::outln!(
            "Could not open a browser (SSH or headless session?). Paste this URL into one:"
        );
    }
    crate::outln!("  {url}");
    let callback = wait_for_callback(&listener, &config, Duration::from_secs(300));
    drop(listener);
    let callback = callback?;
    let code = verify_callback(&callback, &state)?;
    let response = http::exchange(http, credentials, code, redirect)?;
    let (response, warning) = tokens::validate_rotated(&response)?;
    crate::err!("{warning}");
    session.save(&response)?;
    crate::outln!("Tokens saved to token.json. Next: export-tb --date 2026-06-30");
    Ok(())
}

#[cfg(windows)]
fn open_browser(url: &str) -> bool {
    use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};
    let url: Vec<u16> = url.encode_utf16().chain(std::iter::once(0)).collect();
    let verb: Vec<u16> = "open".encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: both strings are NUL-terminated and remain alive through the call.
    unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            url.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        ) as usize
            > 32
    }
}

#[cfg(not(windows))]
fn open_browser(url: &str) -> bool {
    let command = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    std::process::Command::new(command)
        .arg(url)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn configuration_and_state_refuse_wrong_boundaries() {
        assert_eq!(
            callback_config("http://localhost/callback").unwrap().port,
            80
        );
        for uri in [
            "https://localhost/callback",
            "http://example.com/callback",
            "http://localhost:0/callback",
            "http://localhost:65536/callback",
        ] {
            assert!(callback_config(uri).is_err(), "{uri}");
        }
        let callback = Callback {
            code: Some("synthetic-code".into()),
            error: Some("access_denied".into()),
            state: Some("wrong".into()),
        };
        assert!(
            verify_callback(&callback, "right")
                .unwrap_err()
                .stderr
                .starts_with("State mismatch")
        );
    }
    #[test]
    fn bounded_silent_connection_and_exclusive_port() {
        let config = CallbackConfig {
            host: "localhost".into(),
            port: 0,
            path: "/callback".into(),
        };
        let listener = bind(&config).unwrap();
        let address = listener.local_addr().unwrap();
        let config = CallbackConfig {
            port: address.port(),
            ..config
        };
        assert!(bind(&config).is_err());
        let _silent = TcpStream::connect(address).unwrap();
        let start = Instant::now();
        assert!(
            wait_with_budgets(
                &listener,
                &config,
                Duration::from_millis(50),
                Duration::from_millis(60),
                Duration::from_millis(100)
            )
            .is_err()
        );
        assert!(start.elapsed() < Duration::from_secs(1));
    }
    fn receive_request(request: &[u8]) -> Option<Callback> {
        let config = CallbackConfig {
            host: "localhost".into(),
            port: 0,
            path: "/callback".into(),
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        client.write_all(request).unwrap();
        let (mut server, _) = listener.accept().unwrap();
        receive(
            &mut server,
            &config,
            Duration::from_millis(50),
            Duration::from_millis(100),
        )
    }
    #[test]
    fn callback_query_uses_first_nonblank_value_and_ignores_unrelated_requests() {
        let callback=receive_request(b"GET /callback?code=&code=synthetic%2Bcode&code=other&state=synthetic+state HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
        assert_eq!(callback.code.as_deref(), Some("synthetic+code"));
        assert_eq!(callback.state.as_deref(), Some("synthetic state"));
        assert!(receive_request(b"GET /favicon.ico?code=synthetic HTTP/1.1\r\n\r\n").is_none());
        assert!(receive_request(b"POST /callback?code=synthetic HTTP/1.1\r\n\r\n").is_none());
        assert!(receive_request(b"GET /callback?code=synthetic HTTP/2.0\r\n\r\n").is_none());
        assert!(receive_request(b"GET /callback?code=synthetic bad-version\r\n\r\n").is_none());
        assert!(receive_request(b"GET /callback?code=synthetic\r\n").is_some());
    }
    #[test]
    fn dribbling_peer_cannot_extend_the_absolute_connection_budget() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let drip = std::thread::spawn(move || {
            for byte in b"GET /callback?code=synthetic HTTP/1.1\r\n\r\n" {
                if client.write_all(&[*byte]).is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        });
        let (mut server, _) = listener.accept().unwrap();
        let start = Instant::now();
        assert!(
            receive(
                &mut server,
                &CallbackConfig {
                    host: "localhost".into(),
                    port: 0,
                    path: "/callback".into()
                },
                Duration::from_millis(60),
                Duration::from_millis(100)
            )
            .is_none()
        );
        assert!(start.elapsed() < Duration::from_millis(400));
        drop(server);
        drip.join().unwrap();
    }
}
