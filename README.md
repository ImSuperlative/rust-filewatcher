# filewatcher

Lightweight file watcher that outputs change events to stdout. Designed as a companion binary for Laravel Horizon worker restarts, replacing Node.js + chokidar.

Uses OS-native file system events (FSEvents on macOS, inotify on Linux). Single static binary, zero runtime dependencies.

## Usage

```bash
filewatcher [flags] <path> [<path>...]
```

### Flags

| Flag | Default | Description |
|------|---------|-------------|
| `--ext` | `php` | Comma-separated extensions to watch |
| `--poll` | off | Use polling instead of OS-native events |
| `--poll-interval` | `500ms` | Polling interval |
| `--debounce` | `300ms` | Debounce window for coalescing changes |

### Examples

```bash
filewatcher --ext php,blade.php app/ config/ routes/

filewatcher --poll --poll-interval 1s app/
```

### Output

One line per change event (after debounce):

```
changed: app/Jobs/ProcessOrder.php
```

Exits `0` on SIGTERM/SIGINT, `1` on error.

## Build

```bash
cargo build --release
```

## Install via PHP

The Laravel package downloads the correct binary automatically during `composer install`. See `bin/install.php`.

## Targets

- `x86_64-unknown-linux-musl`
- `aarch64-unknown-linux-musl`
- `x86_64-apple-darwin`
- `aarch64-apple-darwin`
