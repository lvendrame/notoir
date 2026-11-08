//! Single-instance forwarding (spec 31 §Single Instance, AD-10): the first
//! process listens on a per-user local socket; later launches forward their
//! arguments to it and exit; `--new-window` is a host request.
//!
//! Protocol: the working directory, then one argument per line, then an empty
//! line; the primary answers `ok`. With `--wait` among the arguments the
//! primary keeps the connection and writes `done` once the files of that
//! request are closed (spec 38); the forwarding process waits for it.

use std::io::{self, BufRead, BufReader, Write};
use std::path::PathBuf;

use interprocess::local_socket::prelude::*;
use interprocess::local_socket::{GenericNamespaced, ListenerOptions, Stream};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstanceRequest {
    pub cwd: PathBuf,
    pub args: Vec<String>,
}

/// The open connection of a `--wait` request. [`Waiter::finish`] releases
/// the waiting terminal; dropping it (the app exits) releases it too.
pub struct Waiter(Option<Stream>);

impl std::fmt::Debug for Waiter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Waiter")
    }
}

impl Waiter {
    pub fn finish(mut self) {
        if let Some(mut s) = self.0.take() {
            let _ = s.write_all(b"done\n");
        }
    }
}

/// Whether a request asks to wait (`--wait`).
pub fn wants_wait(args: &[String]) -> bool {
    args.iter()
        .take_while(|a| a.as_str() != "--")
        .any(|a| a == "--wait" || a == "-w")
}

pub enum Claim {
    /// This process is the primary; requests from later launches arrive via
    /// the callback (on a background thread).
    Primary,
    /// Another instance took the request; this process should exit.
    Forwarded,
}

/// Per-user socket name.
pub fn socket_name() -> String {
    // A Store launch must not forward into a separately installed MSI/NSIS
    // process, which has a different updater and package-owned integrations.
    let app = if cfg!(feature = "microsoft-store") {
        format!("{}.store", crate::paths::BUNDLE_ID)
    } else {
        crate::paths::BUNDLE_ID.to_owned()
    };
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_default();
    // A relocated home (tests, portable setups) gets its own instance.
    match std::env::var(crate::paths::HOME_ENV) {
        Ok(home) if !home.is_empty() => {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            let home_path = std::path::PathBuf::from(&home);
            let home_path = home_path.canonicalize().unwrap_or_else(|_| {
                if home_path.is_absolute() {
                    home_path.clone()
                } else {
                    std::env::current_dir().unwrap_or_default().join(&home_path)
                }
            });
            home_path.hash(&mut h);
            format!("{app}-{user}-{:x}.sock", h.finish())
        }
        _ => format!("{app}-{user}.sock"),
    }
}

fn encode(req: &InstanceRequest) -> String {
    let mut out = req.cwd.to_string_lossy().replace('\n', " ");
    out.push('\n');
    for a in &req.args {
        out.push_str(&a.replace('\n', " "));
        out.push('\n');
    }
    out.push('\n');
    out
}

fn decode(reader: &mut impl BufRead) -> io::Result<InstanceRequest> {
    let mut lines = Vec::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        let line = line.trim_end_matches(['\n', '\r']).to_owned();
        if line.is_empty() {
            break;
        }
        lines.push(line);
    }
    if lines.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "empty request"));
    }
    let cwd = PathBuf::from(lines.remove(0));
    Ok(InstanceRequest { cwd, args: lines })
}

/// Forwards `request` to a running instance, or becomes the primary.
pub fn claim(
    name: &str,
    request: &InstanceRequest,
    on_request: impl Fn(InstanceRequest, Option<Waiter>) + Send + 'static,
) -> io::Result<Claim> {
    claim_with_gate(
        name,
        request,
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
        on_request,
    )
}

/// Like claim, with a lifecycle gate that rejects launches during coordinated Quit.
pub fn claim_with_gate(
    name: &str,
    request: &InstanceRequest,
    accepting: std::sync::Arc<std::sync::atomic::AtomicBool>,
    on_request: impl Fn(InstanceRequest, Option<Waiter>) + Send + 'static,
) -> io::Result<Claim> {
    if forward(name, request)? {
        return Ok(Claim::Forwarded);
    }
    // Serialize simultaneous first launches before reclaiming a stale socket.
    use std::hash::{Hash, Hasher};
    let paths = crate::paths::AppPaths::resolve().map_err(io::Error::other)?;
    paths.ensure_dirs().map_err(io::Error::other)?;
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    name.hash(&mut hash);
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(
            paths
                .data
                .join(format!("instance-{:x}.lock", hash.finish())),
        )?;
    if lock.try_lock().is_err() {
        for _ in 0..100 {
            if forward(name, request)? {
                return Ok(Claim::Forwarded);
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "Notoir host did not accept launch",
        ));
    }
    let ns = name.to_ns_name::<GenericNamespaced>()?;
    let listener = ListenerOptions::new()
        .name(ns)
        .try_overwrite(true)
        .create_sync()?;
    std::thread::Builder::new()
        .name("notoir-instance".into())
        .spawn(move || {
            let _instance_lock = lock;
            for conn in listener.incoming().filter_map(Result::ok) {
                let mut conn = BufReader::new(conn);
                match decode(&mut conn) {
                    Ok(req) => {
                        if !accepting.load(std::sync::atomic::Ordering::Acquire) {
                            let _ = conn.get_mut().write_all(b"busy\n");
                            continue;
                        }
                        let _ = conn.get_mut().write_all(b"ok\n");
                        let waiter = wants_wait(&req.args).then(|| Waiter(Some(conn.into_inner())));
                        on_request(req, waiter);
                    }
                    Err(e) => tracing::warn!(error = %e, "bad single-instance request"),
                }
            }
        })?;
    Ok(Claim::Primary)
}

/// Hands `request` to a running instance; true when one took it. With
/// `--wait`, returns only after that instance answers `done` (or exits).
pub fn forward(name: &str, request: &InstanceRequest) -> io::Result<bool> {
    let ns = name.to_ns_name::<GenericNamespaced>()?;
    let Ok(stream) = Stream::connect(ns) else {
        return Ok(false);
    };
    let mut stream = BufReader::new(stream);
    stream.get_mut().write_all(encode(request).as_bytes())?;
    let mut answer = String::new();
    stream.read_line(&mut answer)?;
    if answer.trim() == "busy" {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "Notoir is quitting; retry after cancelling Quit or after it exits",
        ));
    }
    if answer.trim() != "ok" {
        return Ok(false);
    }
    if wants_wait(&request.args) {
        loop {
            let mut line = String::new();
            // EOF: the instance exited.
            if stream.read_line(&mut line)? == 0 || line.trim() == "done" {
                break;
            }
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;
    use std::time::Duration;

    // spec 31: "Second launch opens file in running instance"
    #[test]
    fn second_launch_forwards_to_primary() {
        let name = format!("notoir-test-{}.sock", std::process::id());
        let (tx, rx) = channel();
        let first = InstanceRequest {
            cwd: "/".into(),
            args: vec![],
        };
        let claimed = claim(&name, &first, move |r, _| tx.send(r).unwrap()).unwrap();
        assert!(matches!(claimed, Claim::Primary));
        let second = InstanceRequest {
            cwd: "/home/me".into(),
            args: vec!["notes.md".into()],
        };
        let forwarded = claim(&name, &second, |_, _| panic!("not primary")).unwrap();
        assert!(matches!(forwarded, Claim::Forwarded));
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), second);
    }

    #[test]
    fn quitting_host_rejects_launch_without_delivering_it() {
        let name = format!("notoir-gated-{}.sock", std::process::id());
        let gate = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (tx, rx) = channel();
        let request = InstanceRequest {
            cwd: "/".into(),
            args: vec![],
        };
        assert!(matches!(
            claim_with_gate(&name, &request, gate.clone(), move |r, _| {
                let _ = tx.send(r);
            })
            .unwrap(),
            Claim::Primary
        ));
        assert_eq!(
            forward(&name, &request).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        assert!(rx.try_recv().is_err());
        gate.store(true, std::sync::atomic::Ordering::Release);
        assert!(forward(&name, &request).unwrap());
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap(), request);
    }

    #[test]
    fn protocol_round_trip() {
        let req = InstanceRequest {
            cwd: "/x".into(),
            args: vec!["a b".into(), "--goto".into(), "c:3".into()],
        };
        let text = encode(&req);
        assert_eq!(decode(&mut text.as_bytes()).unwrap(), req);
    }

    // spec 38: "Wait exits after opened tabs close" (protocol level)
    #[test]
    fn wait_blocks_until_done() {
        let name = format!("notoir-wait-{}.sock", std::process::id());
        let (tx, rx) = channel::<Waiter>();
        let first = InstanceRequest {
            cwd: "/".into(),
            args: vec![],
        };
        let claimed = claim(&name, &first, move |_, w| {
            if let Some(w) = w {
                tx.send(w).unwrap();
            }
        })
        .unwrap();
        assert!(matches!(claimed, Claim::Primary));
        let req = InstanceRequest {
            cwd: "/".into(),
            args: vec!["--wait".into(), "notes.md".into()],
        };
        let n = name.clone();
        let client = std::thread::spawn(move || forward(&n, &req).unwrap());
        let waiter = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        assert!(!client.is_finished(), "returned before done");
        waiter.finish();
        assert!(client.join().unwrap());
    }
}
