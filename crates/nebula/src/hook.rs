//! `nebula _hook <endpoint> <event>`: the agent-CLI hook command on Windows,
//! where a CLI may run hooks under cmd, PowerShell or Git Bash and the unix
//! `curl … ${NEBULA_API_TOKEN}` one-liner cannot be relied on.
//!
//! It does exactly what that one-liner does (see `hooks::installer`): exit
//! quietly when `NEBULA_AGENT_ID` / `NEBULA_API_URL` are unset (a bare
//! `claude` outside nebula), otherwise POST the payload from stdin to
//! `$NEBULA_API_URL/api/hooks/<endpoint>?agentId=…&hookEvent=<event>` with
//! the bearer token, giving up after a few seconds. The reply reaches stdout
//! only for `UserPromptSubmit` (the AUTO-TITLE channel), and cursor always
//! gets its `{"continue": true}`. Every failure is silent and the exit
//! status is always 0: a broken hook must never fault the agent's turn.

use nebula_core::env;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

/// How long each step of the POST may take — the curl one-liner's `-m 3`.
const TIMEOUT: Duration = Duration::from_secs(3);

pub fn run_hook(endpoint: &str, event: &str) -> anyhow::Result<()> {
    let cursor = endpoint == "cursor";
    let reply = post(endpoint, event);
    let mut out = std::io::stdout().lock();
    if event == "UserPromptSubmit" && !cursor {
        if let Some(body) = reply {
            let _ = out.write_all(body.as_bytes());
        }
    }
    if cursor {
        let _ = writeln!(out, "{{\"continue\": true}}");
    }
    let _ = out.flush();
    Ok(())
}

/// The daemon's reply body, or None for anything short of a 200.
fn post(endpoint: &str, event: &str) -> Option<String> {
    let agent = env::non_empty(env::AGENT_ID)?;
    let url = env::non_empty(env::API_URL)?;
    let token = env::non_empty(env::API_TOKEN).unwrap_or_default();
    if ![endpoint, event, &agent].iter().all(|word| is_query_safe(word)) {
        return None;
    }
    let mut payload = Vec::new();
    std::io::stdin().read_to_end(&mut payload).ok()?;

    let rest = url.strip_prefix("http://")?;
    let (authority, base) = match rest.find('/') {
        Some(slash) => rest.split_at(slash),
        None => (rest, ""),
    };
    let addr = authority.to_socket_addrs().ok()?.next()?;
    let mut stream = TcpStream::connect_timeout(&addr, TIMEOUT).ok()?;
    stream.set_read_timeout(Some(TIMEOUT)).ok()?;
    stream.set_write_timeout(Some(TIMEOUT)).ok()?;
    let request = format!(
        "POST {}/api/hooks/{endpoint}?agentId={agent}&hookEvent={event} HTTP/1.1\r\n\
         Host: {authority}\r\n\
         Authorization: Bearer {token}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\r\n",
        base.trim_end_matches('/'),
        payload.len()
    );
    stream.write_all(request.as_bytes()).ok()?;
    stream.write_all(&payload).ok()?;
    let mut response = Vec::new();
    stream.read_to_end(&mut response).ok()?;
    response_body(&response)
}

/// A word that goes into the query string as it is: ids and event names
/// are plain, and anything else is not a nebula session's.
fn is_query_safe(word: &str) -> bool {
    !word.is_empty()
        && word
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// The body of an HTTP/1.1 response read to its end, when it is a 200.
fn response_body(response: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(response);
    let (head, body) = text.split_once("\r\n\r\n")?;
    let mut lines = head.lines();
    if lines.next()?.split_whitespace().nth(1)? != "200" {
        return None;
    }
    let chunked = lines.any(|line| {
        let line = line.to_ascii_lowercase();
        line.starts_with("transfer-encoding:") && line.contains("chunked")
    });
    Some(if chunked {
        dechunk(body)?
    } else {
        body.to_string()
    })
}

/// Undo `Transfer-Encoding: chunked`.
fn dechunk(mut body: &str) -> Option<String> {
    let mut out = String::new();
    loop {
        let (size, rest) = body.split_once("\r\n")?;
        let size = usize::from_str_radix(size.split(';').next()?.trim(), 16).ok()?;
        if size == 0 {
            return Some(out);
        }
        out.push_str(rest.get(..size)?);
        body = rest.get(size..)?.strip_prefix("\r\n")?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_200_gives_its_body_and_anything_else_none() {
        let ok = b"HTTP/1.1 200 OK\r\ncontent-length: 5\r\n\r\nhello";
        assert_eq!(response_body(ok).as_deref(), Some("hello"));
        let empty = b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n";
        assert_eq!(response_body(empty).as_deref(), Some(""));
        let denied = b"HTTP/1.1 401 Unauthorized\r\n\r\nnope";
        assert_eq!(response_body(denied), None);
        assert_eq!(response_body(b"garbage"), None);
    }

    #[test]
    fn a_chunked_body_is_reassembled() {
        let chunked =
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\n{\"a\"\r\n3\r\n:1}\r\n0\r\n\r\n";
        assert_eq!(response_body(chunked).as_deref(), Some("{\"a\":1}"));
    }

    #[test]
    fn only_plain_words_reach_the_query_string() {
        assert!(is_query_safe("01JABCDEF"));
        assert!(is_query_safe("UserPromptSubmit"));
        assert!(!is_query_safe("a&b"));
        assert!(!is_query_safe(""));
    }
}
