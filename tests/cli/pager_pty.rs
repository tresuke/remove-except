use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use assert_fs::prelude::*;

struct Session {
    child: Child,
    master: File,
    slave: File,
    original: libc::termios,
    output: String,
}

impl Session {
    fn start(root: &std::path::Path, extra: &[&str], piped_patterns: bool) -> Self {
        let mut master = -1;
        let mut slave = -1;
        let size = libc::winsize {
            ws_row: 24,
            ws_col: 100,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        // openpty writes two new owned descriptors and reads the initialized size.
        assert_eq!(
            unsafe {
                libc::openpty(
                    &mut master,
                    &mut slave,
                    std::ptr::null_mut(),
                    std::ptr::null(),
                    &size,
                )
            },
            0,
            "openpty: {}",
            std::io::Error::last_os_error()
        );
        // Each descriptor returned by openpty is transferred to one File.
        let master = unsafe { File::from_raw_fd(master) };
        let slave = unsafe { File::from_raw_fd(slave) };
        let mut original = std::mem::MaybeUninit::uninit();
        // tcgetattr initializes original on success.
        assert_eq!(
            unsafe { libc::tcgetattr(slave.as_raw_fd(), original.as_mut_ptr()) },
            0
        );
        let original = unsafe { original.assume_init() };
        let mut command = Command::new(env!("CARGO_BIN_EXE_remove-except"));
        command
            .current_dir(root)
            .args(extra)
            .env("TERM", "xterm")
            .stdout(Stdio::from(slave.try_clone().unwrap()))
            .stderr(Stdio::from(slave.try_clone().unwrap()));
        if piped_patterns {
            command.arg("--patterns-from-stdin").stdin(Stdio::piped());
        } else {
            command
                .arg("keep.txt")
                .stdin(Stdio::from(slave.try_clone().unwrap()));
        }
        // Only async-signal-safe libc calls run between fork and exec. stdout is the PTY slave.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 || libc::ioctl(1, libc::TIOCSCTTY, 0) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = command.spawn().unwrap();
        if piped_patterns {
            child
                .stdin
                .take()
                .unwrap()
                .write_all(b"keep.txt\n")
                .unwrap();
        }
        Self {
            child,
            master,
            slave,
            original,
            output: String::new(),
        }
    }

    fn send(&mut self, keys: &[u8]) {
        self.master.write_all(keys).unwrap();
        self.master.flush().unwrap();
    }

    fn wait_for(&mut self, marker: &str) {
        let start = Instant::now();
        loop {
            if self.output.contains(marker) {
                return;
            }
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "Missing {marker:?}: {}",
                self.output
            );
            let mut descriptor = libc::pollfd {
                fd: self.master.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            // descriptor references one valid PTY descriptor.
            let ready = unsafe { libc::poll(&mut descriptor, 1, 100) };
            assert!(ready >= 0);
            if ready > 0 {
                let mut bytes = [0; 16384];
                match self.master.read(&mut bytes) {
                    Ok(0) => panic!("PTY closed before {marker:?}: {}", self.output),
                    Ok(read) => self
                        .output
                        .push_str(&String::from_utf8_lossy(&bytes[..read])),
                    Err(error) => panic!(
                        "PTY read failed before {marker:?}: {error}; {}",
                        self.output
                    ),
                }
            }
        }
    }

    fn exit(&mut self, expected: i32) {
        let start = Instant::now();
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert_eq!(status.code(), Some(expected), "{}", self.output);
                break;
            }
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "Process did not exit: {}",
                self.output
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        let mut restored = std::mem::MaybeUninit::uninit();
        // The parent retains a valid slave descriptor for checking restoration.
        assert_eq!(
            unsafe { libc::tcgetattr(self.slave.as_raw_fd(), restored.as_mut_ptr()) },
            0
        );
        let restored = unsafe { restored.assume_init() };
        assert_eq!(restored.c_lflag, self.original.c_lflag);
        assert_eq!(restored.c_iflag, self.original.c_iflag);
        assert_eq!(restored.c_oflag, self.original.c_oflag);
    }

    fn resize(&self, rows: u16, columns: u16) {
        let size = libc::winsize {
            ws_row: rows,
            ws_col: columns,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        // TIOCSWINSZ reads the initialized winsize from the provided pointer.
        assert_eq!(
            unsafe { libc::ioctl(self.master.as_raw_fd(), libc::TIOCSWINSZ, &size) },
            0
        );
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if self.child.try_wait().unwrap().is_none() {
            self.child.kill().unwrap();
            self.child.wait().unwrap();
        }
    }
}

fn fixture() -> assert_fs::TempDir {
    let root = assert_fs::TempDir::new().unwrap();
    root.child("keep.txt").write_str("keep").unwrap();
    root.child(format!("{}-path-end.txt", "日本語".repeat(12)))
        .write_str("remove")
        .unwrap();
    for index in 0..60 {
        root.child(format!("remove-{index:02}.txt"))
            .write_str("remove")
            .unwrap();
    }
    root
}

#[test]
fn pty_navigation_input_discard_and_independent_confirmation() {
    let root = fixture();
    let mut session = Session::start(root.path(), &["--pager", "always"], false);
    session.wait_for("c finish");
    session.send(b"y\n");
    std::thread::sleep(Duration::from_millis(150));
    assert!(session.child.try_wait().unwrap().is_none());
    root.child("remove-00.txt").assert("remove");
    session.send(b"\x1b[F");
    session.output.clear();
    session.wait_for("DELETE | lines");
    session.send(b"\x1b[H\t\x1b[Z\x1b[6~\x1b[5~\x1b[B\x1b[A");
    session.resize(20, 70);
    session.output.clear();
    session.wait_for("c finish");
    // One write queues the entire answer before the viewer-to-confirmation transition.
    session.send(b"cyes\n");
    session.wait_for("[y/N]");
    std::thread::sleep(Duration::from_millis(200));
    assert!(session.child.try_wait().unwrap().is_none());
    root.child("remove-00.txt").assert("remove");
    session.send(b"n\n");
    session.exit(3);
    root.child("remove-00.txt").assert("remove");
}

#[test]
fn pty_confirmation_uses_terminal_after_stdin_patterns() {
    let root = fixture();
    let mut session = Session::start(root.path(), &["--pager", "always"], true);
    session.wait_for("c finish");
    session.send(b"c");
    session.wait_for("[y/N]");
    assert!(session.output.contains("Review before deletion:"));
    session.send(b"yes\n");
    session.exit(0);
    root.child("keep.txt").assert("keep");
    assert!(!root.child("remove-00.txt").path().exists());
}

#[test]
fn pty_cancellation_and_dry_run_restore_terminal_without_deleting() {
    for key in [b"q".as_slice(), b"\x1b".as_slice(), b"\x03".as_slice()] {
        let root = fixture();
        let mut session = Session::start(root.path(), &["--pager", "always"], false);
        session.wait_for("c finish");
        session.send(key);
        session.exit(3);
        root.child("remove-00.txt").assert("remove");
    }
    let root = fixture();
    let mut session = Session::start(root.path(), &["--dry-run"], false);
    session.wait_for("PREVIEW ONLY");
    session.wait_for("c finish");
    session.send(b"c");
    session.exit(0);
    root.child("remove-00.txt").assert("remove");
}

#[test]
fn pty_resize_failure_stops_without_deleting_and_restores_input() {
    let root = fixture();
    let mut session = Session::start(root.path(), &["--pager", "always"], false);
    session.wait_for("c finish");
    session.resize(2, 2);
    session.exit(1);
    root.child("remove-00.txt").assert("remove");
}

#[test]
fn pty_no_removals_and_flat_summary_finish_without_confirmation() {
    for extra in [
        vec!["--pager", "always", "."],
        vec!["--pager", "always", "--dry-run", "--layout", "flat"],
        vec!["--pager", "always", "--dry-run", "--summary", "both"],
    ] {
        let root = fixture();
        let mut session = Session::start(root.path(), &extra, false);
        session.wait_for("c finish");
        session.send(b"c");
        session.exit(0);
        root.child("remove-00.txt").assert("remove");
    }
}
