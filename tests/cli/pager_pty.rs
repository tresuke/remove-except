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
        Self::start_with_size(root, extra, piped_patterns, 24, 100)
    }

    fn start_with_size(
        root: &std::path::Path,
        extra: &[&str],
        piped_patterns: bool,
        rows: u16,
        columns: u16,
    ) -> Self {
        Self::start_configured(root, extra, piped_patterns, (rows, columns), |_| {})
    }

    fn start_configured(
        root: &std::path::Path,
        extra: &[&str],
        piped_patterns: bool,
        size: (u16, u16),
        configure: impl FnOnce(&mut Command),
    ) -> Self {
        let mut master = -1;
        let mut slave = -1;
        let size = libc::winsize {
            ws_row: size.0,
            ws_col: size.1,
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
        configure(&mut command);
        let control_fd = if piped_patterns { 1 } else { 0 };
        // Only async-signal-safe libc calls run between fork and exec.
        unsafe {
            command.pre_exec(move || {
                if libc::setsid() == -1 || libc::ioctl(control_fd, libc::TIOCSCTTY, 0) == -1 {
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
            if self.output.contains(marker)
                || console::strip_ansi_codes(&self.output).contains(marker)
            {
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

#[test]
fn pty_short_delete_section_tabs_back_to_keep_and_labels_both() {
    let root = fixture();
    let mut session = Session::start(
        root.path(),
        &["--pager", "always", "remove-*", "*path-end.txt"],
        false,
    );
    session.wait_for("c finish");
    session.output.clear();
    session.send(b"\t");
    session.wait_for("KEEP | lines");
    session.output.clear();
    session.send(b"\t");
    session.wait_for("KEEP + DELETE | lines");
    session.output.clear();
    session.send(b"\t");
    session.wait_for("KEEP | lines");
    session.output.clear();
    session.send(b"\x1b[Z");
    session.wait_for("KEEP + DELETE | lines");
    session.send(b"q");
    session.exit(3);
    root.child("remove-00.txt").assert("remove");
}

#[test]
fn pty_grapheme_paths_wrap_into_bounded_complete_frames() {
    let root = fixture();
    let name = format!("zz-{}-cluster-end.txt", "1\u{fe0f}\u{20e3}".repeat(20));
    root.child(&name).write_str("remove").unwrap();
    let mut session = Session::start(root.path(), &["--dry-run", "--pager", "always"], false);
    session.wait_for("c finish");
    session.resize(24, 40);
    session.output.clear();
    session.send(b"\x1b[F");
    session.wait_for("cluster-end.txt");
    session.wait_for("cancel\r\n");
    let clean = console::strip_ansi_codes(&session.output);
    for line in clean.lines() {
        assert!(
            console::measure_text_width(line.trim_end_matches('\r')) <= 39,
            "{line:?}"
        );
    }
    // Each redraw ends after exactly rows-1 newlines, with no implicit line overflow.
    let mut complete_frames = 0;
    for frame in session.output.split("\x1b[2J").skip(1) {
        if frame.contains("cancel") {
            assert_eq!(frame.bytes().filter(|byte| *byte == b'\n').count(), 23);
            complete_frames += 1;
        }
    }
    assert!(complete_frames > 0, "No complete viewer frame was captured");
    session.send(b"c");
    session.exit(0);
    root.child(&name).assert("remove");
}

#[test]
fn pty_oversized_cluster_falls_back_only_before_auto_startup() {
    let root = fixture();
    root.child("\u{17d8}.txt").write_str("remove").unwrap();
    for (mode, expected) in [("auto", 0), ("always", 1)] {
        let mut session =
            Session::start_with_size(root.path(), &["--dry-run", "--pager", mode], false, 200, 3);
        session.wait_for("A grapheme cluster is wider than the available viewer row");
        if mode == "auto" {
            session.wait_for("Using normal output.");
            session.wait_for("PREVIEW ONLY");
        }
        session.exit(expected);
        root.child("\u{17d8}.txt").assert("remove");
    }
    let mut session = Session::start(root.path(), &["--pager", "always"], false);
    session.wait_for("c finish");
    session.output.clear();
    session.resize(200, 3);
    session.wait_for("A grapheme cluster is wider than the available viewer row");
    assert!(!session.output.contains("Using normal output."));
    session.exit(1);
    root.child("\u{17d8}.txt").assert("remove");
}

#[test]
fn pty_colors_viewer_and_recap_but_no_color_keeps_them_plain() {
    for colored in [true, false] {
        let root = fixture();
        let mut session = Session::start_configured(
            root.path(),
            &["--pager", "always"],
            false,
            (24, 100),
            |command| {
                command.env_remove("CLICOLOR_FORCE").env_remove("NO_COLOR");
                if colored {
                    command.env("CLICOLOR_FORCE", "1");
                } else {
                    command.env("NO_COLOR", "1");
                }
            },
        );
        session.wait_for("c finish");
        session.output.clear();
        session.send(b"\x1b[F");
        session.wait_for("DELETE | lines");
        if colored {
            assert!(
                session
                    .output
                    .contains("\x1b[31m\x1b[1mDELETE\x1b[0m | lines")
            );
        } else {
            assert!(!session.output.contains("\x1b[31m"));
            assert!(!session.output.contains("\x1b[1m"));
        }
        session.output.clear();
        session.send(b"c");
        session.wait_for("[y/N]");
        if colored {
            assert!(
                session
                    .output
                    .contains("\x1b[33m\x1b[1mReview before deletion:\x1b[0m")
            );
            assert!(session.output.contains("\x1b[32m\x1b[1mKEEP:\x1b[0m"));
            assert!(session.output.contains("\x1b[31m\x1b[1mDELETE:\x1b[0m"));
        } else {
            let recap = &session.output[session.output.find("Review before deletion:").unwrap()..];
            assert!(!recap.contains('\x1b'));
        }
        session.send(b"n\n");
        session.exit(3);
        root.child("remove-00.txt").assert("remove");
    }
}

#[test]
fn pty_recap_color_detection_is_independent_of_stdout() {
    let root = fixture();
    let mut session = Session::start_configured(
        root.path(),
        &["--pager", "never"],
        false,
        (24, 100),
        |command| {
            command
                .env_remove("NO_COLOR")
                .env("CLICOLOR_FORCE", "1")
                .stdout(Stdio::null());
        },
    );
    session.wait_for("[y/N]");
    assert!(
        session
            .output
            .contains("\x1b[33m\x1b[1mReview before deletion:\x1b[0m")
    );
    session.send(b"n\n");
    session.exit(3);
    root.child("remove-00.txt").assert("remove");

    let capture = assert_fs::NamedTempFile::new("stderr.txt").unwrap();
    let mut session = Session::start_configured(
        root.path(),
        &["--pager", "always"],
        false,
        (24, 100),
        |command| {
            command
                .env_remove("NO_COLOR")
                .env("CLICOLOR_FORCE", "1")
                .stderr(File::create(capture.path()).unwrap());
        },
    );
    session.wait_for("c finish");
    session.output.clear();
    session.send(b"\x1b[F");
    session.wait_for("DELETE | lines");
    assert!(
        session
            .output
            .contains("\x1b[31m\x1b[1mDELETE\x1b[0m | lines")
    );
    session.send(b"c");
    let start = Instant::now();
    loop {
        let stderr = std::fs::read_to_string(capture.path()).unwrap();
        if stderr.contains("[y/N]") {
            assert!(stderr.contains("Review before deletion:"));
            assert!(!stderr.contains('\x1b'));
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(10), "{stderr}");
        std::thread::sleep(Duration::from_millis(20));
    }
    session.send(b"n\n");
    session.exit(3);
    root.child("remove-00.txt").assert("remove");
}
