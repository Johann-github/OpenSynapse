//! macOS login autostart: a per-user launchd LaunchAgent property list in
//! `~/Library/LaunchAgents` (`vault/Decisions/ADR-0012 macOS agent.md`).
//!
//! Everything here is pure and platform-independent (unit-tested on Windows):
//! rendering our plist, XML escaping, reading our own plist back, and the
//! folder from a given `HOME`. The callers in `lib.rs` do the file I/O.
//!
//! The plist is written as text in Apple's XML property-list format (DOCTYPE
//! `-//Apple//DTD PLIST 1.0//EN`). Keys: `Label`, `ProgramArguments` (the agent
//! path only), `RunAtLoad` true (start when launchd loads the job at login),
//! `KeepAlive` false (no restart after an exit: exit 2 "already running" and
//! exit 3 "elevated" must not loop), `LimitLoadToSessionType` `Aqua` (only the
//! graphical login session, like the XDG autostart entry on Linux).
//! `launchctl` is never run: launchd reads the folder at the next login.

use std::path::PathBuf;

/// launchd job label (reverse DNS of the project's home, `Label` key).
pub const LABEL: &str = "io.github.johann-github.opensynapse.agent";
/// File name of our LaunchAgent: `<label>.plist`, as launchd convention asks.
pub const FILE_NAME: &str = "io.github.johann-github.opensynapse.agent.plist";

/// `$HOME/Library/LaunchAgents/<label>.plist`; `None` when `HOME` is missing,
/// blank or relative.
pub fn plist_path(home: Option<&str>) -> Option<PathBuf> {
    let home = home.filter(|h| h.starts_with('/'))?;
    Some(
        PathBuf::from(home)
            .join("Library")
            .join("LaunchAgents")
            .join(FILE_NAME),
    )
}

/// Escape text for an XML element (`&`, `<`, `>`, and both quotes for good
/// measure). `None` when the text has a character XML 1.0 cannot carry (control
/// characters other than tab, newline, carriage return).
pub fn xml_escape(s: &str) -> Option<String> {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\t' | '\n' | '\r' => out.push(c),
            c if c.is_control() => return None,
            c => out.push(c),
        }
    }
    Some(out)
}

/// Reverse [`xml_escape`]: the five named entities and numeric references
/// (`&#38;`, `&#x26;`). Unknown or broken references are kept as written.
pub fn xml_unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let decoded = rest.find(';').and_then(|end| {
            let name = &rest[1..end];
            let c = match name {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                _ => {
                    let num = name.strip_prefix('#')?;
                    let code = match num.strip_prefix(['x', 'X']) {
                        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                        None => num.parse().ok()?,
                    };
                    char::from_u32(code)
                }
            }?;
            Some((c, end + 1))
        });
        match decoded {
            Some((c, len)) => {
                out.push(c);
                rest = &rest[len..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// The LaunchAgent plist that starts `agent` (absolute path) at login.
/// `Err` when the path cannot be written into XML.
pub fn render(agent: &str) -> Result<String, String> {
    let program =
        xml_escape(agent).ok_or_else(|| format!("{agent:?} contains control characters"))?;
    Ok(format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
         \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n\
         <dict>\n\
         \t<key>Label</key>\n\
         \t<string>{LABEL}</string>\n\
         \t<key>ProgramArguments</key>\n\
         \t<array>\n\
         \t\t<string>{program}</string>\n\
         \t</array>\n\
         \t<key>RunAtLoad</key>\n\
         \t<true/>\n\
         \t<key>KeepAlive</key>\n\
         \t<false/>\n\
         \t<key>LimitLoadToSessionType</key>\n\
         \t<string>Aqua</string>\n\
         </dict>\n\
         </plist>\n"
    ))
}

/// One element of the top-level dict, as far as we read plists.
#[derive(Debug, PartialEq)]
enum Value {
    Bool(bool),
    String(String),
    /// First `<string>` of an `<array>` (the program of `ProgramArguments`).
    FirstString(Option<String>),
    Other,
}

/// The program (first `ProgramArguments` string, unescaped) our plist starts
/// at login, or `None` when it starts nothing: no program, `Disabled` true, or
/// `RunAtLoad` not true. A minimal reader for the files [`render`] writes and
/// light hand edits (whitespace, comments); not a general plist parser.
pub fn program(text: &str) -> Option<String> {
    let text = strip_comments(text);
    let start = text.find("<dict>")? + "<dict>".len();
    let mut s = &text[start..];
    let mut prog = None;
    let mut run_at_load = false;
    let mut disabled = false;
    loop {
        s = s.trim_start();
        let Some(after) = s.strip_prefix("<key>") else {
            break;
        };
        let end = after.find("</key>")?;
        let key = xml_unescape(after[..end].trim());
        s = after[end + "</key>".len()..].trim_start();
        let (value, tail) = read_value(s)?;
        s = tail;
        match (key.as_str(), value) {
            ("ProgramArguments", Value::FirstString(p)) => prog = p,
            ("Program", Value::String(p)) if prog.is_none() => prog = Some(p),
            ("RunAtLoad", Value::Bool(b)) => run_at_load = b,
            ("Disabled", Value::Bool(b)) => disabled = b,
            _ => {}
        }
    }
    prog.filter(|p| run_at_load && !disabled && !p.trim().is_empty())
}

fn strip_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("<!--") {
        out.push_str(&rest[..at]);
        match rest[at..].find("-->") {
            Some(end) => rest = &rest[at + end + 3..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// One value at the start of `s`, and the text after it.
fn read_value(s: &str) -> Option<(Value, &str)> {
    if let Some(t) = s.strip_prefix("<true/>") {
        return Some((Value::Bool(true), t));
    }
    if let Some(t) = s.strip_prefix("<false/>") {
        return Some((Value::Bool(false), t));
    }
    if let Some(t) = s.strip_prefix("<string/>") {
        return Some((Value::String(String::new()), t));
    }
    if let Some(t) = s.strip_prefix("<string>") {
        let end = t.find("</string>")?;
        return Some((
            Value::String(xml_unescape(&t[..end])),
            &t[end + "</string>".len()..],
        ));
    }
    if let Some(t) = s.strip_prefix("<array/>") {
        return Some((Value::FirstString(None), t));
    }
    if let Some(t) = s.strip_prefix("<array>") {
        let end = t.find("</array>")?;
        let first = read_value(t[..end].trim_start()).and_then(|(v, _)| match v {
            Value::String(p) => Some(p),
            _ => None,
        });
        return Some((Value::FirstString(first), &t[end + "</array>".len()..]));
    }
    // Any other element: skip to its matching close (or self-close). Nested
    // elements of the same name are not expected in our file.
    let t = s.strip_prefix('<')?;
    let name_end = t.find(|c: char| c == '>' || c == '/' || c.is_whitespace())?;
    let name = &t[..name_end];
    let open_end = t.find('>')?;
    if t[..open_end].ends_with('/') {
        return Some((Value::Other, &t[open_end + 1..]));
    }
    let close = format!("</{name}>");
    let end = t.find(&close)?;
    Some((Value::Other, &t[end + close.len()..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plist_path_from_home() {
        assert_eq!(
            plist_path(Some("/Users/u")),
            Some(PathBuf::from("/Users/u/Library/LaunchAgents").join(FILE_NAME))
        );
        assert_eq!(plist_path(Some("")), None);
        assert_eq!(plist_path(Some("rel")), None);
        assert_eq!(plist_path(None), None);
        assert_eq!(FILE_NAME, format!("{LABEL}.plist"));
    }

    #[test]
    fn rendered_plist_has_the_keys() {
        let t = render("/Users/u/.local/bin/opensynapse-agent").unwrap();
        assert!(t.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist"));
        assert!(t.contains(&format!("<key>Label</key>\n\t<string>{LABEL}</string>")));
        assert!(t.contains("<key>RunAtLoad</key>\n\t<true/>"));
        assert!(t.contains("<key>KeepAlive</key>\n\t<false/>"));
        assert!(t.contains("<key>LimitLoadToSessionType</key>\n\t<string>Aqua</string>"));
        assert!(t.ends_with("</dict>\n</plist>\n"));
    }

    #[test]
    fn program_round_trips_with_escaping() {
        for p in [
            "/Users/u/.local/bin/opensynapse-agent",
            "/Users/u/My Apps/a&b<c>d\"e'f/ü/opensynapse-agent",
        ] {
            let t = render(p).unwrap();
            assert_eq!(program(&t).as_deref(), Some(p), "{t}");
        }
        let t = render("/a&b/<x>").unwrap();
        assert!(t.contains("<string>/a&amp;b/&lt;x&gt;</string>"), "{t}");
        assert!(render("/a\u{1}b").is_err());
    }

    #[test]
    fn unescape_entities() {
        assert_eq!(xml_unescape("a&amp;b&#38;c&#x26;d&#X3C;"), "a&b&c&d<");
        assert_eq!(xml_unescape("&bogus; & &#xZZ; &"), "&bogus; & &#xZZ; &");
        assert_eq!(xml_escape("tab\there").as_deref(), Some("tab\there"));
    }

    #[test]
    fn switched_off_or_foreign_plists_start_nothing() {
        let base = render("/a/agent").unwrap();
        let disabled = base.replace(
            "<key>RunAtLoad</key>",
            "<key>Disabled</key>\n\t<true/>\n\t<key>RunAtLoad</key>",
        );
        assert_eq!(program(&disabled), None);
        let no_load = base.replace(
            "<key>RunAtLoad</key>\n\t<true/>",
            "<key>RunAtLoad</key>\n\t<false/>",
        );
        assert_eq!(program(&no_load), None);
        assert_eq!(program("<plist><dict></dict></plist>"), None);
        assert_eq!(program("not xml"), None);
        assert_eq!(program(""), None);
    }

    #[test]
    fn tolerates_comments_other_keys_and_program_key() {
        let t = "<?xml version=\"1.0\"?>\n<!-- <dict><key>X</key> -->\n<plist version=\"1.0\">\
                 <dict><key>EnvironmentVariables</key><dict><key>A</key><string>b</string></dict>\
                 <key>Nice</key><integer>5</integer>\
                 <key>ProgramArguments</key><array><string>/x/agent</string><string>-v</string></array>\
                 <key>RunAtLoad</key><true/></dict></plist>";
        assert_eq!(program(t).as_deref(), Some("/x/agent"));
        let t = "<plist><dict><key>Program</key><string>/p/agent</string>\
                 <key>RunAtLoad</key><true/></dict></plist>";
        assert_eq!(program(t).as_deref(), Some("/p/agent"));
    }
}
