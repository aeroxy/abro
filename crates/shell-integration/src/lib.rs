use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShellKind {
    Bash,
    Zsh,
    Fish,
}

impl ShellKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bash => "bash",
            Self::Zsh => "zsh",
            Self::Fish => "fish",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BoundaryPhase {
    Start,
    End,
    Cwd,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandBoundaryEvent {
    pub shell: ShellKind,
    pub phase: BoundaryPhase,
    pub payload: String,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ParsedChunk {
    pub output: String,
    pub events: Vec<CommandBoundaryEvent>,
}

/// Strip ANSI escape sequences from text (CSI sequences like [0m, OSC sequences like ]7;file://...^G)
fn strip_ansi_escapes(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '\x1b' => {
                if let Some(&next) = chars.peek() {
                    match next {
                        '[' => {
                            // CSI sequence: skip until 0x40-0x7E
                            chars.next(); // consume '['
                            while let Some(&c) = chars.peek() {
                                let b = c as u32;
                                if (0x40..=0x7E).contains(&b) {
                                    chars.next();
                                    break;
                                }
                                chars.next();
                            }
                        }
                        ']' => {
                            // OSC sequence: skip until BEL (\x07) or ST (\x1b\\)
                            chars.next(); // consume ']'
                            while let Some(c) = chars.next() {
                                if c == '\x07' {
                                    break;
                                } else if c == '\x1b' {
                                    if let Some(&'\\') = chars.peek() {
                                        chars.next();
                                        break;
                                    }
                                }
                            }
                        }
                        _ => {
                            // Other escape, just skip the next char for now
                            chars.next();
                        }
                    }
                }
            }
            '\x08' => {
                // Backspace - pop the last character
                result.pop();
            }
            '\x07' => {
                // Ignore bell
            }
            '\r' => {
                // Ignore carriage return so we don't mess up HTML whitespace-pre
            }
            _ => {
                result.push(ch);
            }
        }
    }
    result
}

#[derive(Debug)]
pub struct BoundaryParser {
    pending_line: String,
    inside_block: bool,
}

impl Default for BoundaryParser {
    fn default() -> Self {
        Self {
            pending_line: String::new(),
            inside_block: true,
        }
    }
}

pub fn boundary_marker(shell: ShellKind) -> &'static str {
    match shell {
        ShellKind::Bash => "__ABRO_BOUNDARY__:bash",
        ShellKind::Zsh => "__ABRO_BOUNDARY__:zsh",
        ShellKind::Fish => "__ABRO_BOUNDARY__:fish",
    }
}

const HOOK_FILE_DIR: &str = "~/.abro";
const HOOK_FILE_PREFIX: &str = "hooks-";
const HOOK_FILE_SUFFIX: &str = ".sh";

pub fn hook_file_path(shell: ShellKind) -> String {
    format!(
        "{}{}{}{}{}",
        HOOK_FILE_DIR,
        "/",
        HOOK_FILE_PREFIX,
        shell.as_str(),
        HOOK_FILE_SUFFIX
    )
}

pub fn install_snippet_content(shell: ShellKind) -> String {
    let marker = boundary_marker(shell);

    match shell {
        ShellKind::Bash => format!(
            "set +H 2>/dev/null; if [[ -z \"${{__abro_hooks_installed:-}}\" ]]; then __abro_hooks_installed=1; __abro_preexec() {{ local cmd=\"$BASH_COMMAND\"; printf '{marker}:start:%s\\n' \"$cmd\"; }}; trap '__abro_preexec' DEBUG; __abro_precmd() {{ local exit_code=$?; printf '{marker}:end:%s\\n' \"$exit_code\"; printf '{marker}:cwd:%s\\n' \"$PWD\"; }}; PROMPT_COMMAND=\"__abro_precmd${{PROMPT_COMMAND:+;$PROMPT_COMMAND}}\"; fi"
        ),
        ShellKind::Zsh => format!(
            "setopt HIST_IGNORE_SPACE 2>/dev/null; if [[ -z \"${{__abro_hooks_installed:-}}\" ]]; then typeset -g __abro_hooks_installed=1; unsetopt PROMPT_SP 2>/dev/null || true; __abro_preexec() {{ printf '{marker}:start:%s\\n' \"$1\"; }}; __abro_precmd() {{ local exit_code=$?; printf '{marker}:end:%s\\n' \"$exit_code\"; printf '{marker}:cwd:%s\\n' \"$PWD\"; }}; autoload -Uz add-zsh-hook >/dev/null 2>&1 || true; add-zsh-hook preexec __abro_preexec 2>/dev/null || precmd_functions+=(__abro_preexec); add-zsh-hook precmd __abro_precmd 2>/dev/null || precmd_functions+=(__abro_precmd); fi"
        ),
        ShellKind::Fish => format!(
            "if not set -q __abro_hooks_installed; set -g __abro_hooks_installed 1; function __abro_preexec --on-event fish_preexec; printf '{marker}:start:%s\\n' \"$argv\"; end; function __abro_postexec --on-event fish_postexec; printf '{marker}:end:%s\\n' \"$status\"; end; function __abro_prompt --on-event fish_prompt; printf '{marker}:cwd:%s\\n' \"$PWD\"; end; end"
        ),
    }
}

pub fn install_snippet(shell: ShellKind) -> String {
    format!(" source {}", hook_file_path(shell))
}

impl BoundaryParser {
    pub fn ingest(&mut self, chunk: &str) -> ParsedChunk {
        self.pending_line.push_str(chunk);

        let mut parsed = ParsedChunk::default();
        let mut consumed = 0usize;

        while let Some(rel_nl) = self.pending_line[consumed..].find('\n') {
            let nl_idx = consumed + rel_nl;
            let raw_line = &self.pending_line[consumed..nl_idx];
            let line = raw_line.trim_end_matches('\r');

            if is_hook_install_echo(line) {
                // Suppress echoed bootstrap hook installation from terminal output.
            } else if let Some(event) = parse_boundary_line(line) {
                match event.phase {
                    BoundaryPhase::Start => self.inside_block = true,
                    BoundaryPhase::End => self.inside_block = false,
                    _ => {}
                }
                parsed.events.push(event);
            } else if self.inside_block {
                let clean = strip_ansi_escapes(line);
                parsed.output.push_str(&clean);
                parsed.output.push('\n');
            }

            consumed = nl_idx + 1;
        }

        if consumed > 0 {
            self.pending_line.drain(..consumed);
        }

        // Only capture partial line if we're inside a block and it's not a boundary fragment
        if !self.pending_line.is_empty()
            && !should_buffer_partial_line(&self.pending_line)
            && self.inside_block
        {
            let clean = strip_ansi_escapes(&self.pending_line);
            parsed.output.push_str(&clean);
            self.pending_line.clear();
        }

        parsed
    }

    pub fn flush(&mut self) -> ParsedChunk {
        if self.pending_line.is_empty() {
            return ParsedChunk::default();
        }

        let mut parsed = ParsedChunk::default();
        let line = std::mem::take(&mut self.pending_line);
        let line = line.trim_end_matches('\r');

        if is_hook_install_echo(line) {
            // Suppress echoed bootstrap hook installation from terminal output.
        } else if let Some(event) = parse_boundary_line(line) {
            parsed.events.push(event);
        } else {
            parsed.output = strip_ansi_escapes(line);
        }

        parsed
    }
}

fn parse_boundary_line(line: &str) -> Option<CommandBoundaryEvent> {
    for shell in [ShellKind::Bash, ShellKind::Zsh, ShellKind::Fish] {
        let marker = boundary_marker(shell);
        let Some(idx) = line.find(marker) else {
            continue;
        };
        let rest = &line[idx + marker.len()..];

        if let Some(payload) = rest.strip_prefix(":start:") {
            return Some(CommandBoundaryEvent {
                shell,
                phase: BoundaryPhase::Start,
                payload: payload.to_string(),
            });
        }

        if let Some(payload) = rest.strip_prefix(":end:") {
            return Some(CommandBoundaryEvent {
                shell,
                phase: BoundaryPhase::End,
                payload: payload.to_string(),
            });
        }

        if let Some(payload) = rest.strip_prefix(":cwd:") {
            return Some(CommandBoundaryEvent {
                shell,
                phase: BoundaryPhase::Cwd,
                payload: payload.to_string(),
            });
        }
    }

    None
}

fn is_hook_install_echo(line: &str) -> bool {
    let lowered = line.to_ascii_lowercase();
    [
        "__abro_hooks_installed",
        "__abro_preexec",
        "__abro_precmd",
        "__abro_postexec",
        "add-zsh-hook",
        "preexec_functions+=",
        "precmd_functions+=",
        "autoload -uz add-zsh-hook",
        "unsetopt prompt_sp",
    ]
    .iter()
    .any(|needle| lowered.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_command_not_found_single_chunk() {
        let mut parser = BoundaryParser::default();
        let chunk = "zsh: command not found: hello\n\
                     __ABRO_BOUNDARY__:zsh:start:hello\n\
                     __ABRO_BOUNDARY__:zsh:end:127\n\
                     __ABRO_BOUNDARY__:zsh:cwd:/Users/aero\n";
        let parsed = parser.ingest(chunk);
        assert_eq!(parsed.output, "zsh: command not found: hello\n");
        assert_eq!(parsed.events.len(), 3);
        assert_eq!(parsed.events[0].phase, BoundaryPhase::Start);
        assert_eq!(parsed.events[1].phase, BoundaryPhase::End);
        assert_eq!(parsed.events[1].payload, "127");
        assert_eq!(parsed.events[2].phase, BoundaryPhase::Cwd);
    }

    #[test]
    fn test_command_not_found_in_separate_chunks() {
        let mut parser = BoundaryParser::default();

        let p1 = parser.ingest("zsh: command not found: hello\n");
        assert_eq!(p1.output, "zsh: command not found: hello\n");
        assert!(p1.events.is_empty());

        let p2 = parser.ingest("__ABRO_BOUNDARY__:zsh:start:hello\n");
        assert!(p2.output.is_empty());
        assert_eq!(p2.events.len(), 1);
        assert_eq!(p2.events[0].phase, BoundaryPhase::Start);

        let p3 =
            parser.ingest("__ABRO_BOUNDARY__:zsh:end:127\n__ABRO_BOUNDARY__:zsh:cwd:/Users/aero\n");
        assert!(p3.output.is_empty());
        assert_eq!(p3.events.len(), 2);
        assert_eq!(p3.events[0].phase, BoundaryPhase::End);
        assert_eq!(p3.events[1].phase, BoundaryPhase::Cwd);
    }

    #[test]
    fn test_output_after_end_boundary_in_same_chunk_is_dropped() {
        let mut parser = BoundaryParser::default();
        let chunk = "__ABRO_BOUNDARY__:zsh:start:hello\n\
                     __ABRO_BOUNDARY__:zsh:end:127\n\
                     some-prompt-text\n";
        let parsed = parser.ingest(chunk);
        assert!(parsed.output.is_empty());
        assert_eq!(parsed.events.len(), 2);
    }

    #[test]
    fn test_output_after_end_boundary_across_chunks_is_dropped() {
        let mut parser = BoundaryParser::default();

        // First chunk: start and end boundaries
        let p1 =
            parser.ingest("__ABRO_BOUNDARY__:zsh:start:hello\n__ABRO_BOUNDARY__:zsh:end:127\n");
        assert!(p1.output.is_empty());
        assert_eq!(p1.events.len(), 2);

        // Second chunk: prompt text arrives after the block ended
        let p2 = parser.ingest("user@host $ ");
        assert!(p2.output.is_empty());
        assert!(p2.events.is_empty());
    }

    #[test]
    fn test_output_between_start_and_end_across_chunks() {
        let mut parser = BoundaryParser::default();

        // First chunk: start boundary only
        let p1 = parser.ingest("__ABRO_BOUNDARY__:zsh:start:hello\n");
        assert!(p1.output.is_empty());
        assert_eq!(p1.events.len(), 1);

        // Second chunk: command output
        let p2 = parser.ingest("zsh: command not found: hello\n");
        assert_eq!(p2.output, "zsh: command not found: hello\n");
        assert!(p2.events.is_empty());

        // Third chunk: end and cwd boundaries
        let p3 = parser.ingest("__ABRO_BOUNDARY__:zsh:end:127\n__ABRO_BOUNDARY__:zsh:cwd:/tmp\n");
        assert!(p3.output.is_empty());
        assert_eq!(p3.events.len(), 2);

        // Fourth chunk: prompt text - should be DROPPED (block ended)
        let p4 = parser.ingest("user@host $ ");
        assert!(p4.output.is_empty());
    }

    #[test]
    fn test_output_before_start_boundary_in_same_chunk() {
        let mut parser = BoundaryParser::default();
        let chunk = "some-output\n\
                     __ABRO_BOUNDARY__:zsh:start:hello\n\
                     __ABRO_BOUNDARY__:zsh:end:0\n\
                     __ABRO_BOUNDARY__:zsh:cwd:/tmp\n";
        let parsed = parser.ingest(chunk);
        assert_eq!(parsed.output, "some-output\n");
        assert_eq!(parsed.events.len(), 3);
    }

    #[test]
    fn test_split_boundary_marker() {
        let mut parser = BoundaryParser::default();
        let p1 = parser.ingest("__ABRO_BOUNDARY__:zsh");
        assert!(p1.output.is_empty());
        assert!(p1.events.is_empty());

        let p2 = parser
            .ingest(":start:hello\n__ABRO_BOUNDARY__:zsh:end:0\n__ABRO_BOUNDARY__:zsh:cwd:/tmp\n");
        assert!(p2.output.is_empty());
        assert_eq!(p2.events.len(), 3);
        assert_eq!(p2.events[0].phase, BoundaryPhase::Start);
        assert_eq!(p2.events[0].payload, "hello");
    }

}

fn should_buffer_partial_line(partial: &str) -> bool {
    let trimmed = partial.trim_start_matches(|c: char| c.is_ascii_whitespace() || c.is_control());
    let marker_prefixes = [
        "__ABRO_BOUNDARY__:bash",
        "__ABRO_BOUNDARY__:zsh",
        "__ABRO_BOUNDARY__:fish",
    ];

    marker_prefixes.iter().any(|marker| {
        marker.starts_with(trimmed)
            || trimmed.starts_with("__ABRO_")
            || trimmed.starts_with("__ABRO_BOUNDARY__")
    }) || is_hook_install_echo(trimmed)
}
