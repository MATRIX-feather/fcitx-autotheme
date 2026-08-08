// SPDX-FileCopyrightText: 2022~2022 CSSlayer <wengxt@gmail.com>
// SPDX-FileCopyrightText: 2026 Fcitx Breeze Theme Generator Authors
//
// SPDX-License-Identifier: GPL-2.0-or-later
//
//! CLI entry point — a drop-in replacement for `fcitx5-plasma-theme-generator`.
//!
//! Options match the original:
//! - `-t, --theme <name>`: Plasma theme name (defaults to the global theme)
//! - `-o, --output <path>`: output path (default: `~/.local/share/fcitx5/themes/plasma`)
//! - `--fd <fd>`: monitor mode; watch a pipe for EOF and notify after each
//!   regeneration (used by fcitx5's ClassicUI plasma theme watchdog)

use std::os::unix::io::RawFd;
use std::time::Duration;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const APP_NAME: &str = "fcitx5-plasma-theme-generator";

const USAGE: &str = "\
Usage: fcitx5-plasma-theme-generator [options]
Generate Fcitx 5 Classic UI Theme based on Plasma theme

Options:
  -t, --theme <name>    Plasma theme name
  -o, --output <path>   Output path
  -h, --help            Show this help
  -v, --version         Show version
";

struct Args {
    theme: Option<String>,
    output: Option<String>,
    fd: Option<RawFd>,
}

fn parse_args() -> Result<Args, i32> {
    let mut args = Args {
        theme: None,
        output: None,
        fd: None,
    };
    let mut iter = std::env::args().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                return Err(0);
            }
            "-v" | "--version" => {
                println!("{APP_NAME} {VERSION}");
                return Err(0);
            }
            "-t" | "--theme" => {
                args.theme = Some(iter.next().unwrap_or_default());
            }
            "-o" | "--output" => {
                args.output = Some(iter.next().unwrap_or_default());
            }
            "--fd" => {
                let v = iter.next().unwrap_or_default();
                args.fd = v.parse().ok();
            }
            other if other.starts_with("--theme=") => {
                args.theme = Some(other["--theme=".len()..].to_string());
            }
            other if other.starts_with("--output=") => {
                args.output = Some(other["--output=".len()..].to_string());
            }
            other if other.starts_with("--fd=") => {
                args.fd = other["--fd=".len()..].parse().ok();
            }
            _ => {
                eprintln!("Unknown option: {arg}\n\n{USAGE}");
                return Err(2);
            }
        }
    }
    Ok(args)
}

fn fd_is_valid(fd: RawFd) -> bool {
    unsafe { libc::fcntl(fd, libc::F_GETFD) != -1 }
}

fn run_once(theme: Option<&str>, output: &str) -> bool {
    let theme = match fcitx_breeze_theme_generator::Theme::new(theme.unwrap_or("")) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{e}");
            return false;
        }
    };
    match fcitx_breeze_theme_generator::generate_theme(&theme, std::path::Path::new(output)) {
        Ok(_) => {
            eprintln!("Will write new themes to: {output}");
            true
        }
        Err(e) => {
            eprintln!("Failed to generate theme: {e}");
            false
        }
    }
}

/// Monitor mode: after each regeneration, notify via a 1-byte write; quit on
/// pipe EOF (parent closed the pipe). Also regenerate when the theme changes
/// (approximated by polling the theme directory mtime).
fn monitor_mode(fd: RawFd, output: &str) -> ! {
    loop {
        // Notify after the initial generation.
        write_byte(fd);
        run_once(None, output);

        // Wait for either pipe activity or theme file changes.
        let mut read_fd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let mut last_mtime = theme_mtime();
        loop {
            let ret = unsafe { libc::poll(&mut read_fd, 1, 500) };
            if ret < 0 {
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }
            if ret > 0 && (read_fd.revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR)) != 0 {
                if !drain_fd(fd) {
                    std::process::exit(0);
                }
                break;
            }
            let m = theme_mtime();
            if m != last_mtime {
                last_mtime = m;
                break;
            }
        }
    }
}

/// Drain readable bytes from `fd`; returns false when EOF or error (quit).
fn drain_fd(fd: RawFd) -> bool {
    let mut buf = [0u8; 16];
    loop {
        let n = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut _, buf.len()) };
        if n == 0 {
            return false;
        }
        if n < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::WouldBlock {
                return true;
            }
            return false;
        }
        if (n as usize) < buf.len() {
            return true;
        }
    }
}

/// Latest mtime across all theme directories (crude but sufficient change
/// detector for monitor mode).
fn theme_mtime() -> Option<std::time::SystemTime> {
    use std::time::SystemTime;
    let home = std::env::var_os("HOME").unwrap_or_default();
    let mut latest: Option<SystemTime> = None;
    let mut roots = Vec::new();
    if let Some(h) = std::env::var_os("XDG_DATA_HOME") {
        roots.push(std::path::PathBuf::from(h));
    } else {
        roots.push(std::path::Path::new(&home).join(".local/share"));
    }
    roots.push(std::path::PathBuf::from("/usr/share"));

    for root in roots {
        let dir = root.join("plasma/desktoptheme");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(md) = entry.metadata() else {
                continue;
            };
            if let Ok(mtime) = md.modified() {
                if latest.is_none_or(|l| mtime > l) {
                    latest = Some(mtime);
                }
            }
        }
    }
    latest
}

fn write_byte(fd: RawFd) {
    let buf = [0u8];
    unsafe {
        libc::write(fd, buf.as_ptr() as *const _, 1);
    }
}

fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(code) => std::process::exit(code),
    };

    let fd_valid = args.fd.map(fd_is_valid).unwrap_or(false);

    if !fd_valid && args.fd.is_some() {
        std::process::exit(1);
    }

    let output = args.output.clone().unwrap_or_else(|| {
        fcitx_breeze_theme_generator::generator::default_output_path()
            .to_string_lossy()
            .into_owned()
    });

    if fd_valid {
        monitor_mode(args.fd.unwrap(), &output);
    } else {
        let ok = run_once(args.theme.as_deref(), &output);
        std::process::exit(if ok { 0 } else { 1 });
    }
}
