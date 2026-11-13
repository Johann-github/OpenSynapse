//! Freedesktop Desktop Entry files for Linux: the agent's XDG autostart entry
//! and the GUI's application-menu launcher
//! (`vault/Decisions/ADR-0011 Linux autostart and install.md`).
//!
//! Everything here is pure and platform-independent (unit-tested on Windows):
//! rendering entries, quoting `Exec=` per the Desktop Entry Specification,
//! reading our own autostart entry back, and resolving the XDG folders from
//! given environment values. The callers in `lib.rs` / `shortcut.rs` read the
//! environment and do the file I/O.
//!
//! Two escaping layers, applied in this order when writing (reversed when
//! reading): (1) `Exec` argument quoting: `%` becomes `%%`; an argument with a
//! reserved character is put in double quotes, and inside them `"`, `` ` ``,
//! `$` and `\` get a backslash; (2) the general string-value escapes `\\`, `\n`,
//! `\t`, `\r` (`\s` is also read). So a literal backslash inside a quoted path
//! is written as four backslashes, as the specification notes.

use std::path::PathBuf;

/// File name of the agent's autostart entry in `$XDG_CONFIG_HOME/autostart`.
pub const AUTOSTART_FILE: &str = "opensynapse-agent.desktop";
/// File name of the GUI launcher in `$XDG_DATA_HOME/applications`.
pub const LAUNCHER_FILE: &str = "opensynapse.desktop";

/// Characters that force quoting of an `Exec` argument (Desktop Entry
/// Specification, "The Exec key").
const RESERVED: &[char] = &[
    ' ', '\t', '\n', '"', '\'', '\\', '>', '<', '~', '|', '&', ';', '$', '*', '?', '#', '(', ')',
    '`',
];

/// An absolute XDG base folder from an environment value; relative or blank
/// values are invalid per the XDG Base Directory Specification and ignored.
fn xdg_base(value: Option<&str>) -> Option<PathBuf> {
    value.filter(|v| v.starts_with('/')).map(PathBuf::from)
}

fn home_join(home: Option<&str>, parts: &[&str]) -> Option<PathBuf> {
    let home = home.filter(|h| !h.trim().is_empty())?;
    let mut p = PathBuf::from(home);
    for part in parts {
        p.push(part);
    }
    Some(p)
}

/// `$XDG_CONFIG_HOME`, else `$HOME/.config`.
pub fn config_home(xdg_config_home: Option<&str>, home: Option<&str>) -> Option<PathBuf> {
    xdg_base(xdg_config_home).or_else(|| home_join(home, &[".config"]))
}

/// `$XDG_DATA_HOME`, else `$HOME/.local/share`.
pub fn data_home(xdg_data_home: Option<&str>, home: Option<&str>) -> Option<PathBuf> {
    xdg_base(xdg_data_home).or_else(|| home_join(home, &[".local", "share"]))
}

/// `<config home>/autostart/opensynapse-agent.desktop`.
pub fn autostart_path(xdg_config_home: Option<&str>, home: Option<&str>) -> Option<PathBuf> {
    Some(
        config_home(xdg_config_home, home)?
            .join("autostart")
            .join(AUTOSTART_FILE),
    )
}

/// `<data home>/applications/opensynapse.desktop`.
pub fn launcher_path(xdg_data_home: Option<&str>, home: Option<&str>) -> Option<PathBuf> {
    Some(
        data_home(xdg_data_home, home)?
            .join("applications")
            .join(LAUNCHER_FILE),
    )
}

/// One `Exec` argument, quoted only when it has to be (layer 1).
pub fn quote_exec_arg(arg: &str) -> String {
    let arg = arg.replace('%', "%%");
    if !arg.contains(RESERVED) && !arg.chars().any(char::is_control) {
        return arg;
    }
    let mut out = String::with_capacity(arg.len() + 2);
    out.push('"');
    for c in arg.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

/// The first argument (the program) of an `Exec` value whose string escapes
/// are already undone (reverses layer 1 for that argument).
pub fn exec_program(exec: &str) -> String {
    let s = exec.trim_start();
    let mut out = String::new();
    let mut chars = s.chars();
    if s.starts_with('"') {
        chars.next();
        while let Some(c) = chars.next() {
            match c {
                '"' => break,
                '\\' => match chars.next() {
                    Some(n @ ('"' | '`' | '$' | '\\')) => out.push(n),
                    Some(n) => {
                        out.push('\\');
                        out.push(n);
                    }
                    None => out.push('\\'),
                },
                _ => out.push(c),
            }
        }
    } else {
        out.extend(chars.take_while(|c| !c.is_whitespace()));
    }
    out.replace("%%", "%")
}

/// Layer 2: escape a string value for one line of a desktop entry.
pub fn escape_value(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            _ => out.push(c),
        }
    }
    out
}

/// Layer 2 reversed. Unknown escapes are kept as written.
pub fn unescape_value(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => out.push(' '),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some(n) => {
                out.push('\\');
                out.push(n);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// The XDG autostart entry that starts `agent` (absolute path) at login.
pub fn render_autostart(agent: &str) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=OpenSynapse Agent\n\
         Comment=Applies Razer device settings in the background (OpenSynapse)\n\
         Exec={}\n\
         Terminal=false\n\
         X-GNOME-Autostart-enabled=true\n",
        escape_value(&quote_exec_arg(agent))
    )
}

/// The application-menu launcher for the GUI at `gui`, started in `working_dir`.
pub fn render_launcher(gui: &str, working_dir: &str, comment: &str) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=OpenSynapse\n\
         Comment={}\n\
         Exec={}\n\
         TryExec={}\n\
         Path={}\n\
         Terminal=false\n\
         Categories=Settings;HardwareSettings;\n",
        escape_value(comment),
        escape_value(&quote_exec_arg(gui)),
        escape_value(gui),
        escape_value(working_dir),
    )
}

/// The `Exec` value (string escapes undone, quoting kept) of an autostart
/// entry, or `None` when it starts nothing: no `Exec`, `Hidden=true` (the spec's
/// "treat as deleted") or `X-GNOME-Autostart-enabled=false` (how GNOME and
/// others switch an entry off). Only keys of the `[Desktop Entry]` group count.
pub fn autostart_exec(text: &str) -> Option<String> {
    let mut in_main = false;
    let mut exec = None;
    let mut off = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            in_main = line == "[Desktop Entry]";
            continue;
        }
        if !in_main {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "Exec" => exec = Some(unescape_value(value)),
            "Hidden" => off |= value == "true",
            "X-GNOME-Autostart-enabled" => off |= value == "false",
            _ => {}
        }
    }
    exec.filter(|e| !off && !e.trim().is_empty())
}

/// The effective user id from the text of `/proc/self/status` (`Uid:` line:
/// real, effective, saved, filesystem).
pub fn euid_from_proc_status(text: &str) -> Option<u32> {
    text.lines()
        .find_map(|l| l.strip_prefix("Uid:"))?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xdg_dirs_from_env_values() {
        let h = Some("/home/u");
        assert_eq!(
            autostart_path(None, h),
            Some(PathBuf::from("/home/u/.config/autostart").join(AUTOSTART_FILE))
        );
        assert_eq!(
            autostart_path(Some("/x/cfg"), h),
            Some(PathBuf::from("/x/cfg/autostart").join(AUTOSTART_FILE))
        );
        // Relative or empty XDG values are invalid and fall back to HOME.
        assert_eq!(config_home(Some("rel/cfg"), h), config_home(None, h));
        assert_eq!(config_home(Some(""), h), config_home(None, h));
        assert_eq!(
            launcher_path(None, h),
            Some(PathBuf::from("/home/u/.local/share/applications").join(LAUNCHER_FILE))
        );
        assert_eq!(
            launcher_path(Some("/x/data"), None),
            Some(PathBuf::from("/x/data/applications").join(LAUNCHER_FILE))
        );
        assert_eq!(autostart_path(None, None), None);
        assert_eq!(launcher_path(None, Some(" ")), None);
    }

    #[test]
    fn plain_path_is_not_quoted() {
        let p = "/home/u/.local/bin/opensynapse-agent";
        assert_eq!(quote_exec_arg(p), p);
        assert_eq!(exec_program(p), p);
        assert_eq!(exec_program(&format!("{p} --flag")), p);
    }

    #[test]
    fn path_with_space_and_specials_round_trips() {
        let p = r#"/home/u/My Apps/a"b`c$d\e%f/opensynapse-agent"#;
        let q = quote_exec_arg(p);
        assert_eq!(q, r#""/home/u/My Apps/a\"b\`c\$d\\e%%f/opensynapse-agent""#);
        assert_eq!(exec_program(&q), p);
        assert_eq!(exec_program(&format!("{q} -v")), p);
        // In the file, the backslash before the backslash is escaped again.
        let line = escape_value(&q);
        assert!(line.contains(r"\\\\e"), "{line}");
        assert_eq!(unescape_value(&line), q);
    }

    #[test]
    fn value_escapes() {
        assert_eq!(escape_value("a\\b\nc\td"), r"a\\b\nc\td");
        assert_eq!(unescape_value(r"a\sb\\c\nd\q"), "a b\\c\nd\\q");
    }

    #[test]
    fn autostart_entry_round_trips() {
        for p in [
            "/home/u/.local/bin/opensynapse-agent",
            r"/home/u/d i\r/agent",
        ] {
            let text = render_autostart(p);
            assert!(text.starts_with("[Desktop Entry]\n"));
            assert!(text.contains("Type=Application\n"));
            let exec = autostart_exec(&text).unwrap();
            assert_eq!(exec, quote_exec_arg(p));
            assert_eq!(exec_program(&exec), p);
        }
    }

    #[test]
    fn disabled_or_foreign_entries_start_nothing() {
        let base = render_autostart("/a/agent");
        assert_eq!(autostart_exec(&format!("{base}Hidden=true\n")), None);
        assert_eq!(
            autostart_exec(&base.replace(
                "X-GNOME-Autostart-enabled=true",
                "X-GNOME-Autostart-enabled=false"
            )),
            None
        );
        assert_eq!(autostart_exec("[Desktop Entry]\nType=Application\n"), None);
        // Exec in another group does not count; comments are skipped.
        let other = "# c\n[Desktop Entry]\nName=x\n[Desktop Action a]\nExec=/evil\n";
        assert_eq!(autostart_exec(other), None);
        assert_eq!(
            autostart_exec("[Desktop Entry]\n Exec = /a/agent \n").as_deref(),
            Some("/a/agent")
        );
    }

    #[test]
    fn launcher_entry() {
        let t = render_launcher(
            "/home/u/.local/bin/opensynapse-desktop",
            "/home/u/.local/bin",
            "OpenSynapse: Razer device settings",
        );
        assert!(t.contains("\nExec=/home/u/.local/bin/opensynapse-desktop\n"));
        assert!(t.contains("\nTryExec=/home/u/.local/bin/opensynapse-desktop\n"));
        assert!(t.contains("\nPath=/home/u/.local/bin\n"));
        assert!(t.contains("\nCategories=Settings;HardwareSettings;\n"));
        let spaced = render_launcher("/opt/my apps/gui", "/opt/my apps", "c");
        assert!(spaced.contains("\nExec=\"/opt/my apps/gui\"\n"), "{spaced}");
        // TryExec and Path are plain strings, never Exec-quoted.
        assert!(spaced.contains("\nTryExec=/opt/my apps/gui\n"));
    }

    #[test]
    fn euid_from_status() {
        let s = "Name:\tx\nUmask:\t0022\nUid:\t1000\t0\t1000\t1000\nGid:\t1000\t1000\t1000\t1000\n";
        assert_eq!(euid_from_proc_status(s), Some(0));
        assert_eq!(
            euid_from_proc_status("Uid:\t1000\t1000\t1000\t1000\n"),
            Some(1000)
        );
        assert_eq!(euid_from_proc_status("Name:\tx\n"), None);
        assert_eq!(euid_from_proc_status("Uid:\t1000\n"), None);
    }
}
