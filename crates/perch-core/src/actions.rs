//! What to run in the user's terminal. Composed here, spawned by the shell —
//! keeping perch-core free of process spawning and filesystem writes, and
//! letting every platform reuse the same command.

/// One argument in a [`TerminalCommand`]: either a literal flag (never
/// quoted — quoting `--resume` would be harmless today, but the point of this
/// type is that nothing here decides "looks like a flag" by sniffing a
/// string) or a value that came from the filesystem or a session id and must
/// always be quoted, whatever bytes it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Arg {
    Flag(&'static str),
    Value(String),
}

/// A command to hand to the user's terminal: `cd <cwd> && <program> <args...>`.
/// perch-core only composes this string; Task 9's Swift layer writes it into
/// a one-shot script and the user's terminal runs it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TerminalCommand {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: String,
}

/// POSIX single-quoting: wrap in single quotes and replace each embedded quote
/// with `'\''` (close the quote, an escaped literal quote, reopen the quote).
/// This is the only escaping that is safe for every byte a path can hold,
/// which matters because these strings come from the user's filesystem and a
/// session id Perch did not choose.
fn sq(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

impl TerminalCommand {
    /// `claude --resume <session_id>` in `cwd`.
    pub fn resume(session_id: &str, cwd: &str) -> Self {
        TerminalCommand {
            program: "claude".into(),
            args: vec!["--resume".into(), session_id.into()],
            cwd: cwd.into(),
        }
    }

    /// A fresh `claude` session in `cwd`, no arguments.
    pub fn open(cwd: &str) -> Self {
        TerminalCommand {
            program: "claude".into(),
            args: Vec::new(),
            cwd: cwd.into(),
        }
    }

    /// The typed view of `args`, distinguishing the literal `--resume` flag
    /// from the value that follows it. Kept as a derivation over the plain
    /// `Vec<String>` (rather than a stored field) so the public shape stays
    /// exactly what the brief and Task 6's UniFFI mirror expect.
    ///
    /// Matching by exact string equality (rather than a `--` prefix guess)
    /// means a value that merely *looks* like a flag still gets quoted. The
    /// one case this can't distinguish is a session id that is *exactly*
    /// `--resume`, which then renders unquoted like the real flag — but that
    /// collision is safe specifically because the literal `--resume` holds no
    /// shell-meaningful characters (no quote, no `$`, no backtick, no `;`),
    /// so whether it is emitted quoted or bare, the shell parses it to the
    /// same argv. See `a_session_id_that_collides_with_the_flag_literal_...`
    /// below for the regression this relies on.
    fn typed_args(&self) -> Vec<Arg> {
        self.args
            .iter()
            .map(|a| {
                if a == "--resume" {
                    Arg::Flag("--resume")
                } else {
                    Arg::Value(a.clone())
                }
            })
            .collect()
    }

    /// A single shell line the caller can hand to a terminal. Every value
    /// (the cwd, the session id, anything that isn't a known literal flag) is
    /// POSIX single-quoted; only the `--resume` flag itself is emitted bare.
    pub fn shell_line(&self) -> String {
        let mut line = format!("cd {} && {}", sq(&self.cwd), self.program);
        for a in self.typed_args() {
            line.push(' ');
            match a {
                Arg::Flag(f) => line.push_str(f),
                Arg::Value(v) => line.push_str(&sq(&v)),
            }
        }
        line
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resume_builds_the_documented_command() {
        let c = TerminalCommand::resume("abc-123", "/Users/a/proj");
        assert_eq!(c.program, "claude");
        assert_eq!(c.args, vec!["--resume", "abc-123"]);
        assert_eq!(c.cwd, "/Users/a/proj");
        assert_eq!(
            c.shell_line(),
            "cd '/Users/a/proj' && claude --resume 'abc-123'"
        );
    }

    #[test]
    fn open_starts_a_fresh_session_with_no_arguments() {
        let c = TerminalCommand::open("/Users/a/proj");
        assert!(c.args.is_empty());
        assert_eq!(c.shell_line(), "cd '/Users/a/proj' && claude");
    }

    #[test]
    fn paths_with_spaces_survive_quoting() {
        let c = TerminalCommand::open("/Users/a/My Projects/thing");
        assert_eq!(c.shell_line(), "cd '/Users/a/My Projects/thing' && claude");
    }

    #[test]
    fn a_single_quote_in_a_path_cannot_break_out_of_the_quoting() {
        let c = TerminalCommand::open("/Users/a/it's mine");
        // POSIX: close, escaped literal quote, reopen.
        assert_eq!(c.shell_line(), r#"cd '/Users/a/it'\''s mine' && claude"#);
        assert!(
            !c.shell_line().contains("; "),
            "no statement separator can be injected"
        );
    }

    #[test]
    fn a_session_id_is_quoted_too() {
        let c = TerminalCommand::resume("a'; rm -rf /", "/tmp");
        assert_eq!(
            c.shell_line(),
            r#"cd '/tmp' && claude --resume 'a'\''; rm -rf /'"#
        );
    }

    // A session id that happens to be exactly the literal `--resume` collides
    // with the flag in `typed_args`'s exact-match rule and is rendered
    // unquoted like the flag. See the comment on `typed_args` for why this
    // still can't change what the shell runs: the literal holds no
    // shell-meaningful characters, so quoted or not it parses to the same
    // argv, `["--resume", "--resume"]`.
    #[test]
    fn a_session_id_that_collides_with_the_flag_literal_still_renders_safely() {
        let c = TerminalCommand::resume("--resume", "/tmp");
        assert_eq!(c.shell_line(), "cd '/tmp' && claude --resume --resume");
    }

    // Beyond the brief's five: a command-substitution shape. Single quotes in
    // POSIX shells suppress all expansion inside them — `$(...)`, backticks,
    // `$VAR` — so this must survive as inert literal text, not run a command.
    #[test]
    fn a_command_substitution_in_a_path_is_never_evaluated() {
        let c = TerminalCommand::open("/tmp/$(rm -rf /)/`whoami`");
        assert_eq!(
            c.shell_line(),
            "cd '/tmp/$(rm -rf /)/`whoami`' && claude",
            "the substitution stays inside single quotes, where the shell never expands it"
        );
    }
}
