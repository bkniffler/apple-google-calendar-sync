//! Supervises the `insync` CLI: a background `daemon --apply` child, one-shot
//! syncs, and terminal launches for the TUI dashboard and setup wizard.
//!
//! The tray deliberately shells out to the existing CLI instead of embedding the
//! engine and providers, so all credential/secret-store/provider logic lives in
//! one place and the tray stays a thin control surface.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

/// Locate the `insync` CLI binary.
///
/// Order: `INSYNC_BIN` env override, a sibling of the current executable, then
/// bare `insync` resolved through `PATH`.
pub fn locate_insync_binary() -> PathBuf {
    if let Some(path) = std::env::var_os("INSYNC_BIN") {
        return PathBuf::from(path);
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let sibling = dir.join(if cfg!(windows) {
            "insync.exe"
        } else {
            "insync"
        });
        if sibling.exists() {
            return sibling;
        }
    }
    PathBuf::from("insync")
}

pub struct Supervisor {
    binary: PathBuf,
    config_path: PathBuf,
    log_dir: PathBuf,
    daemon: Option<Child>,
    last_sync: Option<Child>,
}

impl Supervisor {
    pub fn new(binary: PathBuf, config_path: PathBuf, log_dir: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(&log_dir);
        Self {
            binary,
            config_path,
            log_dir,
            daemon: None,
            last_sync: None,
        }
    }

    fn base_command(&self) -> Command {
        let mut cmd = Command::new(&self.binary);
        cmd.arg("--config").arg(&self.config_path);
        cmd
    }

    fn log_file(&self, name: &str) -> Option<File> {
        File::create(self.log_dir.join(name)).ok()
    }

    /// Whether the background daemon child is currently alive.
    pub fn background_running(&mut self) -> bool {
        if let Some(child) = self.daemon.as_mut() {
            match child.try_wait() {
                Ok(Some(_)) => {
                    self.daemon = None;
                    false
                }
                Ok(None) => true,
                Err(_) => true,
            }
        } else {
            false
        }
    }

    /// Whether a manual one-shot sync child is still running.
    pub fn manual_sync_running(&mut self) -> bool {
        if let Some(child) = self.last_sync.as_mut() {
            match child.try_wait() {
                Ok(Some(_)) => {
                    self.last_sync = None;
                    false
                }
                Ok(None) => true,
                Err(_) => true,
            }
        } else {
            false
        }
    }

    /// Start `insync daemon --apply` if it is not already running.
    pub fn start_background(&mut self) -> std::io::Result<()> {
        if self.background_running() {
            return Ok(());
        }
        let mut cmd = self.base_command();
        cmd.arg("daemon").arg("--apply");
        if let Some(log) = self.log_file("daemon.log")
            && let Ok(err) = log.try_clone()
        {
            cmd.stdout(Stdio::from(log)).stderr(Stdio::from(err));
        }
        cmd.stdin(Stdio::null());
        self.daemon = Some(cmd.spawn()?);
        Ok(())
    }

    /// Stop the background daemon child if running.
    pub fn stop_background(&mut self) {
        if let Some(mut child) = self.daemon.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    /// Run a single `insync sync --apply` in the background, logging output.
    ///
    /// Intended to be used only while the background daemon is paused, so there
    /// is a single writer against the calendars at a time.
    pub fn run_sync_once(&mut self) -> std::io::Result<()> {
        if self.manual_sync_running() {
            return Ok(());
        }
        let mut cmd = self.base_command();
        cmd.arg("sync").arg("--apply");
        if let Some(log) = self.log_file("sync.log")
            && let Ok(err) = log.try_clone()
        {
            cmd.stdout(Stdio::from(log)).stderr(Stdio::from(err));
        }
        cmd.stdin(Stdio::null());
        self.last_sync = Some(cmd.spawn()?);
        Ok(())
    }

    /// Open the terminal dashboard (`insync tui`) in a new terminal window.
    pub fn open_dashboard(&self) -> std::io::Result<()> {
        launch_in_terminal(&self.binary, &["--config", &self.config_str(), "tui"])
    }

    /// Open the interactive setup wizard (`insync setup --interactive`).
    pub fn open_setup(&self) -> std::io::Result<()> {
        launch_in_terminal(
            &self.binary,
            &["--config", &self.config_str(), "setup", "--interactive"],
        )
    }

    fn config_str(&self) -> String {
        self.config_path.to_string_lossy().into_owned()
    }
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        self.stop_background();
    }
}

/// Launch `binary args...` inside a native terminal window for the current OS.
///
/// Interactive subcommands (the TUI and setup wizard) need a real TTY, so they
/// must run in a terminal emulator rather than as a piped child.
fn launch_in_terminal(binary: &Path, args: &[&str]) -> std::io::Result<()> {
    let bin = binary.to_string_lossy();
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    let quoted = std::iter::once(quote(&bin))
        .chain(args.iter().map(|a| quote(a)))
        .collect::<Vec<_>>()
        .join(" ");

    #[cfg(target_os = "macos")]
    {
        let script = format!(
            "tell application \"Terminal\" to do script \"{}\"",
            quoted.replace('"', "\\\"")
        );
        Command::new("osascript").arg("-e").arg(script).spawn()?;
        return Ok(());
    }

    #[cfg(target_os = "windows")]
    {
        // `start` is a cmd builtin; `cmd /k` keeps the window open after exit.
        let mut cmd = Command::new("cmd");
        cmd.args(["/c", "start", "", "cmd", "/k", &quoted]);
        cmd.spawn()?;
        return Ok(());
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        // Try common Linux terminal emulators. Their "run a command" flag
        // differs: gnome-terminal/tilix use `--`, most others use `-e`.
        let candidates: &[(&str, &[&str])] = &[
            ("x-terminal-emulator", &["-e"]),
            ("gnome-terminal", &["--"]),
            ("konsole", &["-e"]),
            ("xfce4-terminal", &["-e"]),
            ("tilix", &["--"]),
            ("alacritty", &["-e"]),
            ("kitty", &[]),
            ("xterm", &["-e"]),
        ];
        let mut full = Vec::new();
        full.push(bin.to_string());
        full.extend(args.iter().map(|a| a.to_string()));

        for (term, flag) in candidates {
            if which(term).is_none() {
                continue;
            }
            let mut cmd = Command::new(term);
            cmd.args(*flag);
            cmd.args(&full);
            if cmd.spawn().is_ok() {
                return Ok(());
            }
        }
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no supported terminal emulator found (set INSYNC_TERMINAL or install one of \
             gnome-terminal, konsole, xfce4-terminal, alacritty, kitty, xterm)",
        ));
    }

    #[allow(unreachable_code)]
    Ok(())
}

/// Minimal shell-quoting for embedding a path/arg in a `do script`/`cmd` string.
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn quote(value: &str) -> String {
    if value.is_empty() || value.contains(|c: char| c.is_whitespace() || c == '"') {
        format!("\"{}\"", value.replace('"', "\\\""))
    } else {
        value.to_string()
    }
}

/// Resolve an executable name against `PATH`.
#[cfg(all(unix, not(target_os = "macos")))]
fn which(name: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths).find_map(|dir| {
        let candidate = dir.join(name);
        if candidate.is_file() {
            Some(candidate)
        } else {
            None
        }
    })
}
