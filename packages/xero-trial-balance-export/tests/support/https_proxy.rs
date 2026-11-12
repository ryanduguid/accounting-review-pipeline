// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

//! An owned CONNECT endpoint: decrypt fabricated requests without forwarding them.

use rustls::{ServerConfig, ServerConnection, StreamOwned, pki_types::PrivatePkcs8KeyDer};
use std::{
    collections::BTreeMap,
    io::{self, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[derive(Debug)]
pub struct Request {
    pub authority: String,
    pub method: String,
    pub target: String,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

pub struct Proxy {
    pub address: SocketAddr,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<io::Result<usize>>>,
    expected: usize,
}

impl Proxy {
    pub fn new(expected: usize, mut reply: impl FnMut(Request) -> String + Send + 'static) -> Self {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = Arc::clone(&stop);
        let worker = thread::spawn(move || {
            let config = configuration();
            let deadline = Instant::now() + Duration::from_secs(40);
            let mut count = 0;
            while count < expected && !stopping.load(Ordering::Relaxed) {
                let Some(stream) = accept(&listener, deadline)? else {
                    continue;
                };
                exchange(stream, &config, &mut reply)?;
                count += 1;
            }
            Ok(count)
        });
        Self {
            address,
            stop,
            worker: Some(worker),
            expected,
        }
    }

    pub fn finish(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let count = self
            .worker
            .take()
            .unwrap()
            .join()
            .expect("HTTPS fixture thread panicked")
            .unwrap();
        assert_eq!(
            count, self.expected,
            "every expected native request must reach the fixture"
        );
    }
}

impl Drop for Proxy {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn configuration() -> Arc<ServerConfig> {
    let certificate = include_bytes!("../fixtures/native-https/server.der")
        .to_vec()
        .into();
    let key = PrivatePkcs8KeyDer::from(
        include_bytes!("../fixtures/native-https/server-key.der").to_vec(),
    );
    Arc::new(
        ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![certificate], key.into())
            .unwrap(),
    )
}

fn accept(listener: &TcpListener, deadline: Instant) -> io::Result<Option<TcpStream>> {
    if Instant::now() >= deadline {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "HTTPS fixture deadline",
        ));
    }
    match listener.accept() {
        Ok((stream, _)) => Ok(Some(stream)),
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
            thread::sleep(Duration::from_millis(5));
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

fn exchange(
    mut socket: TcpStream,
    config: &Arc<ServerConfig>,
    reply: &mut impl FnMut(Request) -> String,
) -> io::Result<()> {
    socket.set_nonblocking(false)?;
    socket.set_read_timeout(Some(Duration::from_secs(5)))?;
    socket.set_write_timeout(Some(Duration::from_secs(5)))?;
    let connect = read_request(&mut socket)?;
    assert_eq!(connect.method, "CONNECT");
    assert!(
        matches!(
            connect.target.as_str(),
            "api.xero.com:443" | "identity.xero.com:443"
        ),
        "unexpected CONNECT: {connect:?}"
    );
    socket.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")?;
    let connection = ServerConnection::new(Arc::clone(config)).unwrap();
    let mut tls = StreamOwned::new(connection, socket);
    let mut request = read_request(&mut tls)?;
    request.authority = connect.target;
    let body = reply(request);
    write!(
        tls,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    tls.flush()?;
    tls.conn.send_close_notify();
    tls.flush()
}

fn read_request(stream: &mut impl Read) -> io::Result<Request> {
    let mut bytes = Vec::new();
    while !bytes.ends_with(b"\r\n\r\n") {
        if bytes.len() >= 16_384 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "HTTP fixture header limit",
            ));
        }
        let mut byte = [0];
        stream.read_exact(&mut byte)?;
        bytes.push(byte[0]);
    }
    let text =
        std::str::from_utf8(&bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let mut lines = text.split("\r\n");
    let parts: Vec<_> = lines.next().unwrap().split_whitespace().collect();
    assert_eq!(parts.len(), 3);
    assert_eq!(parts[2], "HTTP/1.1");
    let mut headers = BTreeMap::<String, String>::new();
    for line in lines.filter(|s| !s.is_empty()) {
        let (name, value) = line.split_once(':').expect("HTTP fixture header");
        assert!(
            headers
                .insert(name.to_ascii_lowercase(), value.trim().into())
                .is_none(),
            "duplicate HTTP fixture header"
        );
    }
    let length = headers
        .get("content-length")
        .map_or(0, |v| v.parse::<usize>().unwrap());
    assert!(length <= 4096, "HTTP fixture body limit");
    let mut body = vec![0; length];
    stream.read_exact(&mut body)?;
    Ok(Request {
        authority: String::new(),
        method: parts[0].into(),
        target: parts[1].into(),
        headers,
        body,
    })
}
