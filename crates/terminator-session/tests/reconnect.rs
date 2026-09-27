//! A private HOME and a uniquely named shell keep this test away from user sessions.

use std::fs;
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_terminator-session");
const NAME: &str = "test-reconnect";

fn home() -> PathBuf {
    let path = PathBuf::from("/tmp").join(format!(
        "terminator-session-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    fs::create_dir_all(&path).expect("private home");
    path
}

fn attach(home: &PathBuf) -> Child {
    Command::new(BIN)
        .args(["attach", NAME, "--title", "release build"])
        .env("HOME", home)
        .env("SHELL", "/bin/sh")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("attach")
}

fn send(child: &mut Child, line: &str) {
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(line.as_bytes())
        .expect("send line");
}

/// Wait until `needle` appears in the child's stdout.
fn until(child: &mut Child, needle: &str) {
    let stdout = child.stdout.as_mut().expect("stdout");
    let end = Instant::now() + Duration::from_secs(8);
    let mut seen = Vec::new();
    while Instant::now() < end {
        let mut fd = libc::pollfd {
            fd: stdout.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        if unsafe { libc::poll(&mut fd, 1, 200) } <= 0 {
            continue;
        }
        let mut buf = [0u8; 8192];
        let n = stdout.read(&mut buf).expect("read output");
        if n == 0 {
            break;
        }
        seen.extend_from_slice(&buf[..n]);
        if String::from_utf8_lossy(&seen).contains(needle) {
            return;
        }
    }
    panic!("missing {needle:?} in {:?}", String::from_utf8_lossy(&seen));
}

fn listing(home: &PathBuf) -> String {
    let output = Command::new(BIN)
        .arg("list")
        .env("HOME", home)
        .output()
        .expect("list");
    assert!(output.status.success());
    String::from_utf8(output.stdout).expect("utf8 list")
}

fn exited(child: &mut Child) -> bool {
    let end = Instant::now() + Duration::from_secs(5);
    while Instant::now() < end {
        if let Some(status) = child.try_wait().expect("wait child") {
            return status.success();
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

fn wait_list(home: &PathBuf, expected: &str) {
    let end = Instant::now() + Duration::from_secs(5);
    while Instant::now() < end {
        if listing(home).contains(expected) {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("list never showed {expected:?}");
}

#[test]
fn detached_shell_reattaches_and_two_clients_can_share_it() {
    let home = home();
    let mut first = attach(&home);
    send(&mut first, "export KEEP_MARK=alive\nprintf 'READY\\n'\n");
    until(&mut first, "READY");
    // The keeper's shell exports TERMINATOR_SESSION: a child can tell a
    // keeper session apart from a plain shell.
    send(
        &mut first,
        &format!("printf 'ENV:%s\\n' \"${}\"\n", ipc_proto::ENV_SESSION),
    );
    until(&mut first, &format!("ENV:{NAME}"));

    let mut second = attach(&home);
    until(&mut second, "READY"); // history replay
    let list = listing(&home);
    assert!(list.contains(NAME) && list.contains("attached"));
    assert!(list.contains("release build"));
    send(&mut second, "printf 'BOTH:%s\\n' \"$KEEP_MARK\"\n");
    until(&mut first, "BOTH:alive");
    until(&mut second, "BOTH:alive");

    drop(first.stdin.take());
    assert!(exited(&mut first));
    assert!(listing(&home).contains("attached"));
    send(&mut second, "printf 'STILL:%s\\n' \"$KEEP_MARK\"\n");
    until(&mut second, "STILL:alive");
    drop(second.stdin.take());
    assert!(exited(&mut second));
    wait_list(&home, "detached");

    let mut third = attach(&home);
    send(&mut third, "exit\n");
    drop(third.stdin.take());
    assert!(exited(&mut third));
    let end = Instant::now() + Duration::from_secs(5);
    while Instant::now() < end && listing(&home).contains(NAME) {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!listing(&home).contains(NAME));
    fs::remove_dir_all(home).expect("remove private home");
}
