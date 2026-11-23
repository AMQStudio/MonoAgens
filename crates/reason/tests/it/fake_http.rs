//! Programmable std-only fake HTTP server on `127.0.0.1:0` for provider tests.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct Captured {
    pub method: String,
    /// Path plus query string.
    pub target: String,
    /// Lower-cased header names.
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Captured {
    pub fn header(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| v.as_str())
    }

    pub fn path(&self) -> &str {
        self.target.split('?').next().unwrap_or("")
    }
}

pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
    pub delay_ms: u64,
}

impl Reply {
    pub fn json(status: u16, body: &str) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: body.to_owned(),
            delay_ms: 0,
        }
    }
}

pub struct FakeServer {
    addr: SocketAddr,
    pub requests: Arc<Mutex<Vec<Captured>>>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl FakeServer {
    pub fn start<F>(handler: F) -> Self
    where
        F: Fn(&Captured) -> Reply + Send + Sync + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        listener.set_nonblocking(true).expect("nonblocking");
        let stop = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let handler = Arc::new(handler);
        let (stop_bg, req_bg) = (Arc::clone(&stop), Arc::clone(&requests));
        let handle = std::thread::spawn(move || {
            while !stop_bg.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let (h, r) = (Arc::clone(&handler), Arc::clone(&req_bg));
                        std::thread::spawn(move || {
                            let _ = handle_conn(stream, &*h, &r);
                        });
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(5)),
                }
            }
        });
        Self {
            addr,
            requests,
            stop,
            handle: Some(handle),
        }
    }

    /// `http://127.0.0.1:PORT`
    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub fn captured(&self) -> Vec<Captured> {
        self.requests.lock().expect("lock").clone()
    }
}

impl Drop for FakeServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// A loopback URL nothing listens on (connection refused).
pub fn closed_port_url() -> String {
    let l = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = l.local_addr().expect("addr");
    drop(l);
    format!("http://{addr}")
}

fn handle_conn(
    mut stream: TcpStream,
    handler: &(dyn Fn(&Captured) -> Reply + Send + Sync),
    log: &Mutex<Vec<Captured>>,
) -> std::io::Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    let mut headers = Vec::new();
    let mut content_len = 0usize;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line)?;
        let line = line.trim_end().to_owned();
        if line.is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            let (k, v) = (k.trim().to_ascii_lowercase(), v.trim().to_owned());
            if k == "content-length" {
                content_len = v.parse().unwrap_or(0);
            }
            headers.push((k, v));
        }
    }
    let mut body = vec![0u8; content_len];
    reader.read_exact(&mut body)?;
    let mut parts = request_line.split_whitespace();
    let cap = Captured {
        method: parts.next().unwrap_or("").to_owned(),
        target: parts.next().unwrap_or("").to_owned(),
        headers,
        body: String::from_utf8_lossy(&body).into_owned(),
    };
    log.lock().expect("lock").push(cap.clone());
    let reply = handler(&cap);
    if reply.delay_ms > 0 {
        std::thread::sleep(Duration::from_millis(reply.delay_ms));
    }
    let mut head = format!(
        "HTTP/1.1 {} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
        reply.status,
        reply.body.len()
    );
    for (k, v) in &reply.headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    stream.write_all(reply.body.as_bytes())?;
    stream.flush()
}
