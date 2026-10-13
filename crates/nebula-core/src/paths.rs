use crate::env;
use std::path::{Path, PathBuf};

/// The settings files' names in the data dir. Their loaders, the SETTINGS
/// BUNDLE and `nebula config path` all spell them from here.
pub const CONFIG_FILE_NAME: &str = "config.json";
pub const CONFIG_LOCAL_FILE_NAME: &str = "config.local.json";
pub const PRESETS_FILE_NAME: &str = "agent_presets.json";
pub const SSH_HOSTS_FILE_NAME: &str = "ssh_hosts.json";

/// Where the runtime dir lands when neither override nor `XDG_RUNTIME_DIR`
/// is set. World-writable, so the per-uid subdir is what carries mode 0700.
#[cfg(unix)]
const FALLBACK_RUNTIME_ROOT: &str = "/tmp";

/// Runtime dir holding the socket + pidfile. Mode 0700 — this is the auth
/// boundary, same model as tmux. `NEBULA_RUNTIME_DIR` overrides (tests,
/// parallel instances).
#[cfg(unix)]
pub fn runtime_dir() -> PathBuf {
    if let Some(dir) = env::non_empty(env::RUNTIME_DIR) {
        return PathBuf::from(dir);
    }
    if let Some(dir) = xdg_runtime_dir() {
        return dir.join("nebula");
    }
    fallback_runtime_dir()
}

/// Runtime dir holding the pidfile and buildstamp: `%LOCALAPPDATA%\bro\run`.
/// The daemon's endpoint is a named pipe, not a file in here, but its name
/// is derived from this dir ([`pipe_name`]), so `NEBULA_RUNTIME_DIR` still
/// isolates tests and parallel instances. The per-user profile is the
/// access boundary; there is no mode to set.
#[cfg(windows)]
pub fn runtime_dir() -> PathBuf {
    if let Some(dir) = env::non_empty(env::RUNTIME_DIR) {
        return PathBuf::from(dir);
    }
    windows_app_dir().join("run")
}

/// `%LOCALAPPDATA%\bro`, the root of everything the Windows build keeps.
/// Local rather than roaming: the database, logs and pidfile describe this
/// machine's sessions and must not follow a roaming profile elsewhere.
#[cfg(windows)]
fn windows_app_dir() -> PathBuf {
    env::non_empty("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| env::home_dir().map(|home| home.join("AppData").join("Local")))
        .unwrap_or_else(std::env::temp_dir)
        .join("bro")
}

/// The named pipe the daemon listens on for `runtime_dir`, as `user`:
/// `\\.\pipe\bro-<user>-<hash>`. Pipe names are machine-global, so the user
/// keeps two accounts apart and the hash of the runtime dir keeps apart two
/// instances one user runs (`NEBULA_RUNTIME_DIR`). The hash is FNV-1a over
/// the path's bytes: fixed across builds and toolchains, so a client and a
/// daemon from different builds still agree on the name.
pub fn pipe_name_for(user: &str, runtime_dir: &Path) -> String {
    const PRIME: u64 = 0x100_0000_01b3;
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in runtime_dir.to_string_lossy().bytes() {
        hash = (hash ^ u64::from(byte)).wrapping_mul(PRIME);
    }
    // Pipe names take any character but `\`; keep the user part tame anyway
    // so the name reads cleanly in tools that list pipes.
    let user: String = user
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!(r"\\.\pipe\bro-{user}-{:08x}", hash as u32)
}

/// [`pipe_name_for`] the current user and a socket path: the pipe stands in
/// for the socket that would sit at `socket`, so its dir picks the pipe.
pub fn pipe_name_for_socket(socket: &Path) -> String {
    let user = env::non_empty("USERNAME")
        .or_else(|| env::non_empty("USER"))
        .unwrap_or_else(|| "user".into());
    pipe_name_for(&user, socket.parent().unwrap_or(Path::new("")))
}

/// The daemon's named pipe for this user and [`runtime_dir`].
pub fn pipe_name() -> String {
    pipe_name_for_socket(&socket_path())
}

#[cfg(unix)]
fn xdg_runtime_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(env::non_empty("XDG_RUNTIME_DIR")?);
    // XDG_RUNTIME_DIR is supposed to be created by the login manager before
    // applications start. WSL shells can inherit `/run/user/$UID` without
    // systemd/PAM having made it, and an unprivileged daemon cannot create
    // that parent under `/run`; use the same private /tmp fallback tmux-style
    // tools use when the XDG dir is absent.
    dir.is_dir().then_some(dir)
}

#[cfg(unix)]
fn fallback_runtime_dir() -> PathBuf {
    let uid = libc_geteuid();
    Path::new(FALLBACK_RUNTIME_ROOT).join(format!("nebula-{uid}"))
}

// Avoid a libc dependency in this dep-light crate for one call.
#[cfg(unix)]
fn libc_geteuid() -> u32 {
    extern "C" {
        fn geteuid() -> u32;
    }
    unsafe { geteuid() }
}

pub fn socket_path() -> PathBuf {
    runtime_dir().join("daemon.sock")
}

pub fn pidfile_path() -> PathBuf {
    runtime_dir().join("daemon.pid")
}

/// Fingerprint of the binary the running daemon was launched from, written
/// by the daemon at startup. Installers compare it against the binary they
/// just installed to tell an up-to-date daemon from a stale one.
pub fn buildstamp_path() -> PathBuf {
    runtime_dir().join("daemon.build")
}

/// `<pid> <version>` of a daemon that can restart in place onto a new
/// binary, keeping its sessions. A daemon from before IN-PLACE RESTARTS
/// never writes it, which is how a client knows not to ask.
pub fn restart_capability_path() -> PathBuf {
    runtime_dir().join("daemon.restart")
}

/// A client's request for an IN-PLACE RESTART: which binary to restart
/// onto. The daemon reads and removes it when signalled.
pub fn restart_request_path() -> PathBuf {
    runtime_dir().join("restart.request")
}

/// How the last IN-PLACE RESTART went, written by whichever image of the
/// daemon finished it: the old one on a refusal, the new one on success.
pub fn restart_result_path() -> PathBuf {
    runtime_dir().join("restart.result")
}

/// The live sessions an IN-PLACE RESTART carries across the exec, read
/// and removed by the new image as it boots.
pub fn restart_state_path() -> PathBuf {
    runtime_dir().join("restart.state")
}

/// The platform's per-user dirs for this app (`~/Library/Application
/// Support/dev.nebula.nebula` on macOS, `~/.local/share/nebula` on Linux).
#[cfg(unix)]
fn project_dirs() -> Option<directories::ProjectDirs> {
    directories::ProjectDirs::from("dev", "nebula", "nebula")
}

#[cfg(unix)]
pub fn data_dir() -> PathBuf {
    if let Some(dir) = env::non_empty(env::DATA_DIR) {
        return PathBuf::from(dir);
    }
    project_dirs()
        .map(|d| d.data_dir().to_path_buf())
        .unwrap_or_else(|| env::home_dir().unwrap_or_default().join(".nebula"))
}

/// `%LOCALAPPDATA%\bro\data`, beside the runtime dir, rather than the
/// roaming `%APPDATA%` dir `ProjectDirs` would pick: the database holds
/// this machine's sessions and checkouts.
#[cfg(windows)]
pub fn data_dir() -> PathBuf {
    if let Some(dir) = env::non_empty(env::DATA_DIR) {
        return PathBuf::from(dir);
    }
    windows_app_dir().join("data")
}

pub fn db_path() -> PathBuf {
    data_dir().join("nebula.db")
}

/// User settings file (JSON) — the portable layer, which backups and
/// `nebula ssh` carry. Lives beside the DB so `NEBULA_DATA_DIR` isolates it
/// for tests and parallel instances too; `NEBULA_CONFIG_FILE` moves this one
/// file alone (a leading `~/` is expanded, since a quoted value never is).
pub fn config_path() -> PathBuf {
    match env::non_empty(env::CONFIG_FILE) {
        Some(file) => match file.strip_prefix("~/") {
            Some(rest) => env::home_dir().unwrap_or_default().join(rest),
            None => PathBuf::from(file),
        },
        None => data_dir().join(CONFIG_FILE_NAME),
    }
}

/// This machine's settings, layered over [`config_path`] key by key: never
/// exported, forwarded or overwritten by an import. Stays in the data dir
/// when `NEBULA_CONFIG_FILE` moves the portable file.
pub fn config_local_path() -> PathBuf {
    data_dir().join(CONFIG_LOCAL_FILE_NAME)
}

/// Claude Code's config dir: `$CLAUDE_CONFIG_DIR`, else `~/.claude`.
pub fn claude_config_dir() -> Option<PathBuf> {
    env::non_empty(env::CLAUDE_CONFIG_DIR)
        .map(PathBuf::from)
        .or_else(|| env::home_dir().map(|home| home.join(".claude")))
}

/// Logs live under the data dir's `state` on Windows, which has no separate
/// state dir.
#[cfg(windows)]
pub fn log_dir() -> PathBuf {
    data_dir().join("state")
}

#[cfg(unix)]
pub fn log_dir() -> PathBuf {
    // Tests and parallel instances override the data dir; keep their logs
    // beside their data instead of the real user's state dir.
    if env::non_empty(env::DATA_DIR).is_some() {
        return data_dir().join("state");
    }
    project_dirs()
        .map(|d| {
            d.state_dir()
                .map(|s| s.to_path_buf())
                .unwrap_or_else(|| d.data_dir().join("state"))
        })
        .unwrap_or_else(|| data_dir().join("state"))
}

pub fn daemon_log_path() -> PathBuf {
    log_dir().join("daemon.log")
}

pub fn tui_log_path() -> PathBuf {
    log_dir().join("tui.log")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Restores an env var to what it was when the guard was made, so a
    /// failed assertion can't leak an override into the next test.
    struct EnvRestore(&'static str, Option<std::ffi::OsString>);
    impl EnvRestore {
        fn new(var: &'static str) -> Self {
            Self(var, std::env::var_os(var))
        }
    }
    impl Drop for EnvRestore {
        fn drop(&mut self) {
            match &self.1 {
                Some(v) => std::env::set_var(self.0, v),
                None => std::env::remove_var(self.0),
            }
        }
    }

    // One test owns both override vars: env is process-global, and splitting
    // this into several `#[test]`s would race them across threads.
    #[test]
    fn overrides_win_only_when_non_empty() {
        let _restore = (
            EnvRestore::new(crate::env::RUNTIME_DIR),
            EnvRestore::new(crate::env::DATA_DIR),
            EnvRestore::new(crate::env::CONFIG_FILE),
            EnvRestore::new("XDG_RUNTIME_DIR"),
        );
        std::env::remove_var("XDG_RUNTIME_DIR");
        std::env::remove_var(crate::env::CONFIG_FILE);
        let runtime = std::env::temp_dir().join(format!("nebula-paths-rt-{}", std::process::id()));
        let data = std::env::temp_dir().join(format!("nebula-paths-data-{}", std::process::id()));

        std::env::set_var(crate::env::RUNTIME_DIR, &runtime);
        std::env::set_var(crate::env::DATA_DIR, &data);
        assert_eq!(runtime_dir(), runtime);
        assert_eq!(socket_path(), runtime.join("daemon.sock"));
        assert_eq!(pidfile_path(), runtime.join("daemon.pid"));
        assert_eq!(buildstamp_path(), runtime.join("daemon.build"));
        assert_eq!(data_dir(), data);
        assert_eq!(db_path(), data.join("nebula.db"));
        assert_eq!(config_path(), data.join("config.json"));
        assert_eq!(config_local_path(), data.join("config.local.json"));
        // Logs follow the data override so isolated instances keep their
        // logs beside their state.
        assert_eq!(log_dir(), data.join("state"));
        assert_eq!(daemon_log_path(), data.join("state").join("daemon.log"));
        assert_eq!(tui_log_path(), data.join("state").join("tui.log"));

        // The config file override moves that one file; the local layer and
        // everything else stay in the data dir.
        let dotfile = std::env::temp_dir().join("dotfiles").join("nebula.json");
        std::env::set_var(crate::env::CONFIG_FILE, &dotfile);
        assert_eq!(config_path(), dotfile);
        assert_eq!(config_local_path(), data.join("config.local.json"));
        assert_eq!(db_path(), data.join("nebula.db"));
        std::env::set_var(crate::env::CONFIG_FILE, "~/dotfiles/nebula.json");
        assert_eq!(
            config_path(),
            crate::env::home_dir()
                .unwrap()
                .join("dotfiles")
                .join("nebula.json")
        );
        std::env::set_var(crate::env::CONFIG_FILE, "");
        assert_eq!(config_path(), data.join("config.json"));

        std::env::set_var(crate::env::RUNTIME_DIR, "");
        std::env::set_var(crate::env::DATA_DIR, "");
        assert_ne!(runtime_dir(), runtime, "empty override must fall through");
        assert_ne!(data_dir(), data, "empty override must fall through");
        assert_ne!(log_dir(), data.join("state"));
        assert!(
            runtime_dir().is_absolute() && data_dir().is_absolute(),
            "defaults are absolute: {} / {}",
            runtime_dir().display(),
            data_dir().display()
        );

        #[cfg(unix)]
        {
            let xdg =
                std::env::temp_dir().join(format!("nebula-paths-xdg-{}", std::process::id()));
            std::fs::create_dir_all(&xdg).unwrap();
            std::env::set_var("XDG_RUNTIME_DIR", &xdg);
            assert_eq!(runtime_dir(), xdg.join("nebula"));

            std::fs::remove_dir_all(&xdg).unwrap();
            assert_eq!(runtime_dir(), fallback_runtime_dir());
        }
        #[cfg(windows)]
        {
            // Both defaults sit side by side under one `bro` dir.
            assert_eq!(runtime_dir().parent(), data_dir().parent());
            assert!(runtime_dir().ends_with(r"bro\run"), "{}", runtime_dir().display());
            assert_eq!(log_dir(), data_dir().join("state"));
        }
    }

    #[test]
    fn pipe_name_is_per_user_and_per_runtime_dir() {
        let a = Path::new(r"C:\Users\jay\AppData\Local\bro\run");
        let b = Path::new(r"C:\Temp\test-run");
        let name = pipe_name_for("jay", a);
        assert!(name.starts_with(r"\\.\pipe\bro-jay-"), "{name}");
        assert_eq!(name.len(), r"\\.\pipe\bro-jay-".len() + 8, "{name}");
        // Stable for the same inputs, so client and daemon agree.
        assert_eq!(name, pipe_name_for("jay", a));
        assert_ne!(name, pipe_name_for("jay", b), "runtime dirs isolate");
        assert_ne!(name, pipe_name_for("ann", a), "users isolate");
        // A user name with characters a pipe listing would mangle.
        let odd = pipe_name_for(r"DOMAIN\Jay Smith", a);
        assert!(odd.starts_with(r"\\.\pipe\bro-DOMAIN_Jay_Smith-"), "{odd}");
        assert_eq!(odd.matches('\\').count(), 4, "only the prefix's: {odd}");
        // The socket path's dir picks the pipe: tests that put a socket in
        // a temp dir get a pipe of their own.
        assert_eq!(
            pipe_name_for_socket(&b.join("daemon.sock")),
            pipe_name_for_socket(&b.join("other.sock"))
        );
    }
}
