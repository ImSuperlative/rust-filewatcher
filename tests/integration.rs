use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_filewatcher"))
}

fn spawn_line_reader(child: &mut std::process::Child) -> mpsc::Receiver<String> {
    let stdout = child.stdout.take().expect("child has no stdout");
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            match line {
                Ok(l) => {
                    if tx.send(l).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });
    rx
}

#[cfg(unix)]
fn send_sigterm(child: &std::process::Child) {
    unsafe extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }
    unsafe {
        kill(child.id() as i32, 15);
    }
}

#[test]
fn file_creation_detected() {
    let dir = tempfile::tempdir().unwrap();

    let mut child = Command::new(binary())
        .args(["--debounce", "100", dir.path().to_str().unwrap()])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to start filewatcher");

    let rx = spawn_line_reader(&mut child);
    thread::sleep(Duration::from_millis(500));

    std::fs::write(dir.path().join("test.php"), "<?php echo 1;").unwrap();

    let line = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("timed out waiting for changed event");

    assert!(line.starts_with("changed: "), "unexpected: {}", line);
    assert!(line.contains("test.php"), "missing filename: {}", line);

    child.kill().ok();
    let _ = child.wait();
}

#[test]
fn extension_filtering() {
    let dir = tempfile::tempdir().unwrap();

    let mut child = Command::new(binary())
        .args(["--ext", "php", "--debounce", "100", dir.path().to_str().unwrap()])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to start filewatcher");

    let rx = spawn_line_reader(&mut child);
    thread::sleep(Duration::from_millis(500));

    std::fs::write(dir.path().join("readme.txt"), "hello").unwrap();
    thread::sleep(Duration::from_millis(500));

    std::fs::write(dir.path().join("index.php"), "<?php").unwrap();

    let line = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("timed out waiting for php change");

    assert!(line.contains("index.php"), "expected index.php: {}", line);

    child.kill().ok();
    let _ = child.wait();
}

#[test]
fn ignored_directories() {
    let dir = tempfile::tempdir().unwrap();

    std::fs::create_dir_all(dir.path().join(".git")).unwrap();
    std::fs::create_dir_all(dir.path().join("vendor")).unwrap();

    let mut child = Command::new(binary())
        .args(["--debounce", "100", dir.path().to_str().unwrap()])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to start filewatcher");

    let rx = spawn_line_reader(&mut child);
    thread::sleep(Duration::from_millis(500));

    std::fs::write(dir.path().join(".git/config.php"), "x").unwrap();
    std::fs::write(dir.path().join("vendor/autoload.php"), "x").unwrap();
    thread::sleep(Duration::from_millis(500));

    std::fs::write(dir.path().join("app.php"), "<?php").unwrap();

    let line = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("timed out waiting for app.php");

    assert!(line.contains("app.php"), "expected app.php: {}", line);

    child.kill().ok();
    let _ = child.wait();
}

#[test]
fn debouncing() {
    let dir = tempfile::tempdir().unwrap();

    let mut child = Command::new(binary())
        .args(["--debounce", "300", dir.path().to_str().unwrap()])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to start filewatcher");

    let rx = spawn_line_reader(&mut child);
    thread::sleep(Duration::from_millis(500));

    for i in 0..5 {
        std::fs::write(dir.path().join(format!("file{}.php", i)), "<?php").unwrap();
        thread::sleep(Duration::from_millis(10));
    }

    thread::sleep(Duration::from_millis(600));

    let mut lines = Vec::new();
    while let Ok(line) = rx.try_recv() {
        lines.push(line);
    }

    assert!(!lines.is_empty(), "expected at least one output line");
    assert!(
        lines.len() <= 5,
        "too many output lines ({}), debouncing may not be working",
        lines.len()
    );

    child.kill().ok();
    let _ = child.wait();
}

#[test]
fn polling_mode() {
    let dir = tempfile::tempdir().unwrap();

    let mut child = Command::new(binary())
        .args([
            "--poll",
            "--poll-interval",
            "200",
            "--debounce",
            "100",
            dir.path().to_str().unwrap(),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to start filewatcher");

    let rx = spawn_line_reader(&mut child);
    thread::sleep(Duration::from_millis(500));

    std::fs::write(dir.path().join("polled.php"), "<?php").unwrap();

    let line = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("timed out waiting for polled.php");

    assert!(line.contains("polled.php"), "expected polled.php: {}", line);

    child.kill().ok();
    let _ = child.wait();
}

#[cfg(unix)]
#[test]
fn clean_shutdown() {
    let dir = tempfile::tempdir().unwrap();

    let mut child = Command::new(binary())
        .args([dir.path().to_str().unwrap()])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to start filewatcher");

    thread::sleep(Duration::from_millis(300));

    send_sigterm(&child);

    let status = child.wait().expect("failed to wait for child");

    assert!(
        status.success(),
        "expected exit code 0, got {:?}",
        status.code()
    );
}

#[test]
fn no_paths_exits_with_error() {
    let output = Command::new(binary())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("failed to run filewatcher");

    assert!(!output.status.success());

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("at least one path"),
        "expected helpful error on stderr: {}",
        stderr
    );
}

#[test]
fn invalid_path_exits_with_error() {
    let output = Command::new(binary())
        .args(["/nonexistent/path/that/does/not/exist"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("failed to run filewatcher");

    assert!(!output.status.success());

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.is_empty(), "expected error message on stderr");
}
