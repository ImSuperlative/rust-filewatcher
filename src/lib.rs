use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, SystemTime};

use notify::event::ModifyKind;
use notify::{EventKind, RecursiveMode, Watcher};

pub static SHUTDOWN: AtomicBool = AtomicBool::new(false);

#[cfg(unix)]
pub fn install_signal_handlers() {
    unsafe extern "C" {
        fn signal(sig: i32, handler: extern "C" fn(i32)) -> usize;
    }

    extern "C" fn handle(_: i32) {
        SHUTDOWN.store(true, Ordering::SeqCst);
    }

    unsafe {
        signal(2, handle); // SIGINT
        signal(15, handle); // SIGTERM
    }
}

#[cfg(not(unix))]
pub fn install_signal_handlers() {}

pub struct Config {
    pub extensions: Vec<String>,
    pub poll: bool,
    pub poll_interval: Duration,
    pub debounce: Duration,
    pub paths: Vec<PathBuf>,
}

pub fn parse_args() -> Result<Config, String> {
    let args: Vec<String> = env::args().skip(1).collect();
    parse_args_from(&args)
}

fn parse_args_from(args: &[String]) -> Result<Config, String> {
    let mut ext_raw = String::from("php");
    let mut poll = false;
    let mut poll_interval = Duration::from_millis(500);
    let mut debounce = Duration::from_millis(300);
    let mut paths: Vec<PathBuf> = Vec::new();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--ext" => {
                i += 1;
                if i >= args.len() {
                    return Err("--ext requires a value".into());
                }
                ext_raw = args[i].clone();
            }
            "--poll" => {
                poll = true;
            }
            "--poll-interval" => {
                i += 1;
                if i >= args.len() {
                    return Err("--poll-interval requires a value".into());
                }
                poll_interval = parse_duration_str(&args[i])?;
            }
            "--debounce" => {
                i += 1;
                if i >= args.len() {
                    return Err("--debounce requires a value".into());
                }
                debounce = parse_duration_str(&args[i])?;
            }
            arg if arg.starts_with("--") => {
                return Err(format!("unknown flag: {}", arg));
            }
            _ => {
                paths.push(PathBuf::from(&args[i]));
            }
        }
        i += 1;
    }

    if paths.is_empty() {
        return Err("at least one path argument is required".into());
    }

    for p in &paths {
        let meta = fs::metadata(p).map_err(|e| format!("{}: {}", p.display(), e))?;
        if !meta.is_dir() {
            return Err(format!("{} is not a directory", p.display()));
        }
    }

    Ok(Config {
        extensions: parse_extensions(&ext_raw),
        poll,
        poll_interval,
        debounce,
        paths,
    })
}

fn parse_duration_str(s: &str) -> Result<Duration, String> {
    if let Some(ms) = s.strip_suffix("ms") {
        ms.parse::<u64>()
            .map(Duration::from_millis)
            .map_err(|e| format!("invalid duration '{}': {}", s, e))
    } else if let Some(secs) = s.strip_suffix('s') {
        secs.parse::<f64>()
            .map(Duration::from_secs_f64)
            .map_err(|e| format!("invalid duration '{}': {}", s, e))
    } else {
        s.parse::<u64>()
            .map(Duration::from_millis)
            .map_err(|e| format!("invalid duration '{}': {}", s, e))
    }
}

pub fn parse_extensions(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| {
            if s.starts_with('.') {
                s.to_string()
            } else {
                format!(".{}", s)
            }
        })
        .collect()
}

pub fn matches_extension(path: &str, exts: &[String]) -> bool {
    exts.iter().any(|ext| path.ends_with(ext.as_str()))
}

pub fn is_ignored(path: &Path) -> bool {
    match path.file_name().and_then(|n| n.to_str()) {
        Some(name) => name.starts_with('.') || name == "vendor" || name == "node_modules",
        None => false,
    }
}

pub fn is_ignored_path(path: &Path) -> bool {
    for component in path.components() {
        if let Component::Normal(name) = component {
            if let Some(s) = name.to_str() {
                if s.starts_with('.') || s == "vendor" || s == "node_modules" {
                    return true;
                }
            }
        }
    }
    false
}

struct Debouncer {
    tx: Option<mpsc::Sender<String>>,
    handle: Option<thread::JoinHandle<()>>,
}

impl Debouncer {
    fn new(debounce: Duration, mut writer: Box<dyn Write + Send>) -> Self {
        let (tx, rx) = mpsc::channel::<String>();

        let handle = thread::spawn(move || {
            let mut pending: HashSet<String> = HashSet::new();

            loop {
                let msg = if pending.is_empty() {
                    match rx.recv() {
                        Ok(path) => Some(path),
                        Err(_) => break,
                    }
                } else {
                    match rx.recv_timeout(debounce) {
                        Ok(path) => Some(path),
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            Self::flush(&mut pending, &mut writer);
                            None
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => {
                            Self::flush(&mut pending, &mut writer);
                            break;
                        }
                    }
                };

                if let Some(path) = msg {
                    pending.insert(path);
                }
            }
        });

        Debouncer {
            tx: Some(tx),
            handle: Some(handle),
        }
    }

    fn send(&self, path: String) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(path);
        }
    }

    fn shutdown(&mut self) {
        self.tx.take();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }

    fn flush(pending: &mut HashSet<String>, writer: &mut Box<dyn Write + Send>) {
        if pending.is_empty() {
            return;
        }
        for p in pending.drain() {
            let _ = writeln!(writer, "changed: {}", p);
        }
        let _ = writer.flush();
    }
}

impl Drop for Debouncer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn scan_dir(root: &Path, extensions: &[String], state: &mut HashMap<PathBuf, SystemTime>) {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if !is_ignored(&path) {
                    stack.push(path);
                }
            } else if matches_extension(&path.to_string_lossy(), extensions) {
                if let Ok(meta) = fs::metadata(&path) {
                    if let Ok(mtime) = meta.modified() {
                        state.insert(path, mtime);
                    }
                }
            }
        }
    }
}

pub fn run_watcher(config: &Config, writer: Box<dyn Write + Send>) -> Result<(), String> {
    let mut debouncer = Debouncer::new(config.debounce, writer);

    let (tx, rx) = mpsc::channel();
    let mut watcher = notify::RecommendedWatcher::new(tx, notify::Config::default())
        .map_err(|e| format!("failed to create watcher: {}", e))?;

    for root in &config.paths {
        let abs = fs::canonicalize(root).map_err(|e| format!("{}: {}", root.display(), e))?;
        watcher
            .watch(&abs, RecursiveMode::Recursive)
            .map_err(|e| format!("failed to watch {}: {}", abs.display(), e))?;
    }

    loop {
        if SHUTDOWN.load(Ordering::Relaxed) {
            break;
        }

        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(Ok(event)) => {
                for path in &event.paths {
                    if is_ignored_path(path) {
                        continue;
                    }
                    if matches!(event.kind, EventKind::Modify(ModifyKind::Metadata(_))) {
                        continue;
                    }
                    if matches!(event.kind, EventKind::Access(_)) {
                        continue;
                    }

                    let path_str = path.to_string_lossy();
                    if matches_extension(&path_str, &config.extensions) {
                        debouncer.send(path_str.into_owned());
                    }
                }
            }
            Ok(Err(e)) => {
                eprintln!("watcher error: {}", e);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    debouncer.shutdown();
    Ok(())
}

pub fn run_poller(config: &Config, writer: Box<dyn Write + Send>) -> Result<(), String> {
    let mut debouncer = Debouncer::new(config.debounce, writer);
    let mut state: HashMap<PathBuf, SystemTime> = HashMap::new();

    for root in &config.paths {
        let abs = fs::canonicalize(root).map_err(|e| format!("{}: {}", root.display(), e))?;
        scan_dir(&abs, &config.extensions, &mut state);
    }

    loop {
        thread::sleep(config.poll_interval);

        if SHUTDOWN.load(Ordering::Relaxed) {
            break;
        }

        let mut current: HashMap<PathBuf, SystemTime> = HashMap::new();
        for root in &config.paths {
            if let Ok(abs) = fs::canonicalize(root) {
                scan_dir(&abs, &config.extensions, &mut current);
            }
        }

        for (path, mtime) in &current {
            match state.get(path) {
                Some(prev) if prev == mtime => {}
                _ => {
                    debouncer.send(path.to_string_lossy().into_owned());
                }
            }
        }

        for path in state.keys() {
            if !current.contains_key(path) {
                debouncer.send(path.to_string_lossy().into_owned());
            }
        }

        state = current;
    }

    debouncer.shutdown();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extensions_single() {
        assert_eq!(parse_extensions("php"), vec![".php"]);
    }

    #[test]
    fn parse_extensions_multiple() {
        assert_eq!(
            parse_extensions("php,blade.php,js"),
            vec![".php", ".blade.php", ".js"]
        );
    }

    #[test]
    fn parse_extensions_with_dots() {
        assert_eq!(parse_extensions(".php,.js"), vec![".php", ".js"]);
    }

    #[test]
    fn parse_extensions_whitespace() {
        assert_eq!(parse_extensions(" php , js "), vec![".php", ".js"]);
    }

    #[test]
    fn parse_extensions_empty() {
        let result: Vec<String> = parse_extensions("");
        assert!(result.is_empty());
    }

    #[test]
    fn parse_extensions_trailing_comma() {
        assert_eq!(parse_extensions("php,"), vec![".php"]);
    }

    #[test]
    fn matches_extension_exact() {
        let exts = vec![".php".to_string()];
        assert!(matches_extension("app/Models/User.php", &exts));
    }

    #[test]
    fn matches_extension_blade_php() {
        let exts = vec![".blade.php".to_string()];
        assert!(matches_extension("resources/views/home.blade.php", &exts));
    }

    #[test]
    fn matches_extension_no_match() {
        let exts = vec![".php".to_string()];
        assert!(!matches_extension("README.md", &exts));
    }

    #[test]
    fn matches_extension_multiple_exts() {
        let exts = vec![".php".to_string(), ".js".to_string()];
        assert!(matches_extension("app.js", &exts));
        assert!(matches_extension("index.php", &exts));
        assert!(!matches_extension("style.css", &exts));
    }

    #[test]
    fn matches_extension_blade_not_plain_php() {
        let exts = vec![".blade.php".to_string()];
        assert!(!matches_extension("app/Models/User.php", &exts));
        assert!(matches_extension("views/home.blade.php", &exts));
    }

    #[test]
    fn is_ignored_dotfile() {
        assert!(is_ignored(Path::new(".env")));
        assert!(is_ignored(Path::new(".gitignore")));
    }

    #[test]
    fn is_ignored_dotdir() {
        assert!(is_ignored(Path::new(".git")));
        assert!(is_ignored(Path::new(".idea")));
    }

    #[test]
    fn is_ignored_vendor() {
        assert!(is_ignored(Path::new("vendor")));
    }

    #[test]
    fn is_ignored_node_modules() {
        assert!(is_ignored(Path::new("node_modules")));
    }

    #[test]
    fn is_ignored_normal_path() {
        assert!(!is_ignored(Path::new("app")));
        assert!(!is_ignored(Path::new("src")));
        assert!(!is_ignored(Path::new("Models")));
    }

    #[test]
    fn is_ignored_path_dotdir_segment() {
        assert!(is_ignored_path(Path::new("app/.git/config")));
        assert!(is_ignored_path(Path::new(".idea/workspace.xml")));
    }

    #[test]
    fn is_ignored_path_vendor_segment() {
        assert!(is_ignored_path(Path::new("vendor/autoload.php")));
        assert!(is_ignored_path(Path::new("app/vendor/file.php")));
    }

    #[test]
    fn is_ignored_path_node_modules_segment() {
        assert!(is_ignored_path(Path::new("node_modules/express/index.js")));
    }

    #[test]
    fn is_ignored_path_normal() {
        assert!(!is_ignored_path(Path::new("app/Models/User.php")));
        assert!(!is_ignored_path(Path::new("config/app.php")));
    }
}
