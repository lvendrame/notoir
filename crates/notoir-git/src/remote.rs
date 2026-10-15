//! Network operations through the system `git` (docs/specs/28 §Architecture):
//! a child process that can never wait on a hidden prompt, with its output
//! streamed line by line (progress parsed) and cancellation by killing it.
//! Notoir stores no credentials; Git's own helpers and ssh-agent apply.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::{GitError, RepoInfo};

/// One piece of streamed output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitOutput {
    /// A command line about to run (`> git push`).
    Command(String),
    Line(String),
    /// `Receiving objects:  45% (9/20)` → (45, "Receiving objects").
    Progress {
        percent: u8,
        phase: String,
    },
}

pub type OutputSink = Arc<dyn Fn(GitOutput) + Send + Sync>;

/// Cancels a running network operation (the child is killed).
#[derive(Clone, Debug, Default)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FetchOpts {
    pub prune: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PullMode {
    #[default]
    FfOnly,
    Rebase,
    Merge,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PullResult {
    UpToDate,
    Updated,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PushOpts {
    /// First push of a branch: `git push -u <remote> <branch>`.
    pub set_upstream: Option<(String, String)>,
}

/// Network operations (fetch, pull, push, clone).
pub trait GitRemote: Send + Sync {
    fn fetch(
        &self,
        repo: &RepoInfo,
        opts: FetchOpts,
        out: OutputSink,
        cancel: CancellationToken,
    ) -> Result<(), GitError>;
    /// `Err(Diverged)` when a fast-forward-only pull is not possible;
    /// `Err(Conflict)` when a rebase stopped (it is aborted first).
    fn pull(
        &self,
        repo: &RepoInfo,
        mode: PullMode,
        out: OutputSink,
        cancel: CancellationToken,
    ) -> Result<PullResult, GitError>;
    fn push(
        &self,
        repo: &RepoInfo,
        opts: PushOpts,
        out: OutputSink,
        cancel: CancellationToken,
    ) -> Result<(), GitError>;
    fn clone_repo(
        &self,
        url: &str,
        dest: &Path,
        out: OutputSink,
        cancel: CancellationToken,
    ) -> Result<(), GitError>;
}

/// The system `git` executable.
#[derive(Clone, Debug)]
pub struct SystemGit {
    pub program: PathBuf,
}

/// `Receiving objects:  45% (9/20)` → (45, "Receiving objects").
pub fn parse_progress(line: &str) -> Option<(u8, String)> {
    let line = line.strip_prefix("remote: ").unwrap_or(line);
    let (phase, rest) = line.split_once(':')?;
    let pct = rest.trim_start().split('%').next()?;
    if pct.is_empty() || !pct.chars().all(|c| c.is_ascii_digit()) || !rest.contains('%') {
        return None;
    }
    Some((pct.parse::<u8>().ok()?.min(100), phase.trim().to_owned()))
}

/// Maps git's stderr to a typed error (`None`: a generic failure).
pub fn classify_stderr(stderr: &str) -> Option<GitError> {
    let s = stderr.to_lowercase();
    let any = |needles: &[&str]| needles.iter().any(|n| s.contains(n));
    if any(&[
        "authentication failed",
        "could not read username",
        "could not read password",
        "terminal prompts disabled",
        "permission denied (publickey",
        "invalid username or password",
        "authentication required",
        "host key verification failed",
    ]) {
        Some(GitError::Auth)
    } else if any(&[
        "not possible to fast-forward",
        "diverging branches",
        "have diverged",
    ]) {
        Some(GitError::Diverged)
    } else if any(&[
        "has no upstream branch",
        "no tracking information",
        "no upstream configured",
    ]) {
        Some(GitError::NoUpstream)
    } else if any(&["[rejected]", "non-fast-forward", "fetch first"]) {
        Some(GitError::Rejected)
    } else if any(&["index.lock", "another git process"]) {
        Some(GitError::Locked)
    } else {
        None
    }
}

/// A child's stdout or stderr.
type Pipe = Box<dyn Read + Send>;

struct Run {
    ok: bool,
    stdout: String,
    stderr: String,
}

impl SystemGit {
    /// `configured` (setting `git.path`) or `git` on PATH, if it runs.
    pub fn find(configured: Option<&str>) -> Option<Self> {
        let program = PathBuf::from(configured.filter(|s| !s.is_empty()).unwrap_or("git"));
        let ok = Command::new(&program)
            .arg("--version")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        ok.then_some(Self { program })
    }

    fn command(&self, cwd: &Path, args: &[&str]) -> Command {
        let mut cmd = Command::new(&self.program);
        cmd.current_dir(cwd)
            .args(["-c", "color.ui=false", "-c", "core.quotepath=false"])
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // Never wait on a prompt nobody can see.
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GCM_INTERACTIVE", "never")
            .env("LC_ALL", "C");
        if std::env::var_os("GIT_SSH_COMMAND").is_none() && std::env::var_os("GIT_SSH").is_none() {
            cmd.env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes");
        }
        cmd
    }

    /// Runs git, streaming both pipes into `out` until it exits or `cancel`
    /// fires (the child is killed).
    fn run(
        &self,
        cwd: &Path,
        args: &[&str],
        out: &OutputSink,
        cancel: &CancellationToken,
    ) -> Result<Run, GitError> {
        out(GitOutput::Command(format!("git {}", args.join(" "))));
        let mut child = self
            .command(cwd, args)
            .spawn()
            .map_err(|e| GitError::Git(e.to_string()))?;
        let stdout_text = Arc::new(Mutex::new(String::new()));
        let stderr_text = Arc::new(Mutex::new(String::new()));
        let mut readers = Vec::new();
        let pipes: [(Option<Pipe>, Arc<Mutex<String>>); 2] = [
            (
                child
                    .stdout
                    .take()
                    .map(|p| Box::new(p) as Box<dyn Read + Send>),
                stdout_text.clone(),
            ),
            (
                child
                    .stderr
                    .take()
                    .map(|p| Box::new(p) as Box<dyn Read + Send>),
                stderr_text.clone(),
            ),
        ];
        for (pipe, text) in pipes {
            let Some(mut pipe) = pipe else { continue };
            let out = out.clone();
            readers.push(std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                let mut pending = Vec::new();
                let emit = |bytes: &[u8]| {
                    let line = String::from_utf8_lossy(bytes).trim_end().to_owned();
                    if line.is_empty() {
                        return;
                    }
                    if let Ok(mut t) = text.lock() {
                        t.push_str(&line);
                        t.push('\n');
                    }
                    match parse_progress(&line) {
                        Some((percent, phase)) => {
                            out(GitOutput::Progress { percent, phase });
                            // Only the final 100% line goes to the log.
                            if percent == 100 {
                                out(GitOutput::Line(line));
                            }
                        }
                        None => out(GitOutput::Line(line)),
                    }
                };
                loop {
                    match pipe.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            for &b in &buf[..n] {
                                if b == b'\n' || b == b'\r' {
                                    emit(&pending);
                                    pending.clear();
                                } else {
                                    pending.push(b);
                                }
                            }
                        }
                    }
                }
                emit(&pending);
            }));
        }
        let status = loop {
            if cancel.is_cancelled() {
                let _ = child.kill();
                let _ = child.wait();
                for r in readers {
                    let _ = r.join();
                }
                return Err(GitError::Cancelled);
            }
            match child.try_wait() {
                Ok(Some(s)) => break s,
                Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                Err(e) => return Err(GitError::Git(e.to_string())),
            }
        };
        for r in readers {
            let _ = r.join();
        }
        let take = |m: Arc<Mutex<String>>| m.lock().map(|s| s.clone()).unwrap_or_default();
        Ok(Run {
            ok: status.success(),
            stdout: take(stdout_text),
            stderr: take(stderr_text),
        })
    }

    fn failure(run: &Run) -> GitError {
        classify_stderr(&run.stderr).unwrap_or_else(|| {
            let last = run
                .stderr
                .lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("git failed");
            GitError::Git(last.trim().to_owned())
        })
    }
}

impl GitRemote for SystemGit {
    fn fetch(
        &self,
        repo: &RepoInfo,
        opts: FetchOpts,
        out: OutputSink,
        cancel: CancellationToken,
    ) -> Result<(), GitError> {
        let mut args = vec!["fetch", "--progress"];
        if opts.prune {
            args.push("--prune");
        }
        let run = self.run(&repo.workdir, &args, &out, &cancel)?;
        if run.ok {
            Ok(())
        } else {
            Err(Self::failure(&run))
        }
    }

    fn pull(
        &self,
        repo: &RepoInfo,
        mode: PullMode,
        out: OutputSink,
        cancel: CancellationToken,
    ) -> Result<PullResult, GitError> {
        let args: &[&str] = match mode {
            PullMode::FfOnly => &["pull", "--progress", "--ff-only"],
            PullMode::Rebase => &["pull", "--progress", "--rebase"],
            PullMode::Merge => &["pull", "--progress", "--no-rebase", "--no-edit"],
        };
        let run = self.run(&repo.workdir, args, &out, &cancel)?;
        if run.ok {
            let up_to_date = run.stdout.to_lowercase().contains("up to date");
            return Ok(if up_to_date {
                PullResult::UpToDate
            } else {
                PullResult::Updated
            });
        }
        let err = Self::failure(&run);
        let text = format!("{}\n{}", run.stdout, run.stderr).to_lowercase();
        if mode == PullMode::Rebase && text.contains("conflict") {
            // Leave the user's commits as they were (spec 28 Sync Notes).
            let _ = self.run(&repo.workdir, &["rebase", "--abort"], &out, &cancel);
            return Err(GitError::Conflict);
        }
        if mode == PullMode::Merge && text.contains("conflict") {
            return Err(GitError::Conflict);
        }
        Err(err)
    }

    fn push(
        &self,
        repo: &RepoInfo,
        opts: PushOpts,
        out: OutputSink,
        cancel: CancellationToken,
    ) -> Result<(), GitError> {
        let mut args = vec!["push", "--progress"];
        if let Some((remote, branch)) = &opts.set_upstream {
            args.extend(["-u", remote.as_str(), branch.as_str()]);
        }
        let run = self.run(&repo.workdir, &args, &out, &cancel)?;
        if run.ok {
            Ok(())
        } else {
            Err(Self::failure(&run))
        }
    }

    fn clone_repo(
        &self,
        url: &str,
        dest: &Path,
        out: OutputSink,
        cancel: CancellationToken,
    ) -> Result<(), GitError> {
        let parent = dest.parent().unwrap_or(Path::new("."));
        let dest_s = dest.to_string_lossy();
        let run = self.run(
            parent,
            &["clone", "--progress", "--", url, &dest_s],
            &out,
            &cancel,
        )?;
        if run.ok {
            Ok(())
        } else {
            Err(Self::failure(&run))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Git2Backend, GitBackend, StatusOpts};

    #[test]
    fn progress_lines() {
        assert_eq!(
            parse_progress("Receiving objects:  45% (9/20)"),
            Some((45, "Receiving objects".into()))
        );
        assert_eq!(
            parse_progress("remote: Counting objects: 100% (3/3), done."),
            Some((100, "Counting objects".into()))
        );
        assert_eq!(parse_progress("To github.com:a/b.git"), None);
        assert_eq!(parse_progress("error: 50 things"), None);
    }

    #[test]
    fn stderr_classification() {
        let auth =
            "fatal: could not read Username for 'https://github.com': terminal prompts disabled";
        assert_eq!(classify_stderr(auth), Some(GitError::Auth));
        assert_eq!(
            classify_stderr("git@github.com: Permission denied (publickey)."),
            Some(GitError::Auth)
        );
        assert_eq!(
            classify_stderr("fatal: Not possible to fast-forward, aborting."),
            Some(GitError::Diverged)
        );
        assert_eq!(
            classify_stderr("fatal: The current branch x has no upstream branch."),
            Some(GitError::NoUpstream)
        );
        assert_eq!(
            classify_stderr(" ! [rejected]        main -> main (fetch first)"),
            Some(GitError::Rejected)
        );
        assert_eq!(classify_stderr("fatal: something else"), None);
    }

    fn git() -> Option<SystemGit> {
        let g = SystemGit::find(None);
        if g.is_none() {
            eprintln!("system git not found; skipping");
        }
        g
    }

    fn sink() -> (OutputSink, Arc<Mutex<Vec<GitOutput>>>) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let l = log.clone();
        (Arc::new(move |o| l.lock().unwrap().push(o)), log)
    }

    fn sh(cwd: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .current_dir(cwd)
            .args(args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .unwrap();
        assert!(
            ok.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&ok.stderr)
        );
    }

    fn identity(dir: &Path) {
        sh(dir, &["config", "user.name", "Tester"]);
        sh(dir, &["config", "user.email", "t@example.com"]);
        sh(dir, &["config", "commit.gpgsign", "false"]);
        // Independent of the developer's global configuration.
        sh(dir, &["config", "push.autoSetupRemote", "false"]);
        sh(dir, &["config", "pull.rebase", "false"]);
    }

    /// A bare "remote" and two clones of it with one shared commit.
    fn remote_and_clones(g: &SystemGit) -> (tempfile::TempDir, RepoInfo, RepoInfo) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let bare = root.join("remote.git");
        sh(&root, &["init", "--bare", "-b", "main", "remote.git"]);
        let url = bare.to_string_lossy().into_owned();
        let (out, _) = sink();
        for name in ["a", "b"] {
            g.clone_repo(&url, &root.join(name), out.clone(), Default::default())
                .unwrap();
            identity(&root.join(name));
        }
        let a = Git2Backend.discover(&root.join("a")).unwrap().unwrap();
        let b = Git2Backend.discover(&root.join("b")).unwrap().unwrap();
        // First commit and push from a (sets the upstream).
        std::fs::write(a.workdir.join("n.md"), "1\n2\n3\n").unwrap();
        sh(&a.workdir, &["symbolic-ref", "HEAD", "refs/heads/main"]);
        sh(&a.workdir, &["add", "."]);
        sh(&a.workdir, &["commit", "-q", "-m", "first"]);
        g.push(
            &a,
            PushOpts {
                set_upstream: Some(("origin".into(), "main".into())),
            },
            out.clone(),
            Default::default(),
        )
        .unwrap();
        g.fetch(&b, FetchOpts::default(), out.clone(), Default::default())
            .unwrap();
        sh(&b.workdir, &["checkout", "-q", "main"]);
        // A new branch has no upstream until its first push.
        sh(&b.workdir, &["checkout", "-q", "-b", "topic"]);
        assert_eq!(
            g.push(&b, PushOpts::default(), out.clone(), Default::default()),
            Err(GitError::NoUpstream)
        );
        sh(&b.workdir, &["checkout", "-q", "main"]);
        (dir, a, b)
    }

    #[test]
    fn push_pull_and_divergence() {
        let Some(g) = git() else { return };
        let (_d, a, b) = remote_and_clones(&g);
        let (out, log) = sink();
        // b fast-forwards to a's new commit.
        std::fs::write(a.workdir.join("n.md"), "1\n2\n3\n4\n").unwrap();
        sh(&a.workdir, &["commit", "-q", "-am", "four"]);
        g.push(&a, PushOpts::default(), out.clone(), Default::default())
            .unwrap();
        assert_eq!(
            g.pull(&b, PullMode::FfOnly, out.clone(), Default::default()),
            Ok(PullResult::Updated)
        );
        assert_eq!(
            g.pull(&b, PullMode::FfOnly, out.clone(), Default::default()),
            Ok(PullResult::UpToDate)
        );
        // Diverge: both commit different files.
        std::fs::write(a.workdir.join("x.md"), "x\n").unwrap();
        sh(&a.workdir, &["add", "."]);
        sh(&a.workdir, &["commit", "-q", "-m", "x"]);
        g.push(&a, PushOpts::default(), out.clone(), Default::default())
            .unwrap();
        std::fs::write(b.workdir.join("y.md"), "y\n").unwrap();
        sh(&b.workdir, &["add", "."]);
        sh(&b.workdir, &["commit", "-q", "-m", "y"]);
        assert_eq!(
            g.push(&b, PushOpts::default(), out.clone(), Default::default()),
            Err(GitError::Rejected)
        );
        assert_eq!(
            g.pull(&b, PullMode::FfOnly, out.clone(), Default::default()),
            Err(GitError::Diverged)
        );
        assert_eq!(
            g.pull(&b, PullMode::Rebase, out.clone(), Default::default()),
            Ok(PullResult::Updated)
        );
        g.push(&b, PushOpts::default(), out.clone(), Default::default())
            .unwrap();
        let s = Git2Backend.status(&b, StatusOpts::default()).unwrap();
        assert_eq!((s.head.ahead, s.head.behind), (0, 0));
        let log = log.lock().unwrap();
        assert!(
            log.iter()
                .any(|o| matches!(o, GitOutput::Command(c) if c.starts_with("git push")))
        );
    }

    #[test]
    fn rebase_conflict_is_aborted() {
        let Some(g) = git() else { return };
        let (_d, a, b) = remote_and_clones(&g);
        let (out, _) = sink();
        std::fs::write(a.workdir.join("n.md"), "A\n2\n3\n").unwrap();
        sh(&a.workdir, &["commit", "-q", "-am", "a"]);
        g.push(&a, PushOpts::default(), out.clone(), Default::default())
            .unwrap();
        std::fs::write(b.workdir.join("n.md"), "B\n2\n3\n").unwrap();
        sh(&b.workdir, &["commit", "-q", "-am", "b"]);
        g.fetch(&b, FetchOpts::default(), out.clone(), Default::default())
            .unwrap();
        assert!(Git2Backend.would_conflict(&b).unwrap());
        assert_eq!(
            g.pull(&b, PullMode::Rebase, out.clone(), Default::default()),
            Err(GitError::Conflict)
        );
        let s = Git2Backend.status(&b, StatusOpts::default()).unwrap();
        assert_eq!(s.head.state, crate::RepoState::Clean, "rebase aborted");
        assert_eq!(
            std::fs::read_to_string(b.workdir.join("n.md")).unwrap(),
            "B\n2\n3\n",
            "local commit kept"
        );
    }

    #[test]
    fn auth_failure_and_cancel() {
        let Some(g) = git() else { return };
        let (_d, a, _) = remote_and_clones(&g);
        let (out, log) = sink();
        // An https remote with no credentials configured fails without a
        // prompt (GIT_TERMINAL_PROMPT=0). Nothing listens on port 9.
        sh(
            &a.workdir,
            &["remote", "set-url", "origin", "http://127.0.0.1:9/x.git"],
        );
        let err = g
            .fetch(&a, FetchOpts::default(), out.clone(), Default::default())
            .unwrap_err();
        assert!(matches!(err, GitError::Git(_)), "{err:?}");
        assert!(log.lock().unwrap().len() >= 2, "stderr streamed");
        let cancel = CancellationToken::default();
        cancel.cancel();
        assert_eq!(
            g.fetch(&a, FetchOpts::default(), out, cancel),
            Err(GitError::Cancelled)
        );
    }
}
