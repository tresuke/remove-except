use assert_cmd::Command;
use assert_fs::prelude::*;

mod support;

#[test]
fn completion_generation_does_not_read_stdin_or_touch_files() {
    use std::process::{Command as Process, Stdio};
    use std::time::{Duration, Instant};

    let temp = assert_fs::TempDir::new().unwrap();
    let sentinel = temp.child("sentinel.txt");
    sentinel.write_str("untouched").unwrap();
    for (args, code, script_start) in [
        (
            &["--generate-completion", "bash"][..],
            0,
            "_remove__except()",
        ),
        (
            &["--generate-completion=powershell", "--"],
            0,
            "Register-ArgumentCompleter",
        ),
        (
            &["--generate-completion", "bash", "--patterns-from-stdin"],
            2,
            "",
        ),
        (
            &["--patterns-from-stdin", "--generate-completion", "bash"],
            2,
            "",
        ),
    ] {
        let mut child = Process::new(env!("CARGO_BIN_EXE_remove-except"))
            .current_dir(temp.path())
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while child.try_wait().unwrap().is_none() {
            if Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("{args:?} did not exit while stdin was open");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let output = child.wait_with_output().unwrap();
        drop(stdin);
        assert_eq!(output.status.code(), Some(code), "{args:?}: {output:?}");
        if code == 0 {
            assert!(output.stdout.starts_with(script_start.as_bytes()));
            assert!(output.stderr.is_empty());
        } else {
            assert!(output.stdout.is_empty());
        }
        sentinel.assert("untouched");
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 1);
    }
}

#[test]
fn completion_is_exclusive_with_execution_arguments() {
    let temp = assert_fs::TempDir::new().unwrap();
    let sentinel = temp.child("sentinel.txt");
    sentinel.write_str("untouched").unwrap();
    for other in [
        &["--force"][..],
        &["-n"],
        &["--root", "."],
        &["--format", "human"],
        &["--pager", "auto"],
        &["--layout", "tree"],
        &["--summary", "keep"],
        &["--sort-by-type"],
        &["--require-match"],
        &["--max-delete-items", "1"],
        &["--modified-since", "2026-01-01"],
        &["--modified-before", "2026-01-01"],
        &["--size-at-least", "0"],
        &["--size-below", "1"],
        &["absent.txt"],
        &["--generate-completion", "powershell"],
    ] {
        let completion = ["--generate-completion", "bash"];
        for args in [[&completion[..], other], [other, &completion]].map(|parts| parts.concat()) {
            Command::cargo_bin("remove-except")
                .unwrap()
                .current_dir(temp.path())
                .args(&args)
                .assert()
                .code(2)
                .stdout("");
        }
    }
    for args in [
        &["--generate-completion"][..],
        &["--generate-completion", "zsh"],
        &["--generate-completion="],
        &["--generate-completion", "BASH"],
        &["--generate-completion", "bash", "--", "absent.txt"],
    ] {
        Command::cargo_bin("remove-except")
            .unwrap()
            .current_dir(temp.path())
            .args(args)
            .assert()
            .code(2)
            .stdout("");
    }
    sentinel.assert("untouched");
}

#[test]
fn completion_preserves_standard_help_version_and_separator() {
    let temp = assert_fs::TempDir::new().unwrap();
    for info in ["--help", "-h", "--version", "-V"] {
        let expected = Command::cargo_bin("remove-except")
            .unwrap()
            .arg(info)
            .assert()
            .success()
            .stderr("")
            .get_output()
            .stdout
            .clone();
        for args in [
            [info, "--generate-completion", "bash"],
            ["--generate-completion", "bash", info],
        ] {
            Command::cargo_bin("remove-except")
                .unwrap()
                .args(args)
                .assert()
                .success()
                .stderr("")
                .stdout(expected.clone());
        }
    }
    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args([
            "--dry-run",
            "--format",
            "json",
            "--",
            "--generate-completion",
        ])
        .assert()
        .success()
        .get_output()
        .clone();
    assert_eq!(
        parse_json_stdout(&output)["selection"]["patterns"],
        serde_json::json!(["--generate-completion"])
    );
    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--generate-completion", "bash", "--"])
        .assert()
        .success()
        .get_output()
        .clone();
    assert!(output.stdout.starts_with(b"_remove__except()"));
}

#[test]
fn completion_reports_closed_stdout_as_runtime_error() {
    use std::process::{Command as Process, Stdio};

    for (shell, closed_stderr) in [
        ("bash", false),
        ("powershell", false),
        ("bash", true),
        ("powershell", true),
    ] {
        let (reader, writer) = std::io::pipe().unwrap();
        drop(reader);
        let mut command = Process::new(env!("CARGO_BIN_EXE_remove-except"));
        command
            .args(["--generate-completion", shell])
            .stdout(Stdio::from(writer));
        if closed_stderr {
            let (reader, writer) = std::io::pipe().unwrap();
            drop(reader);
            command.stderr(Stdio::from(writer));
        }
        let output = command.output().unwrap();
        assert_eq!(output.status.code(), Some(1), "{shell}: {output:?}");
        if !closed_stderr {
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(stderr.starts_with("Error: could not write completion script:"));
            assert!(!stderr.contains("panicked"));
        }
    }
}

#[test]
fn modification_time_json_combines_patterns_and_preserves_input_order() {
    use std::time::{Duration, SystemTime};
    let temp = assert_fs::TempDir::new().unwrap();
    for (name, seconds) in [
        ("dir/keep.txt", 1_700_000_000),
        ("dir/old.txt", 1_699_999_999),
    ] {
        let file = temp.child(name);
        file.write_str("data").unwrap();
        let handle = std::fs::OpenOptions::new()
            .write(true)
            .open(file.path())
            .unwrap();
        handle
            .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(seconds))
            .unwrap();
    }
    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args([
            "--dry-run",
            "--format",
            "json",
            "--patterns-from-stdin",
            "--modified-since",
            "2023-11-14T22:13:20Z",
            "dir",
        ])
        .write_stdin("other\nother\n")
        .assert()
        .success()
        .get_output()
        .clone();
    let plan = parse_json_stdout(&output);
    assert_eq!(
        plan["selection"],
        serde_json::json!({
            "patterns": ["dir", "other", "other"],
            "modified_since": "2023-11-14T22:13:20Z",
            "modified_before": null,
            "size_at_least": null,
            "size_below": null
        })
    );
    assert_eq!(plan["direct_match_count"], 1);
    assert_eq!(relative_paths(&plan, "keep_items"), ["dir", "dir/keep.txt"]);
    assert_eq!(relative_paths(&plan, "delete_items"), ["dir/old.txt"]);
    temp.child("dir/old.txt").assert("data");
}

#[test]
fn modification_time_only_and_local_day_are_resolved_in_child_process() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("file.txt").write_str("data").unwrap();
    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .env("TZ", "Asia/Tokyo")
        .args([
            "--dry-run",
            "--format",
            "json",
            "--modified-since",
            "2026-11-10",
            "--modified-before",
            "2026-11-11",
        ])
        .assert()
        .success()
        .get_output()
        .clone();
    let plan = parse_json_stdout(&output);
    assert_eq!(
        plan["selection"],
        serde_json::json!({
            "patterns": [],
            "modified_since": "2026-11-09T15:00:00Z",
            "modified_before": "2026-11-10T15:00:00Z",
            "size_at_least": null,
            "size_below": null
        })
    );
    temp.child("file.txt").assert("data");
}

#[test]
fn size_only_json_reports_integer_bounds_and_keeps_matching_files() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("empty").write_str("").unwrap();
    temp.child("small").write_str("x").unwrap();
    temp.child("lower").write_str("ab").unwrap();
    temp.child("inside").write_str("abcd").unwrap();
    temp.child("upper").write_str("abcde").unwrap();
    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args([
            "--dry-run",
            "--format",
            "json",
            "--size-at-least",
            "0002",
            "--size-below",
            "5",
        ])
        .assert()
        .success()
        .get_output()
        .clone();
    let plan = parse_json_stdout(&output);
    assert_eq!(
        plan["selection"],
        serde_json::json!({
            "patterns": [],
            "modified_since": null,
            "modified_before": null,
            "size_at_least": 2,
            "size_below": 5
        })
    );
    assert_eq!(plan["schema_version"], 2);
    assert_eq!(plan["direct_match_count"], 2);
    assert_eq!(relative_paths(&plan, "keep_items"), ["inside", "lower"]);
    assert_eq!(
        relative_paths(&plan, "delete_items"),
        ["empty", "small", "upper"]
    );
}

#[test]
fn size_bounds_accept_u64_max_with_empty_stdin_patterns() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("file.txt").write_str("data").unwrap();
    for (option, lower, upper, matches) in [
        ("--size-at-least", Some(u64::MAX), None, 0),
        ("--size-below", None, Some(u64::MAX), 1),
    ] {
        let output = Command::cargo_bin("remove-except")
            .unwrap()
            .current_dir(temp.path())
            .args([
                "--dry-run",
                "--format",
                "json",
                "--patterns-from-stdin",
                option,
                "18446744073709551615",
            ])
            .write_stdin("")
            .assert()
            .success()
            .get_output()
            .clone();
        let plan = parse_json_stdout(&output);
        assert_eq!(
            plan["selection"],
            serde_json::json!({
                "patterns": [],
                "modified_since": null,
                "modified_before": null,
                "size_at_least": lower,
                "size_below": upper
            })
        );
        assert_eq!(plan["direct_match_count"], matches);
        temp.child("file.txt").assert("data");
    }
}

#[test]
fn size_and_modification_time_json_preserves_combined_selection() {
    use std::time::{Duration, SystemTime};

    let temp = assert_fs::TempDir::new().unwrap();
    let start = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    for (name, contents, modified) in [
        ("dir/match.txt", "keep", start),
        ("dir/too-small.txt", "x", start),
        ("dir/too-large.txt", "large", start),
        ("dir/too-old.txt", "old!", start - Duration::from_secs(1)),
    ] {
        let file = temp.child(name);
        file.write_str(contents).unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(file.path())
            .unwrap()
            .set_modified(modified)
            .unwrap();
    }

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args([
            "--dry-run",
            "--format",
            "json",
            "--modified-since",
            "2023-11-14T22:13:20Z",
            "--size-at-least",
            "2",
            "--size-below",
            "5",
            "dir",
        ])
        .assert()
        .success()
        .get_output()
        .clone();
    let plan = parse_json_stdout(&output);
    assert_eq!(
        plan["selection"],
        serde_json::json!({
            "patterns": ["dir"],
            "modified_since": "2023-11-14T22:13:20Z",
            "modified_before": null,
            "size_at_least": 2,
            "size_below": 5
        })
    );
    assert_eq!(plan["direct_match_count"], 1);
    assert_eq!(
        relative_paths(&plan, "keep_items"),
        ["dir", "dir/match.txt"]
    );
    assert_eq!(
        relative_paths(&plan, "delete_items"),
        ["dir/too-large.txt", "dir/too-old.txt", "dir/too-small.txt"]
    );
}

#[test]
fn size_filter_with_no_matches_warns_and_preserves_everything_in_dry_run() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("small.txt").write_str("small").unwrap();
    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--dry-run", "--size-at-least", "100"])
        .assert()
        .success()
        .get_output()
        .clone();
    assert!(!output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("No items matched the keep conditions")
    );
    temp.child("small.txt").assert("small");
}

#[test]
fn invalid_size_values_and_ranges_never_output_or_delete() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("file.txt").write_str("data").unwrap();
    for value in ["", "+1", "-1", " 1", "1 ", "1KiB", "18446744073709551616"] {
        let output = Command::cargo_bin("remove-except")
            .unwrap()
            .current_dir(temp.path())
            .args(["--dry-run", "--size-at-least", value])
            .assert()
            .code(2)
            .get_output()
            .clone();
        assert!(output.stdout.is_empty(), "{value:?}");
        temp.child("file.txt").assert("data");
    }

    for args in [
        vec!["--dry-run", "--size-below", "0"],
        vec!["--dry-run", "--size-at-least", "5", "--size-below", "5"],
        vec!["--dry-run", "--size-at-least", "6", "--size-below", "5"],
    ] {
        let output = Command::cargo_bin("remove-except")
            .unwrap()
            .current_dir(temp.path())
            .args(args)
            .assert()
            .code(2)
            .get_output()
            .clone();
        assert!(output.stdout.is_empty());
        temp.child("file.txt").assert("data");
    }

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--force", "--patterns-from-stdin", "--size-below", "0"])
        .write_stdin("file.txt\n")
        .assert()
        .code(2)
        .get_output()
        .clone();
    assert!(output.stdout.is_empty());
    temp.child("file.txt").assert("data");
}

#[test]
fn invalid_size_range_exits_while_stdin_is_still_open() {
    use std::process::Stdio;
    use std::time::{Duration, Instant};

    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("file.txt").write_str("data").unwrap();
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_remove-except"))
        .current_dir(temp.path())
        .args(["--force", "--patterns-from-stdin", "--size-below", "0"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    // Keep the pipe open until the child exits; wait_with_output would close it.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }
            result => {
                if let Err(error) = child.kill() {
                    // The child may have exited between polling and killing it.
                    assert!(
                        matches!(child.try_wait(), Ok(Some(_))),
                        "failed to terminate child after {result:?}: {error}"
                    );
                }
                child.wait().expect("failed to reap child");
                panic!("CLI did not exit promptly with stdin still open: {result:?}");
            }
        }
    }

    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("File-size upper bound must be greater than zero")
    );
    temp.child("file.txt").assert("data");
}

#[test]
fn size_bounds_are_shown_in_preview_and_confirmation_recap() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt").write_str("keep").unwrap();
    temp.child("delete.txt").write_str("x").unwrap();
    for (bounds, expected_lower, expected_upper) in [
        (
            vec!["--size-at-least", "3", "--size-below", "5"],
            "3 bytes",
            "5 bytes",
        ),
        (vec!["--size-at-least", "3"], "3 bytes", "unbounded"),
        (vec!["--size-below", "3"], "unbounded", "3 bytes"),
    ] {
        let output = Command::cargo_bin("remove-except")
            .unwrap()
            .current_dir(temp.path())
            .args(["--pager", "never"])
            .args(bounds)
            .write_stdin("n\n")
            .assert()
            .code(3)
            .get_output()
            .clone();
        let stdout = String::from_utf8(output.stdout).unwrap();
        let stderr = String::from_utf8(output.stderr).unwrap();
        for text in [
            format!("Size at least (inclusive): {expected_lower}"),
            format!("Size below (exclusive): {expected_upper}"),
        ] {
            assert!(stdout.lines().any(|line| line == text), "{text}");
            assert!(stderr.lines().any(|line| line == text), "{text}");
        }
        temp.child("keep.txt").assert("keep");
        temp.child("delete.txt").assert("x");
    }
}

#[test]
fn invalid_modification_times_never_output_or_delete() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("file.txt").write_str("data").unwrap();
    for value in [
        "2026-01-01T00:00:00.0Z",
        "2026-01-01T00:00:00.001Z",
        "2026-01-01T00:00:00.000000001Z",
        "2026-01-01t00:00Z",
        "2026-01-01T00:00z",
        "2026-01-01T24:00Z",
        "2026-01-01T00:00:60Z",
        "2026-01-01T00:00-00:00",
        "2026-01-01Z",
        "2026-02-30",
        "2026-01-01T00:00:00Z[UTC]",
        "2026-01-01T00",
        "2026-01-01T00:00:00+09:00:00",
        " 2026-01-01",
        "2026-01-01 ",
        "1600-12-31T23:59:59Z",
        "1599-12-31T23:59:59Z",
        "9999-01-01T00:00:00Z",
        "1601-01-01T00:00:00+09:00",
    ] {
        let output = Command::cargo_bin("remove-except")
            .unwrap()
            .current_dir(temp.path())
            .env("TZ", "UTC")
            .args(["--force", "--modified-since", value])
            .assert()
            .code(2)
            .get_output()
            .clone();
        assert!(output.stdout.is_empty(), "{value}");
        temp.child("file.txt").assert("data");
    }
}

#[test]
fn invalid_or_repeated_modification_bounds_never_output_or_delete() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("file.txt").write_str("data").unwrap();
    for args in [
        [
            "--modified-since",
            "2026-01-01T00:00:00Z",
            "--modified-before",
            "2026-01-01T09:00:00+09:00",
        ],
        [
            "--modified-since",
            "2026-01-02T00:00:00Z",
            "--modified-before",
            "2026-01-01T00:00:00Z",
        ],
        [
            "--modified-since",
            "2026-01-01T00:00:00Z",
            "--modified-since",
            "2026-01-02T00:00:00Z",
        ],
        [
            "--modified-before",
            "2026-01-02T00:00:00Z",
            "--modified-before",
            "2026-01-03T00:00:00Z",
        ],
    ] {
        let output = Command::cargo_bin("remove-except")
            .unwrap()
            .current_dir(temp.path())
            .args(["--force", "--format", "json"])
            .args(args)
            .assert()
            .code(2)
            .get_output()
            .clone();
        assert!(output.stdout.is_empty(), "{args:?}");
        assert!(!output.stderr.is_empty(), "{args:?}");
        temp.child("file.txt").assert("data");
    }
}

#[test]
fn timezone_errors_and_ambiguous_local_times_fail_safely() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("file.txt").write_str("data").unwrap();
    for (zone, value, code) in [
        ("not-a-valid-zone", "2026-01-01", 1),
        ("", "2026-01-01", 1),
        ("America/New_York", "2026-03-08T02:30", 2),
        ("America/New_York", "2026-11-01T01:30", 2),
        ("America/Sao_Paulo", "2018-11-04", 2),
        ("America/Havana", "2020-11-01", 2),
    ] {
        let output = Command::cargo_bin("remove-except")
            .unwrap()
            .current_dir(temp.path())
            .env("TZ", zone)
            .args(["--force", "--modified-since", value])
            .assert()
            .code(code)
            .get_output()
            .clone();
        assert!(output.stdout.is_empty(), "{zone:?} {value}");
        temp.child("file.txt").assert("data");
    }
    for zone in ["", "not-a-valid-zone"] {
        Command::cargo_bin("remove-except")
            .unwrap()
            .current_dir(temp.path())
            .env("TZ", zone)
            .args(["--dry-run", "--modified-since", "1601-01-01T00:00:00Z"])
            .assert()
            .success();
    }
}

#[test]
fn local_days_follow_daylight_saving_transitions() {
    let temp = assert_fs::TempDir::new().unwrap();
    for (since, before, expected_since, expected_before) in [
        (
            "2026-03-08",
            "2026-03-09",
            "2026-03-08T05:00:00Z",
            "2026-03-09T04:00:00Z",
        ),
        (
            "2026-11-01",
            "2026-11-02",
            "2026-11-01T04:00:00Z",
            "2026-11-02T05:00:00Z",
        ),
    ] {
        let output = Command::cargo_bin("remove-except")
            .unwrap()
            .current_dir(temp.path())
            .env("TZ", "America/New_York")
            .args([
                "--dry-run",
                "--format",
                "json",
                "--modified-since",
                since,
                "--modified-before",
                before,
            ])
            .assert()
            .success()
            .get_output()
            .clone();
        let plan = parse_json_stdout(&output);
        assert_eq!(plan["selection"]["modified_since"], expected_since);
        assert_eq!(plan["selection"]["modified_before"], expected_before);
    }
}

#[test]
fn modification_time_with_empty_or_invalid_stdin_and_recap() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("file.txt").write_str("data").unwrap();
    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args([
            "--dry-run",
            "--format",
            "json",
            "--patterns-from-stdin",
            "--modified-before",
            "1601-01-01T00:00:00Z",
        ])
        .write_stdin("\n")
        .assert()
        .success()
        .get_output()
        .clone();
    let plan = parse_json_stdout(&output);
    assert_eq!(plan["selection"]["patterns"], serde_json::json!([]));
    assert_eq!(plan["direct_match_count"], 0);

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args([
            "--force",
            "--patterns-from-stdin",
            "--modified-since",
            "1601-01-01T00:00:00Z",
        ])
        .write_stdin(vec![0xff])
        .assert()
        .code(2)
        .get_output()
        .clone();
    assert!(output.stdout.is_empty());
    temp.child("file.txt").assert("data");

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args([
            "--pager",
            "never",
            "--modified-before",
            "1601-01-01T00:00:00Z",
        ])
        .write_stdin("n\n")
        .assert()
        .code(3)
        .get_output()
        .clone();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    for text in [
        "Modified since (inclusive): unbounded",
        "Modified before (exclusive): 1601-01-01T00:00:00Z",
    ] {
        assert!(stdout.contains(text), "{text}");
        assert!(stderr.contains(text), "{text}");
    }
    assert!(stderr.contains("No items matched the keep conditions"));
    temp.child("file.txt").assert("data");
}

#[test]
fn modification_time_summary_and_force_no_match_follow_keep_conditions() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("file.txt").write_str("data").unwrap();
    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args([
            "--dry-run",
            "--pager",
            "never",
            "--summary",
            "keep",
            "--modified-before",
            "1601-01-01T00:00:00Z",
        ])
        .assert()
        .success()
        .get_output()
        .clone();
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("Modified before (exclusive): 1601-01-01T00:00:00Z"));
    assert!(text.contains("0 direct matches"));
    assert!(!text.contains("direct pattern matches"));
    temp.child("file.txt").assert("data");
    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--force", "--modified-before", "1601-01-01T00:00:00Z"])
        .assert()
        .success()
        .get_output()
        .clone();
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("No items matched the keep conditions")
    );
    assert!(!temp.path().join("file.txt").exists());
}

#[test]
fn time_filter_does_not_keep_an_explicit_empty_directory() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("empty").create_dir_all().unwrap();
    temp.child("keep.txt").write_str("inside").unwrap();
    Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args([
            "--force",
            "--modified-since",
            "1601-01-01T00:00:00Z",
            "empty",
            "keep.txt",
        ])
        .assert()
        .success();
    assert!(!temp.path().join("empty").exists());
    temp.child("keep.txt").assert("inside");
}

#[test]
fn time_filter_does_not_keep_an_explicit_symlink() {
    let temp = assert_fs::TempDir::new().unwrap();
    let outside = assert_fs::TempDir::new().unwrap();
    outside.child("target.txt").write_str("outside").unwrap();
    temp.child("keep.txt").write_str("inside").unwrap();
    let target = outside.path().join("target.txt");
    let link = temp.path().join("link");
    #[cfg(unix)]
    let link_result = std::os::unix::fs::symlink(&target, &link);
    #[cfg(windows)]
    let link_result = std::os::windows::fs::symlink_file(&target, &link);
    link_result.expect("creating the symbolic-link fixture is required for this test");
    Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args([
            "--force",
            "--modified-since",
            "1601-01-01T00:00:00Z",
            "link",
            "keep.txt",
        ])
        .assert()
        .success();
    assert_eq!(
        std::fs::symlink_metadata(&link).unwrap_err().kind(),
        std::io::ErrorKind::NotFound
    );
    temp.child("keep.txt").assert("inside");
    outside.child("target.txt").assert("outside");
}

#[cfg(unix)]
#[test]
fn time_filter_does_not_keep_special_files() {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let temp = assert_fs::TempDir::new().unwrap();
    let fifo = temp.path().join("pipe");
    let name = CString::new(fifo.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args([
            "--dry-run",
            "--format",
            "json",
            "--modified-since",
            "1601-01-01T00:00:00Z",
            ".",
        ])
        .assert()
        .success()
        .get_output()
        .clone();
    let plan = parse_json_stdout(&output);
    assert_eq!(plan["direct_match_count"], 0);
    assert_eq!(relative_paths(&plan, "delete_items"), ["pipe"]);
    assert!(fifo.exists());
}

#[cfg(unix)]
#[path = "cli/pager_pty.rs"]
mod pager_pty;

#[test]
// Verify dry-run lists kept items and deletion targets without modifying files.
fn dry_run_lists_items_without_removing_them() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt").write_str("keep").unwrap();
    temp.child("remove.txt").write_str("remove").unwrap();

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--dry-run", "keep.txt"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let output = String::from_utf8_lossy(&output);
    assert!(output.contains("PREVIEW ONLY  Nothing will be removed"));
    assert!(output.contains("Items to keep (1):"));
    assert!(output.contains("keep.txt"));
    assert!(output.contains("Items to delete (1):"));
    assert!(output.contains("remove.txt"));
    temp.child("remove.txt").assert("remove");
}

#[test]
fn keep_patterns_preserve_leading_whitespace() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child(" keep.txt").write_str("leading").unwrap();
    temp.child("keep.txt").write_str("plain").unwrap();

    Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--force", " keep.txt"])
        .assert()
        .success();

    assert!(temp.path().join(" keep.txt").is_file());
    assert!(!temp.path().join("keep.txt").exists());
}

#[cfg(unix)]
#[test]
fn keep_patterns_preserve_trailing_whitespace() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt ").write_str("trailing").unwrap();
    temp.child("keep.txt").write_str("plain").unwrap();

    Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--force", "keep.txt "])
        .assert()
        .success();

    assert!(temp.path().join("keep.txt ").is_file());
    assert!(!temp.path().join("keep.txt").exists());
}

#[test]
fn keep_patterns_normalize_dot_components_and_repeated_separators() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("nested/keep.txt").write_str("keep").unwrap();
    temp.child("remove.txt").write_str("remove").unwrap();

    for pattern in ["nested/./keep.txt", "nested//keep.txt"] {
        let output = Command::cargo_bin("remove-except")
            .unwrap()
            .current_dir(temp.path())
            .args(["--dry-run", "--layout", "flat", pattern])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let output = String::from_utf8_lossy(&output);
        assert!(output.contains("Items to keep (2):"), "pattern: {pattern}");
        assert!(output.contains("nested/keep.txt"), "pattern: {pattern}");
        assert!(output.contains("remove.txt"), "pattern: {pattern}");
    }
}

#[cfg(unix)]
#[test]
fn preview_escapes_control_characters_in_paths() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt").write_str("keep").unwrap();
    std::fs::write(temp.path().join("bad\n\u{1b}[31mname.txt"), "remove").unwrap();

    for layout in [None, Some("flat")] {
        let mut command = Command::cargo_bin("remove-except").unwrap();
        command.current_dir(temp.path()).arg("--dry-run");
        if let Some(layout) = layout {
            command.args(["--layout", layout]);
        }
        let output = command
            .arg("keep.txt")
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let output = String::from_utf8_lossy(&output);

        assert!(!output.contains('\u{1b}'));
        assert!(!output.contains("bad\n"));
        assert!(output.contains(r"bad\n\u{1b}[31mname.txt"));
    }
}

#[test]
// Verify relative roots resolve from the startup directory and do not target items outside it.
fn relative_root_is_resolved_from_starting_directory() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("target/keep.txt").write_str("keep").unwrap();
    temp.child("target/remove.txt").write_str("remove").unwrap();
    temp.child("outside.txt").write_str("outside").unwrap();
    let canonical_root = std::fs::canonicalize(temp.path().join("target")).unwrap();

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--dry-run", "--root", "target", "keep.txt"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let output = String::from_utf8_lossy(&output);
    let root_line = output
        .lines()
        .find(|line| line.starts_with("Root"))
        .unwrap();
    assert!(
        root_line.ends_with(
            canonical_root
                .file_name()
                .unwrap()
                .to_string_lossy()
                .as_ref()
        )
    );
    #[cfg(windows)]
    assert!(!root_line.contains(r"\\?\"));
    assert!(output.contains("keep.txt"));
    assert!(output.contains("remove.txt"));
    assert!(!output.contains("outside.txt"));
    temp.child("target/remove.txt").assert("remove");
    temp.child("outside.txt").assert("outside");
}

#[test]
// Verify an absolute processing root is accepted.
fn absolute_root_is_accepted() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("target/keep.txt").write_str("keep").unwrap();
    temp.child("target/remove.txt").write_str("remove").unwrap();
    let root = std::fs::canonicalize(temp.path().join("target")).unwrap();

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--dry-run", "--root", root.to_str().unwrap(), "keep.txt"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    assert!(String::from_utf8_lossy(&output).contains("Items to delete (1):"));
}

#[test]
// Verify absolute keep patterns are limited to the selected root and outside paths are rejected.
fn absolute_keep_pattern_is_relative_to_selected_root_boundary() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("target/keep.txt").write_str("keep").unwrap();
    temp.child("target/remove.txt").write_str("remove").unwrap();
    temp.child("target-sibling/outside.txt")
        .write_str("outside")
        .unwrap();
    let selected_root = std::fs::canonicalize(temp.path().join("target")).unwrap();
    let keep_pattern = selected_root
        .join("keep.txt")
        .to_string_lossy()
        .into_owned();

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args([
            "--dry-run",
            "--root",
            selected_root.to_str().unwrap(),
            &keep_pattern,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    assert!(String::from_utf8_lossy(&output).contains("Items to delete (1):"));

    let outside_pattern = temp
        .path()
        .join("target-sibling/*")
        .to_string_lossy()
        .into_owned();
    let stderr = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args([
            "--dry-run",
            "--root",
            selected_root.to_str().unwrap(),
            &outside_pattern,
        ])
        .assert()
        .failure()
        .get_output()
        .stderr
        .clone();
    assert!(String::from_utf8_lossy(&stderr).contains("outside the processing root"));
}

#[test]
// Verify --force preserves the selected root itself and its sibling items.
fn force_with_root_preserves_selected_root_and_siblings() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("target/keep.txt").write_str("keep").unwrap();
    temp.child("target/remove.txt").write_str("remove").unwrap();
    temp.child("outside.txt").write_str("outside").unwrap();
    let root = temp.path().join("target");

    Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--force", "--root", "target", "keep.txt"])
        .assert()
        .success();

    assert!(root.is_dir());
    temp.child("target/keep.txt").assert("keep");
    assert!(!root.join("remove.txt").exists());
    temp.child("outside.txt").assert("outside");
}

#[test]
// Verify --force with no matches preserves the root and removes everything beneath it.
fn force_with_no_matches_removes_all_items_under_selected_root() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("target/nested/remove.txt")
        .write_str("remove")
        .unwrap();
    let root = temp.path().join("target");

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--force", "--root", "target", "missing"])
        .assert()
        .success()
        .get_output()
        .clone();

    assert!(String::from_utf8_lossy(&output.stderr).contains("No items matched"));
    assert!(root.is_dir());
    assert!(!root.join("nested").exists());
}

#[test]
// Verify filesystem roots are rejected even when --force is specified.
fn filesystem_root_is_rejected_even_with_force() {
    let temp = assert_fs::TempDir::new().unwrap();
    let filesystem_root = temp.path().ancestors().last().unwrap();

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args([
            "--force",
            "--root",
            filesystem_root.to_str().unwrap(),
            "keep-nothing",
        ])
        .assert()
        .failure()
        .get_output()
        .stderr
        .clone();

    assert!(String::from_utf8_lossy(&output).contains("Filesystem roots"));
}

#[cfg(windows)]
#[test]
fn drive_relative_roots_and_patterns_are_rejected_by_cli() {
    let temp = assert_fs::TempDir::new().unwrap();

    let root_error = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--dry-run", "--root", "C:folder", "keep.txt"])
        .assert()
        .failure()
        .get_output()
        .stderr
        .clone();
    assert!(String::from_utf8_lossy(&root_error).contains("Drive-relative root"));

    let pattern_error = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--dry-run", "C:folder"])
        .assert()
        .failure()
        .get_output()
        .stderr
        .clone();
    assert!(String::from_utf8_lossy(&pattern_error).contains("Drive-relative pattern"));
}

#[test]
// Verify a symbolic link used as the root is processed and displayed through its canonical target.
fn root_symlink_processes_and_displays_its_canonical_target() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("target/keep.txt").write_str("keep").unwrap();
    temp.child("target/remove.txt").write_str("remove").unwrap();
    let target = temp.path().join("target");
    let link = temp.path().join("target-link");

    #[cfg(unix)]
    let link_result = std::os::unix::fs::symlink(&target, &link);
    #[cfg(windows)]
    let link_result = std::os::windows::fs::symlink_dir(&target, &link);

    if link_result.is_err() {
        eprintln!("Skipping root symlink test because symlink creation is unavailable");
        return;
    }

    let canonical_target = std::fs::canonicalize(&target).unwrap();
    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--dry-run", "--root", "target-link", "keep.txt"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let output = String::from_utf8_lossy(&output);
    let root_line = output
        .lines()
        .find(|line| line.starts_with("Root"))
        .unwrap();
    assert!(
        root_line.ends_with(
            canonical_target
                .file_name()
                .unwrap()
                .to_string_lossy()
                .as_ref()
        )
    );
    #[cfg(windows)]
    assert!(!root_line.contains(r"\\?\"));
    assert!(output.contains("remove.txt"));
    temp.child("target/remove.txt").assert("remove");
}

#[test]
// Verify file and directory links beneath the root are not followed and their targets survive deletion.
fn child_symlinks_are_not_traversed_or_removed_with_their_targets() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("root/keep.txt").write_str("keep").unwrap();
    temp.child("outside/nested/secret.txt")
        .write_str("secret")
        .unwrap();
    temp.child("outside-file.txt")
        .write_str("file target")
        .unwrap();
    let root = temp.path().join("root");
    let target = temp.path().join("outside");
    let directory_link = root.join("external");
    let file_target = temp.path().join("outside-file.txt");
    let file_link = root.join("external-file");

    #[cfg(unix)]
    let directory_link_result = std::os::unix::fs::symlink(&target, &directory_link);
    #[cfg(windows)]
    let directory_link_result = std::os::windows::fs::symlink_dir(&target, &directory_link);

    #[cfg(unix)]
    let file_link_result = std::os::unix::fs::symlink(&file_target, &file_link);
    #[cfg(windows)]
    let file_link_result = std::os::windows::fs::symlink_file(&file_target, &file_link);

    if directory_link_result.is_err() || file_link_result.is_err() {
        eprintln!("Skipping child symlink test because symlink creation is unavailable");
        return;
    }

    let preview = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--dry-run", "--root", "root", "keep.txt"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let preview = String::from_utf8_lossy(&preview);
    assert!(preview.contains("external@"));
    assert!(preview.contains("external-file@"));
    assert!(!preview.contains("secret.txt"));

    Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--force", "--root", "root", "keep.txt"])
        .assert()
        .success();

    assert!(std::fs::symlink_metadata(&directory_link).is_err());
    assert!(std::fs::symlink_metadata(&file_link).is_err());
    temp.child("outside/nested/secret.txt").assert("secret");
    temp.child("outside-file.txt").assert("file target");
}

#[cfg(windows)]
#[test]
fn dangling_directory_symlink_is_removed_without_following_its_target() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("root/keep.txt").write_str("keep").unwrap();
    let target = temp.path().join("missing-target");
    let link = temp.path().join("root/dangling-directory-link");

    if std::os::windows::fs::symlink_dir(&target, &link).is_err() {
        eprintln!(
            "Skipping dangling directory symlink test because symlink creation is unavailable"
        );
        return;
    }

    Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--force", "--root", "root", "keep.txt"])
        .assert()
        .success();

    assert!(std::fs::symlink_metadata(&link).is_err());
    assert!(!target.exists());
    temp.child("root/keep.txt").assert("keep");
}

#[test]
// Verify the no-match warning names the selected processing root.
fn no_match_warning_names_selected_root() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("target/remove.txt").write_str("remove").unwrap();
    let canonical_root = std::fs::canonicalize(temp.path().join("target")).unwrap();

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--dry-run", "--root", "target", "missing"])
        .assert()
        .success()
        .get_output()
        .stderr
        .clone();

    assert!(
        String::from_utf8_lossy(&output).contains(
            canonical_root
                .file_name()
                .unwrap()
                .to_string_lossy()
                .as_ref()
        )
    );
    temp.child("target/remove.txt").assert("remove");
}

#[test]
// Verify default tree output sorts by relative path and does not show deletion-root markers.
fn default_display_sorts_by_relative_path_and_marks_kept_parents_without_root_tags() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt").write_str("keep").unwrap();
    temp.child("0-file.txt").write_str("remove").unwrap();
    temp.child("alpha/nested.txt").write_str("remove").unwrap();
    temp.child("beta/nested.txt").write_str("remove").unwrap();

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--dry-run", "keep.txt"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let output = String::from_utf8_lossy(&output);
    assert!(output.contains("Items to delete (5):"));
    assert!(output.contains("Markers: / directory; @ symlink; + kept parent."));
    assert_eq!(output.matches("Markers:").count(), 1);
    assert!(output.contains("├── 0-file.txt"));
    assert!(output.contains("├── alpha/"));
    assert!(output.contains("│   └── nested.txt"));
    assert!(output.contains("└── beta/"));
    assert!(output.find("0-file.txt").unwrap() < output.find("├── alpha/").unwrap());
    assert!(!output.contains("!"));
    assert!(!output.contains("[Directory]"));
    assert!(!output.contains("[File]"));
    assert!(!output.contains("[DELETE ROOT]"));
    temp.child("alpha/nested.txt").assert("remove");
}

#[test]
// Verify tree ordering keeps each directory subtree contiguous when sibling names share prefixes.
fn tree_display_groups_directory_subtrees_before_prefix_siblings() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt").write_str("keep").unwrap();
    temp.child("a/child.txt").write_str("remove").unwrap();
    temp.child("a.txt").write_str("remove").unwrap();
    temp.child("a-b.txt").write_str("remove").unwrap();

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--dry-run", "keep.txt"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let output = String::from_utf8_lossy(&output);
    let tree = output.split("Items to delete (4):\n").nth(1).unwrap();
    let directory = tree.find("├── a/").unwrap();
    let child = tree.find("│   └── child.txt").unwrap();
    let hyphen_sibling = tree.find("├── a-b.txt").unwrap();
    let dot_sibling = tree.find("└── a.txt").unwrap();
    assert!(directory < child && child < hyphen_sibling && hyphen_sibling < dot_sibling);
    assert_eq!(tree.matches("a/").count(), 1);
    assert!(!tree.contains("a/+"));
    temp.child("a/child.txt").assert("remove");
    temp.child("a.txt").assert("remove");
    temp.child("a-b.txt").assert("remove");
}

#[test]
fn tree_display_groups_kept_parents_and_preserves_flat_and_json_order() {
    let temp = assert_fs::TempDir::new().unwrap();
    for (name, content) in [
        ("a/keep.txt", "keep"),
        ("a/remove.txt", "remove"),
        ("a b.txt", "space"),
        ("a-b.txt", "hyphen"),
        ("a.txt", "dot"),
    ] {
        temp.child(name).write_str(content).unwrap();
    }

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--dry-run", "--pager", "never", "a/keep.txt"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let output = String::from_utf8_lossy(&output);
    let keep = output
        .split("Items to keep (2):\n")
        .nth(1)
        .unwrap()
        .split("Items to delete")
        .next()
        .unwrap();
    assert!(keep.contains("└── a/\n    └── keep.txt"));
    assert_eq!(keep.matches("a/").count(), 1);
    let delete = output.split("Items to delete (4):\n").nth(1).unwrap();
    let expected = [
        "├── a/+",
        "│   └── remove.txt",
        "├── a b.txt",
        "├── a-b.txt",
        "└── a.txt",
    ];
    let positions: Vec<_> = expected
        .iter()
        .map(|line| delete.find(line).unwrap())
        .collect();
    assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(delete.matches("a/+").count(), 1);

    let flat = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--dry-run", "--layout", "flat", "a/keep.txt"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let flat = String::from_utf8_lossy(&flat);
    let flat = flat.split("Items to delete (4):\n").nth(1).unwrap();
    let paths = ["a b.txt", "a-b.txt", "a.txt", "a/remove.txt"];
    let positions: Vec<_> = paths.iter().map(|path| flat.find(path).unwrap()).collect();
    assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(!flat.contains("a/+"));

    let json = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--dry-run", "--format", "json", "a/keep.txt"])
        .assert()
        .success()
        .get_output()
        .clone();
    let plan = parse_json_stdout(&json);
    assert_eq!(relative_paths(&plan, "keep_items"), ["a", "a/keep.txt"]);
    assert_eq!(relative_paths(&plan, "delete_items"), paths);
    assert_eq!(relative_paths(&plan, "delete_roots"), paths);
    for (name, content) in [
        ("a/keep.txt", "keep"),
        ("a/remove.txt", "remove"),
        ("a b.txt", "space"),
        ("a-b.txt", "hyphen"),
        ("a.txt", "dot"),
    ] {
        temp.child(name).assert(content);
    }
}

#[test]
// Verify --layout flat displays relative paths instead of a tree.
fn flat_option_displays_relative_paths_instead_of_a_tree() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt").write_str("keep").unwrap();
    temp.child("nested/remove.txt").write_str("remove").unwrap();

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--dry-run", "--layout", "flat", "keep.txt"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let output = String::from_utf8_lossy(&output);
    assert!(output.contains("RelativePath"));
    assert!(
        output
            .lines()
            .any(|line| { line.contains("Directory") && line.ends_with("nested/") })
    );
    assert!(output.contains("nested/remove.txt"));
    assert!(output.contains("Markers: / directory; @ symlink."));
    assert!(!output.contains("└──"));
    assert!(!output.contains('\u{1b}'));
}

#[test]
// Verify --sort-by-type groups items by type and sorts paths within each type.
fn flat_type_sort_groups_types_and_sorts_paths_within_each_type() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt").write_str("keep").unwrap();
    temp.child("0-file.txt").write_str("remove").unwrap();
    temp.child("y-file.txt").write_str("remove").unwrap();
    temp.child("x-file.txt").write_str("remove").unwrap();
    temp.child("z-dir/remove.txt").write_str("remove").unwrap();

    let default_output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--dry-run", "--layout", "flat", "keep.txt"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let default_output = String::from_utf8_lossy(&default_output);
    assert!(default_output.find("0-file.txt").unwrap() < default_output.find("z-dir/").unwrap());

    let sorted_output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args([
            "--dry-run",
            "--layout",
            "flat",
            "--sort-by-type",
            "keep.txt",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let sorted_output = String::from_utf8_lossy(&sorted_output);
    let delete_section = sorted_output.split("Items to delete").nth(1).unwrap();
    assert!(delete_section.find("Directory").unwrap() < delete_section.find("File").unwrap());
    assert!(
        delete_section.find("x-file.txt").unwrap() < delete_section.find("y-file.txt").unwrap()
    );
    assert!(
        delete_section.find("y-file.txt").unwrap()
            < delete_section.find("z-dir/remove.txt").unwrap()
    );
}

#[test]
// Verify --sort-by-type places symbolic links after files.
fn flat_type_sort_places_symlinks_after_files() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt").write_str("keep").unwrap();
    temp.child("z-file.txt").write_str("remove").unwrap();
    let target = temp.path().join("keep.txt");
    let link = temp.path().join("a-link");

    #[cfg(unix)]
    let link_result = std::os::unix::fs::symlink(&target, &link);
    #[cfg(windows)]
    let link_result = std::os::windows::fs::symlink_file(&target, &link);

    if link_result.is_err() {
        eprintln!("Skipping flat symlink sort test because symlink creation is unavailable");
        return;
    }

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args([
            "--dry-run",
            "--layout",
            "flat",
            "--sort-by-type",
            "keep.txt",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let output = String::from_utf8_lossy(&output);
    let delete_section = output.split("Items to delete").nth(1).unwrap();
    assert!(delete_section.find("File").unwrap() < delete_section.find("Symlink").unwrap());
    assert!(delete_section.contains("a-link@"));
}

#[test]
// Verify --sort-by-type is rejected with tree layout.
fn flat_type_sort_requires_flat_option() {
    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .args(["--dry-run", "--sort-by-type", "keep.txt"])
        .assert()
        .failure()
        .get_output()
        .stderr
        .clone();
    let output = String::from_utf8_lossy(&output);
    assert!(output.contains("--layout flat"));
    assert!(output.contains("Usage:"));
}

#[test]
// Verify help documents the choices and defaults for --layout and --summary.
fn help_documents_layout_and_summary_options() {
    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .arg("--help")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let output = String::from_utf8_lossy(&output);

    assert!(output.contains("[KEEP_PATTERN]..."));
    assert!(output.contains("--layout <LAYOUT>"));
    assert!(output.contains("[default: tree]"));
    assert!(output.contains("--summary <SECTION>"));
    assert!(output.contains("keep, delete, or both"));
    assert!(output.contains("Output:"));
    assert!(output.contains("kept parent directories in the delete tree green"));
    assert!(output.contains("Flat output has no kept-parent marker"));
    assert!(output.contains("PREVIEW ONLY (cyan)"));
    assert!(!output.contains('\u{1b}'));
}

#[test]
fn replaced_display_options_are_rejected() {
    for obsolete_option in [
        "--flat",
        "--keep-summary",
        "--delete-summary",
        "--summary-only",
    ] {
        Command::cargo_bin("remove-except")
            .unwrap()
            .args([obsolete_option, "keep.txt"])
            .assert()
            .failure();
    }
}

#[test]
// Verify tree output marks a kept parent directory that contains a deletion target.
fn tree_display_labels_retained_parent_for_delete_child() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("group/keep.txt").write_str("keep").unwrap();
    temp.child("group/remove.txt").write_str("remove").unwrap();

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--dry-run", "group/keep.txt"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let output = String::from_utf8_lossy(&output);
    assert!(output.contains("└── group/+"));
    assert!(output.contains("    └── remove.txt"));
    assert!(!output.contains('\u{1b}'));
    temp.child("group/remove.txt").assert("remove");
}

#[test]
// Verify --summary keep omits only kept-item paths and preserves the deletion list.
fn keep_summary_hides_only_keep_paths() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt").write_str("keep").unwrap();
    temp.child("remove.txt").write_str("remove").unwrap();

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--dry-run", "--summary", "keep", "keep.txt"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let output = String::from_utf8_lossy(&output);
    assert!(output.contains("KEEP:"));
    assert!(output.contains("1 kept item (including required ancestors)"));
    assert!(output.contains("1 direct pattern match"));
    assert!(!output.contains("keep.txt"));
    assert!(output.contains("Items to delete (1):"));
    assert!(output.contains("remove.txt"));
    assert!(!output.contains('\u{1b}'));
}

#[test]
// Verify --summary delete omits only deletion-target paths and preserves the keep list.
fn delete_summary_hides_only_delete_paths() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt").write_str("keep").unwrap();
    temp.child("remove.txt").write_str("remove").unwrap();

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--dry-run", "--summary", "delete", "keep.txt"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let output = String::from_utf8_lossy(&output);
    assert!(output.contains("Items to keep (1):"));
    assert!(output.contains("keep.txt"));
    assert!(output.contains("DELETE:"));
    assert!(output.contains("1 deletion item; 1 top-level deletion operation"));
    assert!(!output.contains("remove.txt"));
}

#[test]
// Verify --summary both summarizes both kept items and deletion targets.
// Verify --summary both replaces both path sections with counts and keeps files in a dry run.
fn summary_both_shows_counts_for_both_sections() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt").write_str("keep").unwrap();
    temp.child("remove.txt").write_str("remove").unwrap();

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--dry-run", "--summary", "both", "keep.txt"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let summary_output = String::from_utf8_lossy(&output);
    assert!(summary_output.contains("KEEP:"));
    assert!(summary_output.contains("DELETE:"));
    assert!(summary_output.contains("Summary sections omit paths."));
    assert!(!summary_output.contains("Items to keep"));
    assert!(!summary_output.contains("Items to delete"));
    temp.child("remove.txt").assert("remove");
}

#[test]
// Verify -n previews the plan like --dry-run without deleting anything.
fn short_dry_run_option_previews_without_deleting_items() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt").write_str("keep").unwrap();
    temp.child("remove.txt").write_str("remove").unwrap();

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["-n", "keep.txt"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    assert!(String::from_utf8_lossy(&output).contains("remove.txt"));
    temp.child("remove.txt").assert("remove");
}

#[test]
// Verify --force keeps matching items and deletes everything else.
fn force_removes_unmatched_items_and_keeps_matching_items() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt").write_str("keep").unwrap();
    temp.child("remove.txt").write_str("remove").unwrap();

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--force", "keep.txt"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    assert!(String::from_utf8_lossy(&output).contains("Removal complete."));
    temp.child("keep.txt").assert("keep");
    assert!(!temp.path().join("remove.txt").exists());
}

#[test]
// Verify -f deletes nonmatching items like --force.
fn short_force_option_removes_unmatched_items() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt").write_str("keep").unwrap();
    temp.child("remove.txt").write_str("remove").unwrap();

    Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["-f", "keep.txt"])
        .assert()
        .success();

    temp.child("keep.txt").assert("keep");
    assert!(!temp.path().join("remove.txt").exists());
}

#[test]
// Verify answering yes to the confirmation prompt performs deletion.
fn confirmation_accepts_yes_and_removes_unmatched_items() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt").write_str("keep").unwrap();
    temp.child("remove.txt").write_str("remove").unwrap();

    Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .arg("keep.txt")
        .write_stdin("y\n")
        .assert()
        .success();

    temp.child("keep.txt").assert("keep");
    assert!(!temp.path().join("remove.txt").exists());
}

#[test]
// Verify no confirmation is requested and kept items remain when there is nothing to delete.
fn no_removal_operations_do_not_prompt_for_confirmation() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep/nested/important.txt")
        .write_str("keep")
        .unwrap();

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .arg("keep")
        .write_stdin("")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let output = String::from_utf8_lossy(&output);

    assert!(output.contains("NO REMOVALS  No deletion operations are needed"));
    assert!(!output.contains("Are you sure you want to delete"));
    temp.child("keep/nested/important.txt").assert("keep");
}

#[test]
// Verify multiple keep patterns use OR semantics.
fn multiple_patterns_keep_items_matching_any_pattern() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep-a.txt").write_str("keep").unwrap();
    temp.child("keep-b.txt").write_str("keep").unwrap();
    temp.child("remove.txt").write_str("remove").unwrap();

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--dry-run", "keep-a.txt", "keep-b.txt"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let output = String::from_utf8_lossy(&output);
    let (keep_section, delete_section) = output.split_once("Items to delete").unwrap();
    assert!(keep_section.contains("keep-a.txt"));
    assert!(keep_section.contains("keep-b.txt"));
    assert!(delete_section.contains("remove.txt"));
    assert!(!delete_section.contains("keep-a.txt"));
    assert!(!delete_section.contains("keep-b.txt"));
}

#[test]
// Verify CLI invocations without keep patterns are rejected.
fn missing_patterns_are_rejected() {
    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .arg("--dry-run")
        .assert()
        .code(2)
        .get_output()
        .stderr
        .clone();

    assert!(String::from_utf8_lossy(&output).contains("required"));
}

#[test]
// Verify the confirmation prompt defaults to No and cancels deletion.
fn confirmation_defaults_to_no() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("remove.txt").write_str("remove").unwrap();

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--summary", "both", "keep-nothing"])
        .write_stdin("n\n")
        .assert()
        .code(3)
        .get_output()
        .clone();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stdout.contains("Summary sections omit paths."));
    assert!(stdout.contains("Aborted."));
    assert!(stderr.contains("Delete the plan? Summary sections do not list item paths."));
    temp.child("remove.txt").assert("remove");
}

#[test]
// Verify partial, unknown, and non-UTF-8 answers all decline without deleting anything.
fn confirmation_declines_answers_other_than_y_or_yes() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("remove.txt").write_str("remove").unwrap();

    for answer in [
        &b"maybe\n"[..],
        b"yes please\n",
        b"y",
        b"yes",
        b"\x82\xcd\x82\xa2\r\n",
        b"",
    ] {
        let output = Command::cargo_bin("remove-except")
            .unwrap()
            .current_dir(temp.path())
            .arg("keep-nothing")
            .write_stdin(answer)
            .assert()
            .code(3)
            .get_output()
            .stdout
            .clone();
        assert!(String::from_utf8_lossy(&output).contains("Aborted."));
        temp.child("remove.txt").assert("remove");
    }
}

#[test]
// Verify normal execution lists kept items and deletion targets before confirmation and preserves files when declined.
fn confirmation_shows_both_item_lists_before_prompt_and_preserves_items_on_rejection() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt").write_str("keep").unwrap();
    temp.child("remove.txt").write_str("remove").unwrap();

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .arg("keep.txt")
        .write_stdin("n\n")
        .assert()
        .code(3)
        .get_output()
        .clone();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    let keep_list = stdout.find("Items to keep (1):").unwrap();
    let delete_list = stdout.find("Items to delete (1):").unwrap();
    let prompt = stderr
        .find("Are you sure you want to delete the above items?")
        .unwrap();
    assert!(keep_list < delete_list);
    assert!(!stdout.contains("Are you sure you want to delete"));
    assert!(stderr.find("Review before deletion:").unwrap() < prompt);
    assert!(stderr.contains("KEEP: 1 items (including required ancestors); 1 direct matches"));
    assert!(stderr.contains("DELETE: 1 items; 1 top-level deletion operations"));
    assert!(stdout.contains("Aborted."));
    temp.child("keep.txt").assert("keep");
    temp.child("remove.txt").assert("remove");
}

#[test]
fn redirected_recap_has_no_ansi_even_when_color_is_forced() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt").write_str("keep").unwrap();
    temp.child("remove.txt").write_str("remove").unwrap();
    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--pager", "never", "keep.txt"])
        .env("CLICOLOR_FORCE", "1")
        .write_stdin("n\n")
        .assert()
        .code(3)
        .get_output()
        .clone();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("Review before deletion:"));
    assert!(!stderr.contains('\x1b'));
    temp.child("remove.txt").assert("remove");
}

#[test]
fn pager_help_and_invalid_values() {
    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .arg("--help")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let help = String::from_utf8(output).unwrap();
    assert!(help.contains("--pager <MODE>"));
    assert!(help.contains("[default: auto]"));
    assert!(help.contains("auto, always, never"));
    Command::cargo_bin("remove-except")
        .unwrap()
        .args(["--pager", "sometimes", "keep.txt"])
        .assert()
        .code(2);
}

#[test]
fn explicit_always_rejects_json_and_force_before_deletion() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt").write_str("keep").unwrap();
    temp.child("remove.txt").write_str("remove").unwrap();
    for mode in [vec!["--format", "json"], vec!["--force"]] {
        let mut command = Command::cargo_bin("remove-except").unwrap();
        command
            .current_dir(temp.path())
            .args(["--pager", "always", "keep.txt"])
            .args(mode)
            .assert()
            .code(2);
        temp.child("remove.txt").assert("remove");
    }
}

#[test]
fn pager_auto_falls_back_and_never_preserves_the_listing() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt").write_str("keep").unwrap();
    temp.child("remove.txt").write_str("remove").unwrap();
    let mut listings = Vec::new();
    for mode in ["auto", "never"] {
        let output = Command::cargo_bin("remove-except")
            .unwrap()
            .current_dir(temp.path())
            .args(["--dry-run", "--pager", mode, "keep.txt"])
            .assert()
            .success()
            .get_output()
            .clone();
        assert!(!output.stdout.contains(&0x1b));
        assert!(!String::from_utf8_lossy(&output.stderr).contains("Review before deletion:"));
        assert_eq!(
            String::from_utf8_lossy(&output.stderr).contains("Viewer unavailable:"),
            mode == "auto"
        );
        listings.push(output.stdout);
    }
    assert_eq!(listings[0], listings[1]);
    temp.child("remove.txt").assert("remove");
}

#[test]
// The viewer refuses redirected stdin when it would carry the confirmation answer.
// The missing controlling terminal itself is covered by the Ubuntu PTY tests.
fn explicit_always_with_redirected_confirmation_stdin_fails_without_deletion() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("remove.txt").write_str("remove").unwrap();
    let output = command_without_terminal()
        .current_dir(temp.path())
        .args(["--pager", "always", "missing"])
        .write_stdin("yes\n")
        .assert()
        .code(1)
        .get_output()
        .clone();
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Cannot start the requested viewer"));
    assert!(stderr.contains("Standard input is redirected for confirmation"));
    temp.child("remove.txt").assert("remove");
}

#[test]
fn recap_repeats_no_match_and_summary_warnings() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("remove.txt").write_str("remove").unwrap();
    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--pager", "never", "--summary", "both", "missing"])
        .write_stdin("\n")
        .assert()
        .code(3)
        .get_output()
        .clone();
    let stderr = String::from_utf8_lossy(&output.stderr);
    let recap = &stderr[stderr.find("Review before deletion:").unwrap()..];
    assert!(recap.contains("KEEP: 0 items (including required ancestors); 0 direct matches"));
    assert!(recap.contains("Everything under the root is a deletion target."));
    assert!(recap.contains("Summary sections omit paths."));
    temp.child("remove.txt").assert("remove");
}

#[test]
fn json_and_force_do_not_enter_viewer_or_emit_recap() {
    for mode in ["auto", "never"] {
        let temp = assert_fs::TempDir::new().unwrap();
        temp.child("keep.txt").write_str("keep").unwrap();
        temp.child("remove.txt").write_str("remove").unwrap();
        let output = Command::cargo_bin("remove-except")
            .unwrap()
            .current_dir(temp.path())
            .args(["--pager", mode, "--format", "json", "keep.txt"])
            .write_stdin("no\n")
            .assert()
            .code(3)
            .get_output()
            .clone();
        let plan = parse_json_stdout(&output);
        assert_eq!(plan["schema_version"], 2);
        assert!(!String::from_utf8_lossy(&output.stderr).contains("Review before deletion:"));
        let forced = Command::cargo_bin("remove-except")
            .unwrap()
            .current_dir(temp.path())
            .args(["--pager", mode, "--force", "keep.txt"])
            .assert()
            .success()
            .get_output()
            .clone();
        assert!(forced.stderr.is_empty());
        assert!(!temp.child("remove.txt").path().exists());
    }
}

#[test]
// Verify --force performs deletion as summarized when used with summary output.
fn force_with_summary_options_still_removes_unmatched_items() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt").write_str("keep").unwrap();
    temp.child("remove.txt").write_str("remove").unwrap();

    Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--force", "--summary", "both", "keep.txt"])
        .assert()
        .success();

    temp.child("keep.txt").assert("keep");
    assert!(!temp.path().join("remove.txt").exists());
}

fn parse_json_stdout(output: &std::process::Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).expect("stdout must contain exactly one JSON value")
}

fn relative_paths(plan: &serde_json::Value, section: &str) -> Vec<String> {
    plan[section]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["relative_path"].as_str().unwrap().to_owned())
        .collect()
}

/// Builds a command that cannot open a controlling terminal for confirmation.
fn command_without_terminal() -> Command {
    let binary = assert_cmd::cargo::cargo_bin("remove-except");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        let mut command = std::process::Command::new(binary);
        command.creation_flags(DETACHED_PROCESS);
        Command::from_std(command)
    }
    #[cfg(not(windows))]
    {
        let mut command = std::process::Command::new("setsid");
        command.arg(binary);
        Command::from_std(command)
    }
}

#[test]
// Verify stdin patterns strip a leading BOM and line endings, skip empty lines, and keep other text literally.
fn stdin_patterns_are_read_one_per_line_and_combined_with_positional_patterns() {
    let temp = assert_fs::TempDir::new().unwrap();
    for name in [
        "keep.txt",
        "#hash",
        " spaced",
        "positional.txt",
        "remove.txt",
    ] {
        temp.child(name).write_str("data").unwrap();
    }

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args([
            "--dry-run",
            "--format",
            "json",
            "--patterns-from-stdin",
            "positional.txt",
        ])
        .write_stdin("\u{feff}keep.txt\r\n\r\n\n#hash\n spaced\n")
        .assert()
        .success()
        .get_output()
        .clone();

    let plan = parse_json_stdout(&output);
    assert_eq!(
        relative_paths(&plan, "keep_items"),
        [" spaced", "#hash", "keep.txt", "positional.txt"]
    );
    assert_eq!(relative_paths(&plan, "delete_items"), ["remove.txt"]);
    temp.child("remove.txt").assert("data");
}

#[test]
// Verify invalid UTF-8 on stdin is rejected with its line number before anything is deleted.
fn stdin_patterns_reject_invalid_utf8_with_line_number() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("remove.txt").write_str("remove").unwrap();

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--force", "--patterns-from-stdin"])
        .write_stdin(b"keep.txt\n\xff\n".as_slice())
        .assert()
        .code(2)
        .get_output()
        .clone();

    assert!(String::from_utf8_lossy(&output.stderr).contains("line 2 is not valid UTF-8"));
    assert!(output.stdout.is_empty());
    temp.child("remove.txt").assert("remove");
}

#[test]
// Verify UTF-16 input without a BOM is rejected instead of producing patterns that match nothing.
fn stdin_patterns_reject_utf16_without_bom() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt").write_str("keep").unwrap();
    let utf16: Vec<u8> = "keep.txt\n"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--force", "--patterns-from-stdin"])
        .write_stdin(utf16)
        .assert()
        .code(2)
        .get_output()
        .clone();

    assert!(String::from_utf8_lossy(&output.stderr).contains("NUL character"));
    assert!(output.stdout.is_empty());
    temp.child("keep.txt").assert("keep");
}

#[test]
// Verify stdin containing only empty lines still requires at least one keep pattern.
fn stdin_with_only_empty_lines_is_rejected() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("remove.txt").write_str("remove").unwrap();

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--force", "--patterns-from-stdin"])
        .write_stdin("\r\n\n")
        .assert()
        .code(2)
        .get_output()
        .stderr
        .clone();

    assert!(String::from_utf8_lossy(&output).contains("At least one pattern"));
    temp.child("remove.txt").assert("remove");
}

#[test]
// Verify stdin patterns never take a confirmation answer from the consumed input stream.
fn stdin_patterns_without_terminal_do_not_delete() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt").write_str("keep").unwrap();
    temp.child("remove.txt").write_str("remove").unwrap();

    let output = command_without_terminal()
        .current_dir(temp.path())
        .arg("--patterns-from-stdin")
        .write_stdin("keep.txt\ny\n")
        .assert()
        .code(3)
        .get_output()
        .clone();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("No terminal is available to confirm deletion"));
    temp.child("keep.txt").assert("keep");
    temp.child("remove.txt").assert("remove");
}

#[test]
// Verify --force deletes with stdin patterns without needing a terminal.
fn stdin_patterns_with_force_delete_without_terminal() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt").write_str("keep").unwrap();
    temp.child("remove.txt").write_str("remove").unwrap();

    command_without_terminal()
        .current_dir(temp.path())
        .args(["--force", "--patterns-from-stdin"])
        .write_stdin("keep.txt\n")
        .assert()
        .success();

    temp.child("keep.txt").assert("keep");
    assert!(!temp.path().join("remove.txt").exists());
}

#[test]
// Verify JSON output contains the complete plan and nothing else on stdout.
fn json_dry_run_outputs_complete_plan_only() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep/nested/keep.txt")
        .write_str("keep")
        .unwrap();
    temp.child("keep/remove.txt").write_str("remove").unwrap();
    temp.child("old/file.txt").write_str("remove").unwrap();

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--dry-run", "--format", "json", "keep/nested/keep.txt"])
        .assert()
        .success()
        .get_output()
        .clone();

    let plan = parse_json_stdout(&output);
    let canonical_root = std::fs::canonicalize(temp.path()).unwrap();
    assert_eq!(plan["schema_version"], 2);
    assert_eq!(plan["root"], canonical_root.to_str().unwrap());
    assert_eq!(plan["direct_match_count"], 1);
    assert_eq!(
        relative_paths(&plan, "keep_items"),
        ["keep", "keep/nested", "keep/nested/keep.txt"]
    );
    assert_eq!(
        relative_paths(&plan, "delete_items"),
        ["keep/remove.txt", "old", "old/file.txt"]
    );
    assert_eq!(
        relative_paths(&plan, "delete_roots"),
        ["keep/remove.txt", "old"]
    );
    assert_eq!(plan["keep_items"][0]["type"], "directory");
    assert_eq!(plan["delete_items"][0]["type"], "file");
    assert!(!String::from_utf8_lossy(&output.stdout).contains('\u{1b}'));
    temp.child("old/file.txt").assert("remove");
}

#[test]
fn unicode_globs_match_question_marks_and_classes_by_scalar() {
    let temp = assert_fs::TempDir::new().unwrap();
    for name in [
        "é.txt",
        "あ.txt",
        "è.txt",
        "ê.txt",
        "ë.txt",
        "e\u{301}.txt",
        "nested/é.txt",
    ] {
        temp.child(name).write_str(name).unwrap();
    }

    for (pattern, expected_kept, expected_deleted) in [
        (
            "?.txt",
            vec!["é.txt", "あ.txt", "è.txt", "ê.txt", "ë.txt"],
            vec!["e\u{301}.txt", "nested", "nested/é.txt"],
        ),
        (
            "??.txt",
            vec!["e\u{301}.txt"],
            vec![
                "é.txt",
                "あ.txt",
                "è.txt",
                "ê.txt",
                "ë.txt",
                "nested",
                "nested/é.txt",
            ],
        ),
        (
            "[éあ].txt",
            vec!["é.txt", "あ.txt"],
            vec![
                "è.txt",
                "ê.txt",
                "ë.txt",
                "e\u{301}.txt",
                "nested",
                "nested/é.txt",
            ],
        ),
        (
            "[è-ê].txt",
            vec!["é.txt", "è.txt", "ê.txt"],
            vec!["あ.txt", "ë.txt", "e\u{301}.txt", "nested", "nested/é.txt"],
        ),
        (
            "[!é].txt",
            vec!["あ.txt", "è.txt", "ê.txt", "ë.txt"],
            vec!["é.txt", "e\u{301}.txt", "nested", "nested/é.txt"],
        ),
    ] {
        let output = Command::cargo_bin("remove-except")
            .unwrap()
            .current_dir(temp.path())
            .args(["--dry-run", "--format", "json", pattern])
            .assert()
            .success()
            .get_output()
            .clone();
        let plan = parse_json_stdout(&output);
        let kept = relative_paths(&plan, "keep_items");
        let deleted = relative_paths(&plan, "delete_items");

        for path in expected_kept {
            assert!(
                kept.contains(&path.to_owned()),
                "{pattern} should keep {path}"
            );
        }
        for path in expected_deleted {
            assert!(
                deleted.contains(&path.to_owned()),
                "{pattern} should delete {path}"
            );
        }
    }

    for name in [
        "é.txt",
        "あ.txt",
        "è.txt",
        "ê.txt",
        "ë.txt",
        "e\u{301}.txt",
        "nested/é.txt",
    ] {
        temp.child(name).assert(name);
    }
}

#[cfg(unix)]
#[test]
fn recursive_globs_keep_paths_containing_newlines() {
    for (pattern, name) in [
        ("**", "a\nb"),
        ("dir/**", "dir/a\nb"),
        ("**/x", "a\nb/x"),
        ("dir/**/x", "dir/a\nb/x"),
        ("**/*.txt", "a\nb/nested/file.txt"),
    ] {
        let temp = assert_fs::TempDir::new().unwrap();
        temp.child(name).write_str("keep").unwrap();
        let output = Command::cargo_bin("remove-except")
            .unwrap()
            .current_dir(temp.path())
            .args(["--dry-run", "--format", "json", pattern])
            .assert()
            .success()
            .get_output()
            .clone();
        let plan = parse_json_stdout(&output);
        assert!(
            relative_paths(&plan, "keep_items").contains(&name.to_owned()),
            "{pattern:?} should keep {name:?}"
        );
        assert!(relative_paths(&plan, "delete_items").is_empty());
        assert!(relative_paths(&plan, "delete_roots").is_empty());
        assert_eq!(plan["direct_match_count"], 1);
        temp.child(name).assert("keep");
    }
}

#[cfg(unix)]
#[test]
fn escaped_backslashes_before_hex_text_remain_literal() {
    for (pattern, name) in [
        (r"\\x80.txt", r"\x80.txt"),
        (r"\\xC3.txt", r"\xC3.txt"),
        (r"\\xc3\\xa9.txt", r"\xc3\xa9.txt"),
        ("\\\\x80é.txt", "\\x80é.txt"),
    ] {
        let temp = assert_fs::TempDir::new().unwrap();
        temp.child(name).write_str("keep").unwrap();
        temp.child("other.txt").write_str("remove").unwrap();
        let output = Command::cargo_bin("remove-except")
            .unwrap()
            .current_dir(temp.path())
            .args(["--dry-run", "--format", "json", pattern])
            .assert()
            .success()
            .get_output()
            .clone();
        let plan = parse_json_stdout(&output);
        assert_eq!(relative_paths(&plan, "keep_items"), [name]);
        assert_eq!(relative_paths(&plan, "delete_items"), ["other.txt"]);
        assert_eq!(relative_paths(&plan, "delete_roots"), ["other.txt"]);
        assert_eq!(plan["direct_match_count"], 1);
        temp.child(name).assert("keep");
        temp.child("other.txt").assert("remove");
    }
}

#[test]
fn descending_unicode_class_ranges_are_input_errors() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt").write_str("keep").unwrap();
    for pattern in ["[ê-è]", "[ん-あ]", "[z-a]"] {
        let output = Command::cargo_bin("remove-except")
            .unwrap()
            .current_dir(temp.path())
            .args(["--dry-run", "--format", "json", pattern])
            .assert()
            .code(2)
            .get_output()
            .clone();
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains("Invalid glob pattern"));
        temp.child("keep.txt").assert("keep");
    }
}

#[test]
// Verify JSON mode keeps execution messages and prompts on stderr.
fn json_execution_keeps_stdout_machine_readable() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt").write_str("keep").unwrap();
    temp.child("remove.txt").write_str("remove").unwrap();

    let declined = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--format", "json", "keep.txt"])
        .write_stdin("n\n")
        .assert()
        .code(3)
        .get_output()
        .clone();
    parse_json_stdout(&declined);
    let declined_stderr = String::from_utf8_lossy(&declined.stderr);
    assert!(declined_stderr.contains("Are you sure you want to delete the above items?"));
    assert!(declined_stderr.contains("Aborted."));
    temp.child("remove.txt").assert("remove");

    let forced = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--force", "--format", "json", "keep.txt"])
        .assert()
        .success()
        .get_output()
        .clone();
    parse_json_stdout(&forced);
    assert!(String::from_utf8_lossy(&forced.stderr).contains("Removal complete."));
    temp.child("keep.txt").assert("keep");
    assert!(!temp.path().join("remove.txt").exists());
}

#[test]
// Verify a confirmed JSON run keeps the prompt and completion message off stdout.
fn json_confirmed_execution_keeps_stdout_machine_readable() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt").write_str("keep").unwrap();
    temp.child("remove.txt").write_str("remove").unwrap();

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--format", "json", "keep.txt"])
        .write_stdin("y\n")
        .assert()
        .success()
        .get_output()
        .clone();

    let plan = parse_json_stdout(&output);
    assert_eq!(relative_paths(&plan, "delete_roots"), ["remove.txt"]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Are you sure you want to delete the above items?"));
    assert!(stderr.contains("Removal complete."));
    temp.child("keep.txt").assert("keep");
    assert!(!temp.path().join("remove.txt").exists());
}

#[test]
// Verify the no-match warning goes to stderr while stdout stays a single JSON plan.
fn json_no_match_warning_stays_on_stderr() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("remove.txt").write_str("remove").unwrap();

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--dry-run", "--format", "json", "missing.txt"])
        .assert()
        .success()
        .get_output()
        .clone();

    let plan = parse_json_stdout(&output);
    assert_eq!(plan["direct_match_count"], 0);
    assert_eq!(relative_paths(&plan, "delete_items"), ["remove.txt"]);
    assert!(String::from_utf8_lossy(&output.stderr).contains("WARNING: No items matched"));
    temp.child("remove.txt").assert("remove");
}

#[test]
// Verify JSON mode does not prompt when the plan has no deletion targets.
fn json_without_deletion_targets_does_not_prompt() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt").write_str("keep").unwrap();

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--format", "json", "keep.txt"])
        .assert()
        .success()
        .get_output()
        .clone();

    let plan = parse_json_stdout(&output);
    assert!(plan["delete_items"].as_array().unwrap().is_empty());
    assert!(plan["delete_roots"].as_array().unwrap().is_empty());
    assert!(output.stderr.is_empty());
    temp.child("keep.txt").assert("keep");
}

#[test]
// Verify JSON reports symbolic links with the symlink type.
fn json_reports_symlink_items() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt").write_str("keep").unwrap();
    let target = temp.path().join("keep.txt");
    let link = temp.path().join("link");

    #[cfg(unix)]
    let link_result = std::os::unix::fs::symlink(&target, &link);
    #[cfg(windows)]
    let link_result = std::os::windows::fs::symlink_file(&target, &link);

    if link_result.is_err() {
        eprintln!("Skipping JSON symlink test because symlink creation is unavailable");
        return;
    }

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--dry-run", "--format", "json", "keep.txt"])
        .assert()
        .success()
        .get_output()
        .clone();

    let plan = parse_json_stdout(&output);
    assert_eq!(plan["delete_items"][0]["relative_path"], "link");
    assert_eq!(plan["delete_items"][0]["type"], "symlink");
}

#[test]
// Verify human-oriented display options cannot be combined with JSON output.
fn json_rejects_display_options() {
    for options in [
        ["--summary", "both"].as_slice(),
        ["--layout", "tree"].as_slice(),
        ["--layout", "flat"].as_slice(),
        ["--sort-by-type"].as_slice(),
    ] {
        Command::cargo_bin("remove-except")
            .unwrap()
            .args(["--dry-run", "--format", "json"])
            .args(options)
            .arg("keep.txt")
            .assert()
            .code(2);
    }
}

#[test]
fn non_unicode_entries_fail_before_output_and_deletion() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt").write_str("keep").unwrap();
    temp.child("remove.txt").write_str("remove").unwrap();
    temp.child("nested").create_dir_all().unwrap();
    let first = temp
        .path()
        .join("nested")
        .join(support::non_unicode_name(0xff));
    let second = temp
        .path()
        .join("nested")
        .join(support::non_unicode_name(0xfe));
    std::fs::write(&first, "first").unwrap();
    std::fs::write(&second, "second").unwrap();

    for display in [
        [].as_slice(),
        ["--layout", "tree"].as_slice(),
        ["--layout", "flat"].as_slice(),
        ["--layout", "flat", "--sort-by-type"].as_slice(),
        ["--summary", "keep"].as_slice(),
        ["--summary", "delete"].as_slice(),
        ["--summary", "both"].as_slice(),
        ["--format", "json"].as_slice(),
    ] {
        for mode in [None, Some("--dry-run"), Some("--force")] {
            for pattern in ["keep.txt", ".", "nested/bad-\u{fffd}.txt"] {
                let output = Command::cargo_bin("remove-except")
                    .unwrap()
                    .current_dir(temp.path())
                    .args(display)
                    .args(mode)
                    .arg(pattern)
                    .write_stdin("yes\n")
                    .timeout(std::time::Duration::from_secs(10))
                    .assert()
                    .code(1)
                    .get_output()
                    .clone();

                assert!(
                    output.stdout.is_empty(),
                    "display: {display:?}, mode: {mode:?}"
                );
                let stderr = String::from_utf8_lossy(&output.stderr);
                assert!(stderr.contains("cannot be represented as Unicode"));
                assert!(!stderr.contains("[y/N]"));
                temp.child("keep.txt").assert("keep");
                temp.child("remove.txt").assert("remove");
                assert_eq!(std::fs::read(&first).unwrap(), b"first");
                assert_eq!(std::fs::read(&second).unwrap(), b"second");
            }
        }
    }
    for options in [
        ["--pager", "always"].as_slice(),
        ["--pager", "never"].as_slice(),
        ["--dry-run", "--pager", "always"].as_slice(),
        ["--patterns-from-stdin"].as_slice(),
        ["--patterns-from-stdin", "--force"].as_slice(),
    ] {
        let output = Command::cargo_bin("remove-except")
            .unwrap()
            .current_dir(temp.path())
            .args(options)
            .arg("keep.txt")
            .write_stdin(".\n")
            .timeout(std::time::Duration::from_secs(10))
            .assert()
            .code(1)
            .get_output()
            .clone();
        assert!(output.stdout.is_empty());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("cannot be represented as Unicode")
        );
        temp.child("remove.txt").assert("remove");
        assert_eq!(std::fs::read(&first).unwrap(), b"first");
        assert_eq!(std::fs::read(&second).unwrap(), b"second");
    }
}

#[test]
fn unicode_paths_remain_lossless_in_json() {
    let temp = assert_fs::TempDir::new().unwrap();
    let root = temp.child("root-\u{fffd}");
    root.create_dir_all().unwrap();
    let mut names = vec!["bad-\u{fffd}.txt", "bad-é.txt"];
    #[cfg(unix)]
    names.extend(["line\nbreak.txt", r"back\slash.txt"]);
    for name in &names {
        root.child(name).write_str(name).unwrap();
    }
    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(root.path())
        .args(["--dry-run", "--format", "json", "."])
        .assert()
        .success()
        .get_output()
        .clone();
    let plan = parse_json_stdout(&output);
    names.sort_unstable();
    assert_eq!(relative_paths(&plan, "keep_items"), names);
    assert!(relative_paths(&plan, "delete_items").is_empty());
    assert_eq!(plan["direct_match_count"], names.len());
    assert_eq!(
        plan["root"],
        std::fs::canonicalize(root.path())
            .unwrap()
            .to_str()
            .unwrap()
    );
    for name in names {
        root.child(name).assert(name);
    }
}

#[test]
fn non_unicode_roots_fail_before_output_and_deletion() {
    let temp = assert_fs::TempDir::new().unwrap();
    let root = temp.path().join(support::non_unicode_name(0xff));
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("remove.txt"), "remove").unwrap();

    for explicit_root in [true, false] {
        for format in ["human", "json"] {
            for mode in [None, Some("--dry-run"), Some("--force")] {
                let mut command = Command::cargo_bin("remove-except").unwrap();
                if explicit_root {
                    command.current_dir(temp.path()).arg("--root").arg(&root);
                } else {
                    command.current_dir(&root);
                }
                let output = command
                    .args(["--format", format])
                    .args(mode)
                    .arg("missing")
                    .write_stdin("yes\n")
                    .timeout(std::time::Duration::from_secs(10))
                    .assert()
                    .code(1)
                    .get_output()
                    .clone();
                assert!(output.stdout.is_empty());
                assert!(
                    String::from_utf8_lossy(&output.stderr)
                        .contains("cannot be represented as Unicode")
                );
                assert_eq!(std::fs::read(root.join("remove.txt")).unwrap(), b"remove");
            }
        }
    }
}

#[cfg(unix)]
#[test]
fn non_unicode_link_names_and_resolved_roots_are_rejected() {
    let temp = assert_fs::TempDir::new().unwrap();
    let target = assert_fs::TempDir::new().unwrap();
    let invalid_root = target.path().join(support::non_unicode_name(0xff));
    std::fs::create_dir(&invalid_root).unwrap();
    std::fs::write(invalid_root.join("outside.txt"), "outside").unwrap();
    let link = temp.path().join("link");
    std::os::unix::fs::symlink(&invalid_root, &link).unwrap();

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .arg("--root")
        .arg(&link)
        .args(["--force", "."])
        .assert()
        .code(1)
        .get_output()
        .clone();
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot be represented as Unicode"));

    let invalid_link = temp.path().join(support::non_unicode_name(0xfe));
    std::os::unix::fs::symlink(target.path().join("missing"), &invalid_link).unwrap();
    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--force", "."])
        .assert()
        .code(1)
        .get_output()
        .clone();
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot be represented as Unicode"));
    assert!(
        std::fs::symlink_metadata(invalid_link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        std::fs::read(invalid_root.join("outside.txt")).unwrap(),
        b"outside"
    );
}

#[cfg(unix)]
#[test]
// Verify JSON output fails before printing or deleting when a path is not valid Unicode.
fn json_rejects_non_unicode_paths_before_output() {
    use std::os::unix::ffi::OsStrExt;

    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt").write_str("keep").unwrap();
    let invalid_name = std::ffi::OsStr::from_bytes(b"bad-\xff.txt");
    std::fs::write(temp.path().join(invalid_name), "remove").unwrap();

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--force", "--format", "json", "keep.txt"])
        .assert()
        .code(1)
        .get_output()
        .clone();

    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot be represented as Unicode"));
    assert!(temp.path().join(invalid_name).exists());
}

#[test]
// Verify invalid patterns and roots use the input-error exit code.
fn validation_errors_use_exit_code_two() {
    let temp = assert_fs::TempDir::new().unwrap();

    for pattern in ["[", "{", "}", "a{b"] {
        let output = Command::cargo_bin("remove-except")
            .unwrap()
            .current_dir(temp.path())
            .args(["--dry-run", pattern])
            .assert()
            .code(2)
            .get_output()
            .clone();
        assert!(String::from_utf8_lossy(&output.stderr).contains("Invalid glob pattern"));
    }
    Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--dry-run", "--root", "missing-directory", "keep.txt"])
        .assert()
        .code(2);
}

#[test]
// Verify help documents pipeline options and exit codes.
fn help_documents_pipeline_options_and_exit_codes() {
    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .arg("--help")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let output = String::from_utf8_lossy(&output);

    assert!(output.contains("--patterns-from-stdin"));
    assert!(output.contains("--size-at-least <BYTES>"));
    assert!(output.contains("--size-below <BYTES>"));
    assert!(output.contains("--require-match"));
    assert!(output.contains("--max-delete-items"));
    assert!(output.contains("guards"));
    assert!(output.contains("guard_violations"));
    assert!(output.contains("--format <FORMAT>"));
    assert!(output.contains("schema_version 2"));
    assert!(output.contains("ASCII decimal byte counts in the u64 range"));
    assert!(output.contains("Exit codes:"));
    assert!(output.contains("A dry-run violation does not itself change the exit code"));
    assert!(output.contains("viewer cancellation exits 3"));
    assert!(output.contains("viewer startup or terminal errors exit 1"));
    let confirmation_line = output
        .lines()
        .find(|line| line.contains("Pending viewer input"))
        .unwrap();
    assert!(confirmation_line.starts_with("  Pending viewer input"));
    assert!(confirmation_line.contains("newline-terminated y/yes"));
}

#[test]
fn help_documents_modification_time_formats() {
    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .arg("--help")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let output = String::from_utf8_lossy(&output);

    assert!(output.contains("YYYY-MM-DDTHH:MM[:SS]"));
    assert!(output.contains("YYYY-MM-DD HH:MM[:SS]"));
    assert!(output.contains("Date-times may include Z or +/-HH:MM"));
}

#[test]
fn force_guard_violations_reject_without_human_plan_or_deletion() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("remove.txt").write_str("remove").unwrap();

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args([
            "--force",
            "--require-match",
            "--max-delete-items",
            "0",
            "missing.txt",
        ])
        .assert()
        .code(3)
        .get_output()
        .clone();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.stdout.is_empty());
    assert!(stderr.contains("SAFETY GUARD: --require-match"));
    assert!(stderr.contains("SAFETY GUARD: --max-delete-items"));
    assert!(stderr.contains("Deletion rejected by safety guards; nothing was removed."));
    assert!(!stderr.contains("Are you sure"));
    temp.child("remove.txt").assert("remove");
}

#[test]
fn normal_require_match_rejects_before_confirmation_and_viewer() {
    for pager in ["never", "always"] {
        let temp = assert_fs::TempDir::new().unwrap();
        temp.child("remove.txt").write_str("remove").unwrap();

        let output = Command::cargo_bin("remove-except")
            .unwrap()
            .current_dir(temp.path())
            .args(["--require-match", "--pager", pager, "missing.txt"])
            .write_stdin("y\n")
            .assert()
            .code(3)
            .get_output()
            .clone();

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.stdout.is_empty());
        assert!(stderr.contains("SAFETY GUARD: --require-match"));
        assert!(stderr.contains("Deletion rejected by safety guards; nothing was removed."));
        assert!(!stderr.contains("Are you sure"));
        assert!(!stderr.contains("Cannot start the requested viewer"));
        temp.child("remove.txt").assert("remove");
    }
}

#[test]
fn dry_run_guard_violation_prints_human_plan_without_viewer_or_deletion() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt").write_str("keep").unwrap();
    temp.child("remove.txt").write_str("remove").unwrap();

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args([
            "--dry-run",
            "--max-delete-items",
            "0",
            "--pager",
            "never",
            "keep.txt",
        ])
        .assert()
        .code(0)
        .get_output()
        .clone();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stdout.contains("PREVIEW ONLY"));
    let (keep_section, delete_section) = stdout.split_once("Items to delete").unwrap();
    assert!(keep_section.contains("Items to keep"));
    assert!(keep_section.contains("keep.txt"));
    assert!(delete_section.contains("remove.txt"));
    assert!(stderr.contains("SAFETY GUARD: --max-delete-items"));
    assert!(stderr.contains("exceed the maximum of 0"));
    temp.child("keep.txt").assert("keep");
    temp.child("remove.txt").assert("remove");
}

#[test]
fn dry_run_guard_violation_with_unavailable_always_viewer_uses_viewer_error_code() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt").write_str("keep").unwrap();
    temp.child("remove.txt").write_str("remove").unwrap();

    let output = command_without_terminal()
        .current_dir(temp.path())
        .args([
            "--dry-run",
            "--max-delete-items",
            "0",
            "--pager",
            "always",
            "keep.txt",
        ])
        .assert()
        .code(1)
        .get_output()
        .clone();

    assert!(String::from_utf8_lossy(&output.stderr).contains("Cannot start the requested viewer"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("--max-delete-items"));
    temp.child("remove.txt").assert("remove");
}

#[test]
fn require_match_distinguishes_empty_and_positive_delete_free_plans() {
    let temp = assert_fs::TempDir::new().unwrap();

    Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--force", "--require-match", "."])
        .assert()
        .code(3);

    temp.child("keep.txt").write_str("keep").unwrap();
    Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--force", "--require-match", "keep.txt"])
        .assert()
        .success();
    temp.child("keep.txt").assert("keep");
}

#[test]
fn max_delete_items_accepts_u64_max() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt").write_str("keep").unwrap();
    temp.child("remove.txt").write_str("remove").unwrap();

    Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args([
            "--force",
            "--max-delete-items",
            "18446744073709551615",
            "keep.txt",
        ])
        .assert()
        .success();
    temp.child("keep.txt").assert("keep");
    assert!(!temp.path().join("remove.txt").exists());
}

#[test]
fn json_always_contains_guard_fields_when_guards_are_unspecified() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt").write_str("keep").unwrap();

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--dry-run", "--format", "json", "keep.txt"])
        .assert()
        .success()
        .get_output()
        .clone();

    let plan = parse_json_stdout(&output);
    assert_eq!(
        plan["guards"],
        serde_json::json!({
            "require_match": false,
            "max_delete_items": null
        })
    );
    assert_eq!(plan["guard_violations"], serde_json::json!([]));
}

#[test]
fn json_force_guard_violation_keeps_machine_output_and_file() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("remove.txt").write_str("remove").unwrap();

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args([
            "--force",
            "--format",
            "json",
            "--max-delete-items",
            "0",
            "keep.txt",
        ])
        .assert()
        .code(3)
        .get_output()
        .clone();

    let plan = parse_json_stdout(&output);
    assert_eq!(
        plan["guard_violations"],
        serde_json::json!(["max_delete_items"])
    );
    assert_eq!(
        plan["guards"],
        serde_json::json!({
            "require_match": false,
            "max_delete_items": 0
        })
    );
    temp.child("remove.txt").assert("remove");
}

#[test]
fn force_max_delete_items_rejects_nested_and_descendant_counts() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt").write_str("keep").unwrap();
    temp.child("remove/nested/deep.txt")
        .write_str("remove")
        .unwrap();

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--force", "--max-delete-items", "2", "keep.txt"])
        .assert()
        .code(3)
        .get_output()
        .clone();

    assert!(String::from_utf8_lossy(&output.stderr).contains("3 deletion items"));
    temp.child("remove/nested/deep.txt").assert("remove");
}

#[test]
fn max_delete_items_allows_exact_limit_and_zero_for_no_removals() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt").write_str("keep").unwrap();
    temp.child("remove.txt").write_str("remove").unwrap();

    Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--force", "--max-delete-items", "1", "keep.txt"])
        .assert()
        .success();
    assert!(!temp.path().join("remove.txt").exists());

    Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--force", "--max-delete-items", "0", "."])
        .assert()
        .success();
    temp.child("keep.txt").assert("keep");

    temp.child("remove.txt").write_str("remove").unwrap();
    Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--force", "--max-delete-items", "0", "keep.txt"])
        .assert()
        .code(3);
    temp.child("remove.txt").assert("remove");
}

#[test]
fn dry_run_reports_both_guard_violations_in_json_and_exits_zero() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("remove/nested.txt").write_str("remove").unwrap();

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args([
            "--dry-run",
            "--format",
            "json",
            "--require-match",
            "--max-delete-items",
            "1",
            "missing.txt",
        ])
        .assert()
        .success()
        .get_output()
        .clone();

    let plan = parse_json_stdout(&output);
    assert_eq!(
        plan["guards"],
        serde_json::json!({
            "require_match": true,
            "max_delete_items": 1
        })
    );
    assert_eq!(
        plan["guard_violations"],
        serde_json::json!(["require_match", "max_delete_items"])
    );
    temp.child("remove/nested.txt").assert("remove");
}

#[test]
fn size_only_selection_determines_safety_guard_counts() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt").write_str("keep").unwrap();
    temp.child("remove/nested.txt").write_str("x").unwrap();

    for (mode, lower, limit, code, matches, deletes, violations) in [
        ("--force", 5, 3, 3, 0, 3, &["require_match"][..]),
        ("--force", 4, 1, 3, 1, 2, &["max_delete_items"][..]),
        (
            "--dry-run",
            5,
            2,
            0,
            0,
            3,
            &["require_match", "max_delete_items"][..],
        ),
        ("--force", 4, 2, 0, 1, 2, &[][..]),
    ] {
        let output = Command::cargo_bin("remove-except")
            .unwrap()
            .current_dir(temp.path())
            .args([mode, "--format", "json", "--require-match"])
            .arg("--size-at-least")
            .arg(lower.to_string())
            .arg("--max-delete-items")
            .arg(limit.to_string())
            .assert()
            .code(code)
            .get_output()
            .clone();

        let plan = parse_json_stdout(&output);
        assert_eq!(plan["schema_version"], 2);
        assert_eq!(
            plan["selection"],
            serde_json::json!({
                "patterns": [],
                "modified_since": null,
                "modified_before": null,
                "size_at_least": lower,
                "size_below": null
            })
        );
        assert_eq!(
            plan["guards"],
            serde_json::json!({"require_match": true, "max_delete_items": limit})
        );
        assert_eq!(plan["guard_violations"], serde_json::json!(violations));
        assert_eq!(plan["direct_match_count"], matches);
        assert_eq!(plan["delete_items"].as_array().unwrap().len(), deletes);
        temp.child("keep.txt").assert("keep");
        if mode == "--force" && violations.is_empty() {
            assert!(!temp.path().join("remove").exists());
        } else {
            temp.child("remove/nested.txt").assert("x");
        }
    }
}

#[test]
fn duplicate_or_invalid_max_delete_items_uses_input_error_code() {
    let temp = assert_fs::TempDir::new().unwrap();

    Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args([
            "--dry-run",
            "--max-delete-items",
            "1",
            "--max-delete-items",
            "2",
            ".",
        ])
        .assert()
        .code(2);

    for invalid in ["", "-1", "+1", " 1", "1 ", "1MB", "18446744073709551616"] {
        Command::cargo_bin("remove-except")
            .unwrap()
            .current_dir(temp.path())
            .args(["--dry-run", "--max-delete-items", invalid, "."])
            .assert()
            .code(2);
    }
}
