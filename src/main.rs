use std::io::{self, BufWriter};
use std::process;

fn main() {
    let config = match filewatcher::parse_args() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {}", e);
            process::exit(1);
        }
    };

    filewatcher::install_signal_handlers();

    let writer: Box<dyn io::Write + Send> = Box::new(BufWriter::new(io::stdout()));

    let result = if config.poll {
        filewatcher::run_poller(&config, writer)
    } else {
        filewatcher::run_watcher(&config, writer)
    };

    if let Err(e) = result {
        eprintln!("error: {}", e);
        process::exit(1);
    }
}
