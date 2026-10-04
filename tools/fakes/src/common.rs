use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

/// Prints a line and appends it to logs/<name>.log in the project folder.
pub struct Log {
    file: Option<File>,
}

impl Log {
    pub fn new(name: &str) -> Log {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../logs");
        let _ = std::fs::create_dir_all(&dir);
        let file = OpenOptions::new().create(true).write(true).truncate(true)
            .open(dir.join(format!("{name}.log"))).ok();
        Log { file }
    }

    pub fn line(&mut self, s: &str) {
        let t = gmsky_protocol::shm::now_ms();
        let msg = format!("[{t}] {s}");
        println!("{msg}");
        if let Some(f) = &mut self.file {
            let _ = writeln!(f, "{msg}");
        }
    }
}

/// `--seconds N` from the command line, or the default.
pub fn seconds_arg(default: u64) -> u64 {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == "--seconds")
        .and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok()).unwrap_or(default)
}
