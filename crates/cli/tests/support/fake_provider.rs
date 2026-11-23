//! Tiny fake OpenAI-compatible provider server on `127.0.0.1:0` (std only).
//!
//! Serves `GET /models` and `POST /chat/completions`; anything else is 404.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

pub struct FakeProvider {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl FakeProvider {
    /// Starts the server with the given model ids and fixed assistant reply text.
    pub fn start(models: &[&str], reply: &str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        listener.set_nonblocking(true).expect("nonblocking");
        let stop = Arc::new(AtomicBool::new(false));
        let stop_bg = Arc::clone(&stop);
        let models_body = serde_json::json!({
            "object": "list",
            "data": models.iter().map(|m| serde_json::json!({"id": m, "object": "model"})).collect::<Vec<_>>(),
        })
        .to_string();
        let chat_body = serde_json::json!({
            "id": "chatcmpl-fake",
            "object": "chat.completion",
            "choices": [{"index": 0, "finish_reason": "stop",
                "message": {"role": "assistant", "content": reply}}],
            "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2},
        })
        .to_string();
        let handle = std::thread::spawn(move || {
            while !stop_bg.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let _ = handle_conn(stream, &models_body, &chat_body);
                    }
                    Err(_) => std::thread::sleep(std::time::Duration::from_millis(10)),
                }
            }
        });
        Self {
            addr,
            stop,
            handle: Some(handle),
        }
    }

    /// Base URL such as `http://127.0.0.1:PORT`.
    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }
}

impl Drop for FakeProvider {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn handle_conn(mut stream: TcpStream, models_body: &str, chat_body: &str) -> std::io::Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(std::time::Duration::from_secs(2)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    let mut content_len = 0usize;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line)?;
        if line == "\r\n" || line.is_empty() {
            break;
        }
        if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            content_len = v.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0u8; content_len];
    reader.read_exact(&mut body)?;
    let mut parts = request_line.split_whitespace();
    let (method, path) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let path = path.split('?').next().unwrap_or("");
    let (status, payload) = match (method, path) {
        ("GET", "/models") | ("GET", "/v1/models") => ("200 OK", models_body),
        ("POST", "/chat/completions") | ("POST", "/v1/chat/completions") => ("200 OK", chat_body),
        _ => ("404 Not Found", "{\"error\":\"not found\"}"),
    };
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
        payload.len()
    )?;
    stream.flush()
}

/// Blocking GET helper returning `(status_line, body)` for tests.
pub fn http_get(addr_url: &str, path: &str) -> (String, String) {
    http_request("GET", addr_url, path, "")
}

/// Blocking request helper returning `(status_line, body)` for tests.
pub fn http_request(method: &str, addr_url: &str, path: &str, body: &str) -> (String, String) {
    let host = addr_url.trim_start_matches("http://");
    let mut s = TcpStream::connect(host).expect("connect");
    write!(
        s,
        "{method} {path} HTTP/1.1\r\nHost: {host}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .expect("send");
    let mut raw = String::new();
    s.read_to_string(&mut raw).expect("read");
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((&raw, ""));
    (head.lines().next().unwrap_or("").to_owned(), body.to_owned())
}
