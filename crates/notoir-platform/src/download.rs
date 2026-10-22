//! Bounded HTTP(S) document downloads. No document or UI policy lives here.
use std::{
    io::Read,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use ureq::ResponseExt;
pub const MAX_BYTES: usize = 20 * 1024 * 1024;
#[derive(Debug)]
pub struct Download {
    pub url: String,
    pub content_type: String,
    pub name: String,
    pub bytes: Vec<u8>,
}
pub fn valid_url(url: &str) -> bool {
    url.parse::<ureq::http::Uri>().is_ok_and(|u| {
        matches!(u.scheme_str(), Some("http" | "https"))
            && u.authority()
                .is_some_and(|a| !a.as_str().contains('@') && !a.host().is_empty())
    })
}
pub fn fetch(url: &str, cancelled: &AtomicBool) -> Result<Download, String> {
    if !valid_url(url) {
        return Err("invalidUrl".into());
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(30)))
        .max_redirects(5)
        .build()
        .into();
    let mut response = agent.get(url).call().map_err(|e| e.to_string())?;
    if cancelled.load(Ordering::Relaxed) {
        return Err("cancelled".into());
    }
    let url = response.get_uri().to_string();
    if !valid_url(&url) {
        return Err("invalidUrl".into());
    }
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .to_lowercase();
    if response
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<usize>().ok())
        .is_some_and(|n| n > MAX_BYTES)
    {
        return Err("tooLarge".into());
    }
    let mut bytes = Vec::new();
    let mut reader = response.body_mut().as_reader();
    let mut chunk = [0; 16 * 1024];
    loop {
        if cancelled.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        let n = reader.read(&mut chunk).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        if bytes.len() + n > MAX_BYTES {
            return Err("tooLarge".into());
        }
        bytes.extend_from_slice(&chunk[..n]);
    }
    let path = url.parse::<ureq::http::Uri>().map_err(|e| e.to_string())?;
    let raw = path.path().rsplit('/').next().unwrap_or("");
    let mut name = decode_name(raw);
    if name.is_empty() {
        name = "download".into();
    }
    if !name.contains('.') {
        let ext = match content_type.as_str() {
            "text/csv" => ".csv",
            "application/json" => ".json",
            "text/markdown" => ".md",
            "text/html" => ".html",
            "application/yaml" | "text/yaml" => ".yaml",
            _ => ".txt",
        };
        name.push_str(ext);
    }
    Ok(Download {
        url,
        content_type,
        name,
        bytes,
    })
}
fn decode_name(raw: &str) -> String {
    let mut bytes = Vec::new();
    let mut i = 0;
    while i < raw.len() {
        if raw.as_bytes()[i] == b'%'
            && i + 2 < raw.len()
            && let Ok(b) = u8::from_str_radix(&raw[i + 1..i + 3], 16)
        {
            bytes.push(b);
            i += 3;
            continue;
        }
        bytes.push(raw.as_bytes()[i]);
        i += 1;
    }
    String::from_utf8_lossy(&bytes)
        .chars()
        .map(|c| {
            if c.is_control() || "/\\:*?\"<>|".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Write, net::TcpListener};
    fn fixture(header: &'static str, body: Vec<u8>) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!(
            "http://{}/folder/%E2%82%AC.csv",
            listener.local_addr().unwrap()
        );
        let worker = std::thread::spawn(move || {
            let until = std::time::Instant::now() + Duration::from_secs(10);
            let mut stream = loop {
                if let Ok((s, _)) = listener.accept() {
                    break s;
                }
                assert!(std::time::Instant::now() < until);
                std::thread::sleep(Duration::from_millis(10));
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let mut block = [0; 1024];
                let Ok(n) = stream.read(&mut block) else {
                    return;
                };
                if n == 0 {
                    return;
                }
                request.extend_from_slice(&block[..n]);
                // A TLS client intentionally sends a non-HTTP handshake in the TLS failure test.
                if request.first() == Some(&0x16) {
                    break;
                }
            }
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\n{header}\r\nConnection: close\r\n\r\n"
            );
            if header.contains("chunked") {
                let _ = write!(stream, "{:x}\r\n", body.len());
            }
            let _ = stream.write_all(&body);
            if header.contains("chunked") {
                let _ = stream.write_all(b"\r\n0\r\n\r\n");
            }
        });
        (url, worker)
    }
    #[test]
    fn http_download_names_and_bounds() {
        let cancelled = AtomicBool::new(false);
        let (url, worker) = fixture("Content-Type: text/csv", b"a,b\n1,2\n".to_vec());
        let result = fetch(&url, &cancelled).unwrap();
        worker.join().unwrap();
        assert_eq!(result.name, "€.csv");
        assert_eq!(result.bytes, b"a,b\n1,2\n");
        let (url, worker) = fixture("Content-Length: 20971521", vec![]);
        assert_eq!(
            fetch(&url, &cancelled).map(|d| d.bytes.len()),
            Err("tooLarge".into())
        );
        worker.join().unwrap();
        let (url, worker) = fixture(
            "Content-Type: text/plain\r\nTransfer-Encoding: chunked",
            vec![b'a'; MAX_BYTES + 1],
        );
        assert_eq!(
            fetch(&url, &cancelled).map(|d| d.bytes.len()),
            Err("tooLarge".into())
        );
        worker.join().unwrap();
    }
    #[test]
    fn url_validation_and_tls_failure() {
        assert!(!valid_url("file:///tmp/notes.txt"));
        assert!(!valid_url("https://user:pass@example.com/a"));
        assert!(!valid_url("https://"));
        let (url, worker) = fixture("Content-Type: text/plain", vec![]);
        assert!(fetch(&url.replacen("http:", "https:", 1), &AtomicBool::new(false)).is_err());
        worker.join().unwrap();
    }
}
