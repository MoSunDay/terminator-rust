//! Prints the ssh bootstrap command line for a target (e2e helper).
use remote::{bootstrap_command, RemoteTarget};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let session = args.get(1).map(String::as_str).unwrap_or("zt-e2e");
    let t = RemoteTarget {
        label: "e2e".into(),
        host: "localhost".into(),
        user: None,
        port: None,
        session_name: session.into(),
    };
    println!("{}", bootstrap_command(&t, remote::DEFAULT_PALETTE_HEX));
}
