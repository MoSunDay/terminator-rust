//! Execute the generated command against private fake executables only.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn private_home() -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let home = PathBuf::from(format!(
        "/tmp/remote-keeper-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir_all(home.join("bin")).expect("create fake bin");
    home
}

fn command(path: &Path, body: &str) {
    fs::write(path, body).expect("write fake command");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("chmod fake command");
}

fn run(home: &Path) -> std::process::Output {
    let target = remote::RemoteTarget {
        label: "release build".into(),
        host: "unused".into(),
        user: None,
        port: None,
        session_name: "work".into(),
    };
    Command::new("/bin/sh")
        .args(["-c", &remote::bootstrap_command(&target)])
        .env("HOME", home)
        .env(
            "PATH",
            format!("{}:/usr/bin:/bin", home.join("bin").display()),
        )
        .output()
        .expect("run bootstrap")
}

#[test]
fn keeper_is_used_without_consulting_zellij() {
    let home = private_home();
    command(
        &home.join("bin/zellij"),
        "#!/bin/sh\nprintf 'zellij invoked\\n' > \"$HOME/zellij-used\"\n",
    );
    command(
        &home.join("bin/terminator-session"),
        "#!/bin/sh\nprintf '%s\\n' \"$*\" > \"$HOME/choice\"\n",
    );
    assert!(run(&home).status.success());
    assert_eq!(
        fs::read_to_string(home.join("choice")).expect("keeper args"),
        "attach work --title release build\n"
    );
    assert!(!home.join("zellij-used").exists());

    fs::remove_file(home.join("bin/terminator-session")).expect("remove fake keeper");
    let missing = run(&home);
    assert_eq!(missing.status.code(), Some(remote::EXIT_NO_KEEPER_SHELL));
    assert!(!home.join("zellij-used").exists());
    fs::remove_dir_all(home).expect("remove private home");
}
