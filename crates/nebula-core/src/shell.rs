//! POSIX shell helpers the daemon, the TUI and the CLI share — and, on
//! Windows, their stand-ins: which shell a terminal runs, and how a program
//! named the way an agent harness names it (`claude`, `copilot`) is started
//! without a POSIX shell to resolve it.

/// Single-quote `arg` for a POSIX shell: `it's` → `'it'\''s'`. Inside
/// single quotes the only character needing an escape is `'` itself.
pub fn single_quote(arg: &str) -> String {
    format!("'{}'", arg.replace('\'', "'\\''"))
}

/// The user's shell: `$SHELL`, else `/bin/sh`.
#[cfg(unix)]
pub fn user_shell() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into())
}

/// The user's shell: `BRO_SHELL`, else PowerShell 7 (`pwsh.exe`) when it is
/// on PATH, else Windows PowerShell, else `%COMSPEC%` (`cmd.exe`).
#[cfg(windows)]
pub fn user_shell() -> String {
    if let Some(shell) = crate::env::non_empty(crate::env::SHELL_OVERRIDE) {
        return shell;
    }
    for candidate in ["pwsh", "powershell"] {
        if let Some(path) = resolve_program(candidate) {
            return path.display().to_string();
        }
    }
    comspec()
}

/// `%COMSPEC%`, else plain `cmd.exe` for the loader to find.
#[cfg(windows)]
pub fn comspec() -> String {
    crate::env::non_empty("COMSPEC").unwrap_or_else(|| "cmd.exe".into())
}

/// The shells [`shell_line_args`] knows how to hand a command line to.
#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShellKind {
    /// `pwsh` and Windows `powershell`: `-Command <line>`.
    PowerShell,
    /// `cmd.exe`: `/d /s /c <line>`.
    Cmd,
    /// Anything else (Git Bash, a WSL `bash.exe`): POSIX `-c <line>`.
    Posix,
}

#[cfg(windows)]
fn shell_kind(shell: &str) -> ShellKind {
    let stem = std::path::Path::new(shell)
        .file_stem()
        .map(|s| s.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    match stem.as_str() {
        "pwsh" | "powershell" => ShellKind::PowerShell,
        "cmd" => ShellKind::Cmd,
        _ => ShellKind::Posix,
    }
}

/// Arguments that make `shell` run `line` (a RUN TERMINAL's `.nebula.json`
/// `run`, pipes and `&&` and all) and exit with its status.
#[cfg(windows)]
pub fn shell_line_args(shell: &str, line: &str) -> Vec<String> {
    let lead: &[&str] = match shell_kind(shell) {
        ShellKind::PowerShell => &["-NoLogo", "-Command"],
        ShellKind::Cmd => &["/d", "/s", "/c"],
        ShellKind::Posix => &["-l", "-c"],
    };
    lead.iter()
        .map(|s| s.to_string())
        .chain([line.to_string()])
        .collect()
}

/// Arguments for an interactive session of `shell` in a plain terminal —
/// the Windows counterpart of the `-l` login flag unix terminals get.
#[cfg(windows)]
pub fn interactive_shell_args(shell: &str) -> Vec<String> {
    match shell_kind(shell) {
        ShellKind::PowerShell => vec!["-NoLogo".into()],
        ShellKind::Cmd => vec![],
        ShellKind::Posix => vec!["-l".into()],
    }
}

/// Env var that carries a `cmd.exe` command line past the argv quoting
/// every spawn applies (see [`launch`]).
#[cfg(windows)]
pub const CMD_LINE_VAR: &str = "BRO_CMDLINE";

/// The extensions a script launch understands beyond `%PATHEXT%`, which
/// leaves out PowerShell scripts.
#[cfg(windows)]
const SCRIPT_EXTS: &[&str] = &[".PS1"];

/// Find `name` the way a typed command would: as given when it names a
/// path, else in each `path` dir in turn, trying the `pathext` extensions
/// (then `.ps1`) on a bare name. A file without one of those extensions is
/// never a match — npm drops an extensionless POSIX script beside each
/// `.cmd` shim, and that is not something Windows can start.
#[cfg(windows)]
pub fn resolve_program_in(
    name: &str,
    path: &std::ffi::OsStr,
    pathext: &str,
) -> Option<std::path::PathBuf> {
    use std::path::{Path, PathBuf};
    let exts: Vec<String> = pathext
        .split(';')
        .map(str::trim)
        .filter(|e| e.starts_with('.') && e.len() > 1)
        .map(str::to_ascii_uppercase)
        .chain(SCRIPT_EXTS.iter().map(|e| e.to_string()))
        .collect();
    let has_known_ext = |p: &Path| {
        p.extension()
            .map(|e| format!(".{}", e.to_string_lossy().to_ascii_uppercase()))
            .is_some_and(|e| exts.contains(&e))
    };
    let try_at = |base: PathBuf| -> Option<PathBuf> {
        if has_known_ext(&base) && base.is_file() {
            return Some(base);
        }
        exts.iter()
            .map(|ext| {
                let mut with = base.clone().into_os_string();
                with.push(ext.to_ascii_lowercase());
                PathBuf::from(with)
            })
            .find(|candidate| candidate.is_file())
    };
    if name.is_empty() {
        return None;
    }
    let as_path = Path::new(name);
    if as_path.is_absolute() || name.contains(['/', '\\']) {
        return try_at(as_path.to_path_buf());
    }
    std::env::split_paths(path)
        .filter(|dir| !dir.as_os_str().is_empty())
        .find_map(|dir| try_at(dir.join(name)))
}

/// [`resolve_program_in`] against this process's `PATH` and `PATHEXT`.
#[cfg(windows)]
pub fn resolve_program(name: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let pathext = crate::env::non_empty("PATHEXT").unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".into());
    resolve_program_in(name, &path, &pathext)
}

/// How to start a program on Windows: the executable to create, its argv,
/// and env vars the launch needs on top of the session's own.
#[cfg(windows)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

/// Turn `program args…` into something `CreateProcess` can start, with no
/// POSIX shell to resolve it:
///
/// - the program is resolved through PATH + PATHEXT ([`resolve_program`]);
///   one that does not resolve is passed through for the spawn to report;
/// - an `.exe` (or `.com`) runs directly;
/// - a `.cmd`/`.bat` runs under `cmd.exe /d /s /c`. Every spawn quotes its
///   argv the MSVCRT way (`\"` for a quote), which `cmd.exe` does not read,
///   so the command line rides in [`CMD_LINE_VAR`] instead and the argv
///   only names it (`%BRO_CMDLINE%`): cmd expands it once and parses the
///   result with its own quoting. Each word is double-quoted there, which
///   makes `&`, `|`, `<`, `>` and `^` literal. What cmd cannot carry
///   literally — `%` (expanded again by the script), `"`, and line breaks —
///   is refused rather than mangled;
/// - a `.ps1` runs under `pwsh -NoProfile -ExecutionPolicy Bypass -File`,
///   or Windows PowerShell when there is no `pwsh`.
///
/// A `cmd.exe /d /s /c <line>` launch (a RUN TERMINAL under cmd) takes the
/// same env route for its line.
#[cfg(windows)]
pub fn launch(program: &str, args: &[String]) -> Result<Launch, String> {
    use std::path::Path;
    let is_cmd = |p: &str| {
        Path::new(p)
            .file_stem()
            .is_some_and(|s| s.eq_ignore_ascii_case("cmd"))
    };
    if is_cmd(program)
        && args.len() == 4
        && args[..3]
            .iter()
            .zip(["/d", "/s", "/c"])
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
    {
        return cmd_line_launch(program.to_string(), args[3].clone());
    }
    let Some(resolved) = resolve_program(program) else {
        return Ok(Launch {
            program: program.to_string(),
            args: args.to_vec(),
            env: vec![],
        });
    };
    let ext = resolved
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let path = resolved.display().to_string();
    match ext.as_str() {
        "cmd" | "bat" => {
            let mut line = cmd_quote(&path)?;
            for arg in args {
                line.push(' ');
                line.push_str(&cmd_quote(arg)?);
            }
            cmd_line_launch(comspec(), line)
        }
        "ps1" => {
            let host = resolve_program("pwsh")
                .or_else(|| resolve_program("powershell"))
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "powershell.exe".into());
            let mut argv: Vec<String> = ["-NoLogo", "-NoProfile", "-ExecutionPolicy", "Bypass"]
                .iter()
                .map(|s| s.to_string())
                .collect();
            argv.push("-File".into());
            argv.push(path);
            argv.extend(args.iter().cloned());
            Ok(Launch {
                program: host,
                args: argv,
                env: vec![],
            })
        }
        _ => Ok(Launch {
            program: path,
            args: args.to_vec(),
            env: vec![],
        }),
    }
}

/// `cmd.exe /d /s /c %BRO_CMDLINE%` with `line` in that variable. `/s`
/// strips quotes only when the text after `/c` opens with one, and here it
/// opens with `%`, so the expanded line is run exactly as written.
#[cfg(windows)]
fn cmd_line_launch(cmd: String, line: String) -> Result<Launch, String> {
    if line.contains(['\r', '\n']) {
        return Err("a cmd.exe command line cannot hold a line break".into());
    }
    Ok(Launch {
        program: cmd,
        args: vec![
            "/d".into(),
            "/s".into(),
            "/c".into(),
            format!("%{CMD_LINE_VAR}%"),
        ],
        env: vec![(CMD_LINE_VAR.to_string(), line)],
    })
}

/// One word of a `cmd.exe` command line, always double-quoted so cmd's
/// special characters inside it are literal. A run of backslashes before
/// the closing quote is doubled: the script's own program (node, for an npm
/// shim) reads its argv the MSVCRT way, where `\"` would escape the quote.
#[cfg(windows)]
fn cmd_quote(word: &str) -> Result<String, String> {
    if let Some(bad) = word.chars().find(|c| matches!(c, '%' | '"' | '\r' | '\n')) {
        return Err(format!(
            "argument {word:?} holds {bad:?}, which a .cmd/.bat launch cannot pass through cmd.exe"
        ));
    }
    let trailing = word.len() - word.trim_end_matches('\\').len();
    Ok(format!("\"{word}{}\"", "\\".repeat(trailing)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_quote_edge_cases() {
        assert_eq!(single_quote(""), "''");
        assert_eq!(single_quote("plain"), "'plain'");
        assert_eq!(single_quote("'''"), "''\\'''\\'''\\'''");
    }

    #[cfg(windows)]
    #[test]
    fn resolve_program_follows_path_and_pathext() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        // npm's layout: a POSIX script with no extension beside the shim.
        std::fs::write(first.path().join("tool"), "#!/bin/sh\n").unwrap();
        std::fs::write(first.path().join("tool.cmd"), "@echo off\n").unwrap();
        std::fs::write(second.path().join("tool.exe"), "").unwrap();
        std::fs::write(second.path().join("script.ps1"), "").unwrap();
        std::fs::write(second.path().join("both.ps1"), "").unwrap();
        std::fs::write(second.path().join("both.bat"), "").unwrap();
        let path = std::env::join_paths([first.path(), second.path()]).unwrap();
        let pathext = ".COM;.EXE;.BAT;.CMD";

        // The first PATH dir wins, and never with the extensionless file.
        assert_eq!(
            resolve_program_in("tool", &path, pathext),
            Some(first.path().join("tool.cmd"))
        );
        // PATHEXT order beats `.ps1` within a dir; `.ps1` alone still runs.
        assert_eq!(
            resolve_program_in("both", &path, pathext),
            Some(second.path().join("both.bat"))
        );
        assert_eq!(
            resolve_program_in("script", &path, pathext),
            Some(second.path().join("script.ps1"))
        );
        // A name that already carries its extension, any case.
        assert_eq!(
            resolve_program_in("TOOL.EXE", &path, pathext)
                .map(|p| p.to_string_lossy().to_ascii_lowercase()),
            Some(
                second
                    .path()
                    .join("tool.exe")
                    .to_string_lossy()
                    .to_ascii_lowercase()
            )
        );
        // A path is taken as given, extension added when bare.
        let direct = second.path().join("tool");
        assert_eq!(
            resolve_program_in(&direct.display().to_string(), &path, pathext),
            Some(second.path().join("tool.exe"))
        );
        assert_eq!(resolve_program_in("missing", &path, pathext), None);
        assert_eq!(resolve_program_in("", &path, pathext), None);
    }

    #[cfg(windows)]
    #[test]
    fn cmd_words_are_quoted_and_unsafe_ones_refused() {
        assert_eq!(cmd_quote("plain").unwrap(), "\"plain\"");
        assert_eq!(cmd_quote("a & b | c").unwrap(), "\"a & b | c\"");
        assert_eq!(cmd_quote(r"C:\dir\").unwrap(), r#""C:\dir\\""#);
        assert!(cmd_quote("100%").is_err());
        assert!(cmd_quote("say \"hi\"").is_err());
        assert!(cmd_quote("two\nlines").is_err());
    }

    #[cfg(windows)]
    #[test]
    fn a_cmd_line_rides_in_its_env_var() {
        let launch = launch(
            r"C:\Windows\System32\cmd.exe",
            &["/d".into(), "/s".into(), "/c".into(), "npm run dev && echo \"done\"".into()],
        )
        .unwrap();
        assert_eq!(launch.args, ["/d", "/s", "/c", "%BRO_CMDLINE%"]);
        assert_eq!(
            launch.env,
            [(CMD_LINE_VAR.to_string(), "npm run dev && echo \"done\"".to_string())]
        );
        assert!(cmd_line_launch("cmd".into(), "a\nb".into()).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn shell_args_follow_the_shell() {
        assert_eq!(
            shell_line_args(r"C:\Program Files\PowerShell\7\pwsh.exe", "ls"),
            ["-NoLogo", "-Command", "ls"]
        );
        assert_eq!(shell_line_args("CMD.EXE", "dir"), ["/d", "/s", "/c", "dir"]);
        assert_eq!(interactive_shell_args("powershell.exe"), ["-NoLogo"]);
        assert!(interactive_shell_args("cmd.exe").is_empty());
    }
}
