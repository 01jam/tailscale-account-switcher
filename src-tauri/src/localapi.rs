//! A small client for tailscaled's LocalAPI, over its Unix socket.
//!
//! The `tailscale` CLI is built on this same API, so what comes back is the
//! JSON `tailscale status --json` prints. Asking the socket directly makes a
//! poll every few seconds cost a round-trip rather than a process, and it gives
//! two things the CLI keeps to itself: the control server of every account, and
//! a way to start a login on the account in front without `tailscale up`
//! demanding every non-default flag back.
//!
//! Only the socket is spoken to — no TCP port, no password — which is how Linux
//! and a standalone tailscaled on macOS expose it. The macOS app keeps its API
//! behind a token instead: there `socket()` finds nothing and every caller falls
//! back to the CLI.

use serde_json::Value;
use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

/// A local daemon answers in milliseconds. Anything slower is stuck, and the
/// poll behind this should move on rather than queue up behind it.
const TIMEOUT: Duration = Duration::from_secs(4);

/// The daemon refuses any other Host: it is how a browser tricked into talking
/// to the socket gets turned away.
const HOST: &str = "local-tailscaled.sock";

/// Where tailscaled listens, if it does so on a socket at all.
pub fn socket() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("TAILSCALE_SWITCH_SOCKET") {
        return Some(PathBuf::from(path));
    }
    [
        "/var/run/tailscale/tailscaled.sock",
        "/run/tailscale/tailscaled.sock",
        // A standalone tailscaled on macOS.
        "/var/run/tailscaled.socket",
    ]
    .iter()
    .map(PathBuf::from)
    .find(|path| path.exists())
}

#[derive(Debug)]
pub enum Error {
    /// No socket, nothing listening on it, or a reply that is not HTTP. The
    /// caller's cue to try the CLI instead.
    Unreachable(String),
    /// The daemon answered, and the answer was no: this user may not do that
    /// on the account in front.
    Denied(String),
    /// Any other refusal, with the daemon's own words.
    Failed(u16, String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Unreachable(why) => write!(f, "{why}"),
            Error::Denied(why) => write!(f, "Access denied: {why}"),
            Error::Failed(status, why) if why.is_empty() => write!(f, "HTTP {status}"),
            Error::Failed(_, why) => write!(f, "{why}"),
        }
    }
}

impl std::error::Error for Error {}

pub struct Reply {
    pub status: u16,
    pub body: Vec<u8>,
}

pub async fn get_json(path: &str) -> Result<Value, Error> {
    let reply = call("GET", path, None).await?;
    serde_json::from_slice(&reply.body).map_err(|e| Error::Unreachable(e.to_string()))
}

/// One request, one connection. HTTP/1.0 on purpose: the daemon's Go server
/// then answers with a plain body and closes, so reading to the end is reading
/// the whole reply — no keep-alive, no chunking to undo.
#[cfg(unix)]
pub async fn call(method: &str, path: &str, body: Option<&Value>) -> Result<Reply, Error> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::UnixStream;

    let socket = socket().ok_or_else(|| Error::Unreachable("no tailscaled socket".into()))?;
    let payload = body.map(Value::to_string).unwrap_or_default();
    let request = format!(
        "{method} {path} HTTP/1.0\r\nHost: {HOST}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{payload}",
        payload.len()
    );

    let exchange = async {
        let mut stream = UnixStream::connect(&socket).await?;
        stream.write_all(request.as_bytes()).await?;
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await?;
        Ok::<_, std::io::Error>(raw)
    };
    let raw = match tokio::time::timeout(TIMEOUT, exchange).await {
        Ok(Ok(raw)) => raw,
        Ok(Err(e)) => return Err(Error::Unreachable(e.to_string())),
        Err(_) => {
            return Err(Error::Unreachable(
                "tailscaled did not answer in time".into(),
            ))
        }
    };

    let reply = parse(&raw).ok_or_else(|| Error::Unreachable("malformed LocalAPI reply".into()))?;
    match reply.status {
        200..=299 => Ok(reply),
        403 => Err(Error::Denied(message_of(&reply.body))),
        status => Err(Error::Failed(status, message_of(&reply.body))),
    }
}

#[cfg(not(unix))]
pub async fn call(_method: &str, _path: &str, _body: Option<&Value>) -> Result<Reply, Error> {
    Err(Error::Unreachable(
        "no tailscaled socket on this platform".into(),
    ))
}

fn parse(raw: &[u8]) -> Option<Reply> {
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = std::str::from_utf8(&raw[..split]).ok()?;
    let mut lines = head.split("\r\n");
    let status = lines.next()?.split_whitespace().nth(1)?.parse().ok()?;
    let chunked = lines.any(|line| {
        let line = line.to_ascii_lowercase();
        line.starts_with("transfer-encoding:") && line.contains("chunked")
    });

    let body = &raw[split + 4..];
    let body = if chunked {
        dechunk(body)?
    } else {
        body.to_vec()
    };
    Some(Reply { status, body })
}

/// Not something the daemon sends an HTTP/1.0 client — but a reply that came
/// chunked anyway should read as its body, not as garbage JSON.
fn dechunk(mut body: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let end = body.windows(2).position(|w| w == b"\r\n")?;
        let size = std::str::from_utf8(&body[..end]).ok()?;
        let size = usize::from_str_radix(size.split(';').next()?.trim(), 16).ok()?;
        body = &body[end + 2..];
        if size == 0 {
            return Some(out);
        }
        out.extend_from_slice(body.get(..size)?);
        body = body.get(size + 2..)?;
    }
}

/// Refusals come as plain text from `http.Error`, or as `{"error": "…"}` from
/// the handlers that edit preferences.
fn message_of(body: &[u8]) -> String {
    if let Ok(value) = serde_json::from_slice::<Value>(body) {
        if let Some(error) = value.get("error").and_then(Value::as_str) {
            return error.trim().to_string();
        }
    }
    String::from_utf8_lossy(body).trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_plain_reply() {
        let raw = b"HTTP/1.0 200 OK\r\nContent-Type: application/json\r\n\r\n{\"a\":1}";
        let reply = parse(raw).unwrap();
        assert_eq!(reply.status, 200);
        assert_eq!(reply.body, b"{\"a\":1}");
    }

    #[test]
    fn reads_a_chunked_reply() {
        let raw = b"HTTP/1.1 403 Forbidden\r\nTransfer-Encoding: chunked\r\n\r\n8\r\nprofiles\r\nE\r\n access denied\r\n0\r\n\r\n";
        let reply = parse(raw).unwrap();
        assert_eq!(reply.status, 403);
        assert_eq!(message_of(&reply.body), "profiles access denied");
    }

    #[test]
    fn prefers_the_json_error_field() {
        assert_eq!(message_of(b"{\"error\":\"nope\"}\n"), "nope");
        assert_eq!(message_of(b"login access denied\n"), "login access denied");
    }

    #[test]
    fn rejects_what_is_not_http() {
        assert!(parse(b"garbage").is_none());
    }
}
