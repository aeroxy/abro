use std::collections::{BTreeMap, VecDeque};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use anyhow::Error as AnyhowError;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use serde::{Deserialize, Serialize};
use shell_integration::{
    hook_file_path, install_snippet, install_snippet_content, BoundaryParser, CommandBoundaryEvent,
    ShellKind,
};
use thiserror::Error;
use vte::{Parser as AnsiParser, Perform};

pub use shell_integration::BoundaryPhase;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SessionId(u64);

impl SessionId {
    pub fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSpec {
    pub tab_title: String,
    pub shell: String,
    pub cwd: PathBuf,
}

impl SessionSpec {
    pub fn new(
        tab_title: impl Into<String>,
        shell: impl Into<String>,
        cwd: impl Into<PathBuf>,
    ) -> Self {
        Self {
            tab_title: tab_title.into(),
            shell: shell.into(),
            cwd: cwd.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRecord {
    pub id: SessionId,
    pub spec: SessionSpec,
}

#[derive(Debug, Default)]
pub struct SessionManager {
    next_id: u64,
    active: Option<SessionId>,
    sessions: BTreeMap<SessionId, SessionRecord>,
}

impl SessionManager {
    pub fn create_session(&mut self, spec: SessionSpec) -> SessionId {
        self.next_id += 1;
        let id = SessionId(self.next_id);

        let record = SessionRecord { id, spec };
        self.sessions.insert(id, record);
        self.active = Some(id);

        id
    }

    pub fn close_session(&mut self, id: SessionId) -> bool {
        let removed = self.sessions.remove(&id).is_some();
        if !removed {
            return false;
        }

        if self.active == Some(id) {
            self.active = self.sessions.keys().next_back().copied();
        }

        true
    }

    pub fn set_active(&mut self, id: SessionId) -> bool {
        if self.sessions.contains_key(&id) {
            self.active = Some(id);
            true
        } else {
            false
        }
    }

    pub fn active(&self) -> Option<&SessionRecord> {
        self.active.and_then(|id| self.sessions.get(&id))
    }

    pub fn by_id(&self, id: SessionId) -> Option<&SessionRecord> {
        self.sessions.get(&id)
    }

    pub fn all(&self) -> Vec<&SessionRecord> {
        self.sessions.values().collect()
    }

    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }
}

#[derive(Debug, Error)]
pub enum PtyError {
    #[error("pty operation failed: {0}")]
    Pty(#[from] AnyhowError),
    #[error("io operation failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("unknown session id {0}")]
    UnknownSession(u64),
}

pub struct PtySession {
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    writer: Box<dyn Write + Send>,
    output_rx: Receiver<Vec<u8>>,
}

impl PtySession {
    pub fn spawn(
        shell: &str,
        args: &[&str],
        cwd: Option<&Path>,
        rows: u16,
        cols: u16,
    ) -> Result<Self, PtyError> {
        let pty_system = native_pty_system();
        let pair = pty_system.openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;

        let mut command = CommandBuilder::new(shell);
        command.args(args);
        if let Some(cwd) = cwd {
            command.cwd(cwd);
        }

        let child = pair.slave.spawn_command(command)?;
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader()?;
        let (output_tx, output_rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut buffer = [0u8; 4096];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(size) => {
                        if output_tx.send(buffer[..size].to_vec()).is_err() {
                            break;
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
        });

        let writer = pair.master.take_writer()?;

        Ok(Self {
            master: pair.master,
            child,
            writer,
            output_rx,
        })
    }

    pub fn write(&mut self, bytes: &[u8]) -> Result<(), PtyError> {
        self.writer.write_all(bytes)?;
        self.writer.flush()?;
        Ok(())
    }

    pub fn read_for(&mut self, timeout: Duration) -> String {
        let deadline = Instant::now() + timeout;
        let mut output = Vec::new();

        loop {
            let now = Instant::now();
            if now >= deadline {
                break;
            }

            match self
                .output_rx
                .recv_timeout(deadline.saturating_duration_since(now))
            {
                Ok(chunk) => {
                    output.extend_from_slice(&chunk);
                    while let Ok(extra) = self.output_rx.try_recv() {
                        output.extend_from_slice(&extra);
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }

        String::from_utf8_lossy(&output).to_string()
    }

    pub fn resize(&mut self, rows: u16, cols: u16) -> Result<(), PtyError> {
        self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        Ok(())
    }

    pub fn kill(&mut self) -> Result<(), PtyError> {
        self.child.kill()?;
        Ok(())
    }

    pub fn wait(&mut self) -> Result<u32, PtyError> {
        let status = self.child.wait()?;
        Ok(status.exit_code())
    }
}

#[derive(Default)]
pub struct PtySessionManager {
    next_id: u64,
    sessions: BTreeMap<SessionId, PtySession>,
}

impl PtySessionManager {
    pub fn spawn_session(
        &mut self,
        shell: &str,
        args: &[&str],
        cwd: Option<&Path>,
        rows: u16,
        cols: u16,
    ) -> Result<SessionId, PtyError> {
        self.next_id += 1;
        let id = SessionId(self.next_id);
        let session = PtySession::spawn(shell, args, cwd, rows, cols)?;
        self.sessions.insert(id, session);
        Ok(id)
    }

    pub fn write(&mut self, id: SessionId, input: &str) -> Result<(), PtyError> {
        self.session_mut(id)?.write(input.as_bytes())
    }

    pub fn read_for(&mut self, id: SessionId, timeout: Duration) -> Result<String, PtyError> {
        Ok(self.session_mut(id)?.read_for(timeout))
    }

    pub fn resize(&mut self, id: SessionId, rows: u16, cols: u16) -> Result<(), PtyError> {
        self.session_mut(id)?.resize(rows, cols)
    }

    pub fn kill(&mut self, id: SessionId) -> Result<(), PtyError> {
        self.session_mut(id)?.kill()
    }

    pub fn wait(&mut self, id: SessionId) -> Result<u32, PtyError> {
        self.session_mut(id)?.wait()
    }

    pub fn remove(&mut self, id: SessionId) -> bool {
        self.sessions.remove(&id).is_some()
    }

    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    fn session_mut(&mut self, id: SessionId) -> Result<&mut PtySession, PtyError> {
        self.sessions
            .get_mut(&id)
            .ok_or(PtyError::UnknownSession(id.raw()))
    }
}

#[derive(Debug, Error)]
pub enum TabSessionError {
    #[error(transparent)]
    Pty(#[from] PtyError),
    #[error("unknown tab id {0}")]
    UnknownTab(u64),
    #[error("no active tab")]
    NoActiveTab,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabBoundaryEvent {
    pub tab_id: SessionId,
    pub event: CommandBoundaryEvent,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TabReadChunk {
    pub output: String,
    pub boundary_events: Vec<TabBoundaryEvent>,
}

#[derive(Default)]
pub struct TabSessionManager {
    tabs: SessionManager,
    pty_sessions: PtySessionManager,
    pty_by_tab: BTreeMap<SessionId, SessionId>,
    boundary_parsers: BTreeMap<SessionId, BoundaryParser>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnsiColor {
    Named(u8),
    Rgb(u8, u8, u8),
}

impl Default for AnsiColor {
    fn default() -> Self {
        AnsiColor::Named(7) // White
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellStyle {
    pub fg: AnsiColor,
    pub bg: AnsiColor,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub dim: bool,
}

impl Default for CellStyle {
    fn default() -> Self {
        Self {
            fg: AnsiColor::Named(7),
            bg: AnsiColor::Named(0),
            bold: false,
            italic: false,
            underline: false,
            dim: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyledCell {
    pub c: char,
    pub style: CellStyle,
}

#[derive(Default)]
struct RenderPerformer {
    cells: Vec<StyledCell>,
    current_style: CellStyle,
}

impl RenderPerformer {
    fn take_cells(&mut self) -> Vec<StyledCell> {
        std::mem::take(&mut self.cells)
    }
}

impl Perform for RenderPerformer {
    fn print(&mut self, c: char) {
        self.cells.push(StyledCell {
            c,
            style: self.current_style,
        });
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            b'\n' | b'\r' | b'\t' | 0x08 => {
                self.cells.push(StyledCell {
                    c: byte as char,
                    style: self.current_style,
                });
            }
            _ => {}
        }
    }

    fn csi_dispatch(
        &mut self,
        params: &vte::Params,
        _intermediates: &[u8],
        _ignore: bool,
        action: char,
    ) {
        if action == 'm' {
            // SGR (Select Graphic Rendition) sequences
            let mut i = 0;
            let params_slice: Vec<u16> = params.iter().map(|p| p[0]).collect();
            while i < params_slice.len() {
                match params_slice[i] {
                    0 => {
                        // Reset
                        self.current_style = CellStyle::default();
                    }
                    1 => self.current_style.bold = true,
                    2 => self.current_style.dim = true,
                    3 => self.current_style.italic = true,
                    4 => self.current_style.underline = true,
                    22 => {
                        self.current_style.bold = false;
                        self.current_style.dim = false;
                    }
                    23 => self.current_style.italic = false,
                    24 => self.current_style.underline = false,
                    30..=37 => {
                        // Standard foreground colors
                        self.current_style.fg = AnsiColor::Named((params_slice[i] - 30) as u8);
                    }
                    38 => {
                        // Extended foreground
                        if i + 1 < params_slice.len() {
                            if params_slice[i + 1] == 5 && i + 2 < params_slice.len() {
                                // 256-color
                                self.current_style.fg = AnsiColor::Named(params_slice[i + 2] as u8);
                                i += 2;
                            } else if params_slice[i + 1] == 2 && i + 4 < params_slice.len() {
                                // 24-bit RGB
                                self.current_style.fg = AnsiColor::Rgb(
                                    params_slice[i + 2] as u8,
                                    params_slice[i + 3] as u8,
                                    params_slice[i + 4] as u8,
                                );
                                i += 4;
                            }
                        }
                    }
                    39 => {
                        // Default foreground
                        self.current_style.fg = AnsiColor::Named(7);
                    }
                    40..=47 => {
                        // Standard background colors
                        self.current_style.bg = AnsiColor::Named((params_slice[i] - 40) as u8);
                    }
                    48 => {
                        // Extended background
                        if i + 1 < params_slice.len() {
                            if params_slice[i + 1] == 5 && i + 2 < params_slice.len() {
                                // 256-color
                                self.current_style.bg = AnsiColor::Named(params_slice[i + 2] as u8);
                                i += 2;
                            } else if params_slice[i + 1] == 2 && i + 4 < params_slice.len() {
                                // 24-bit RGB
                                self.current_style.bg = AnsiColor::Rgb(
                                    params_slice[i + 2] as u8,
                                    params_slice[i + 3] as u8,
                                    params_slice[i + 4] as u8,
                                );
                                i += 4;
                            }
                        }
                    }
                    49 => {
                        // Default background
                        self.current_style.bg = AnsiColor::Named(0);
                    }
                    90..=97 => {
                        // Bright foreground colors
                        self.current_style.fg = AnsiColor::Named((params_slice[i] - 90 + 8) as u8);
                    }
                    100..=107 => {
                        // Bright background colors
                        self.current_style.bg = AnsiColor::Named((params_slice[i] - 100 + 8) as u8);
                    }
                    _ => {}
                }
                i += 1;
            }
        }
    }
}

pub struct TerminalRenderModel {
    parser: AnsiParser,
    performer: RenderPerformer,
    scrollback: VecDeque<Vec<StyledCell>>,
    current_line: Vec<StyledCell>,
    cursor_col: usize,
    max_lines: usize,
}

impl TerminalRenderModel {
    pub fn new(max_lines: usize) -> Self {
        Self {
            parser: AnsiParser::new(),
            performer: RenderPerformer::default(),
            scrollback: VecDeque::new(),
            current_line: Vec::new(),
            cursor_col: 0,
            max_lines: max_lines.max(1),
        }
    }

    pub fn ingest_bytes(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.performer, bytes);

        let cells = self.performer.take_cells();
        for cell in cells {
            match cell.c {
                '\n' => self.push_line(),
                '\r' => self.cursor_col = 0,
                '\x08' => {
                    self.cursor_col = self.cursor_col.saturating_sub(1);
                }
                '\t' => {
                    let tab_stop = ((self.cursor_col / 8) + 1) * 8;
                    while self.cursor_col < tab_stop {
                        self.write_cell(StyledCell {
                            c: ' ',
                            style: cell.style,
                        });
                    }
                }
                c if !c.is_control() => self.write_cell(cell),
                _ => {}
            }
        }
    }

    pub fn line_count(&self) -> usize {
        self.scrollback.len() + usize::from(!self.current_line.is_empty())
    }

    pub fn rendered_text(&self) -> String {
        let mut lines: Vec<String> = Vec::new();
        for line_cells in self.scrollback.iter() {
            lines.push(line_cells.iter().map(|c| c.c).collect());
        }
        if !self.current_line.is_empty() {
            lines.push(self.current_line.iter().map(|c| c.c).collect());
        }
        lines.join("\n")
    }

    pub fn styled_lines(&self) -> Vec<Vec<StyledCell>> {
        let mut result: Vec<Vec<StyledCell>> = self.scrollback.iter().cloned().collect();
        if !self.current_line.is_empty() {
            result.push(self.current_line.clone());
        }
        eprintln!("styled_lines: {} lines", result.len());
        result
    }

    pub fn tail(&self, max_lines: usize) -> String {
        let mut lines: Vec<String> = Vec::new();
        for line_cells in self.scrollback.iter() {
            lines.push(line_cells.iter().map(|c| c.c).collect());
        }
        if !self.current_line.is_empty() {
            lines.push(self.current_line.iter().map(|c| c.c).collect());
        }

        let len = lines.len();
        let start = len.saturating_sub(max_lines);
        lines[start..].join("\n")
    }

    fn push_line(&mut self) {
        self.scrollback
            .push_back(std::mem::take(&mut self.current_line));
        self.cursor_col = 0;
        while self.scrollback.len() > self.max_lines {
            self.scrollback.pop_front();
        }
    }

    fn write_cell(&mut self, cell: StyledCell) {
        if self.cursor_col >= self.current_line.len() {
            self.current_line.push(cell);
        } else {
            self.current_line[self.cursor_col] = cell;
        }
        self.cursor_col += 1;
    }
}

fn shell_kind_from_path(shell: &str) -> Option<ShellKind> {
    let file_name = Path::new(shell).file_name()?.to_str()?.to_ascii_lowercase();
    match file_name.as_str() {
        "bash" => Some(ShellKind::Bash),
        "zsh" => Some(ShellKind::Zsh),
        "fish" => Some(ShellKind::Fish),
        _ => None,
    }
}

impl TabSessionManager {
    pub fn open_tab(
        &mut self,
        spec: SessionSpec,
        args: &[&str],
        rows: u16,
        cols: u16,
    ) -> Result<SessionId, TabSessionError> {
        let tab_id = self.tabs.create_session(spec.clone());
        match self.pty_sessions.spawn_session(
            &spec.shell,
            args,
            Some(spec.cwd.as_path()),
            rows,
            cols,
        ) {
            Ok(pty_id) => {
                self.pty_by_tab.insert(tab_id, pty_id);
                self.boundary_parsers
                    .insert(tab_id, BoundaryParser::default());

                if let Some(shell_kind) = shell_kind_from_path(&spec.shell) {
                    let hook_path = hook_file_path(shell_kind);
                    let expanded_path = if hook_path.starts_with("~/") {
                        if let Some(home) = dirs::home_dir() {
                            home.join(hook_path.trim_start_matches("~/")).to_path_buf()
                        } else {
                            PathBuf::from(hook_path)
                        }
                    } else {
                        PathBuf::from(hook_path)
                    };

                    if let Some(parent) = expanded_path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    let _ = std::fs::write(&expanded_path, install_snippet_content(shell_kind));

                    let snippet = format!("{}\n", install_snippet(shell_kind));
                    if let Err(error) = self.pty_sessions.write(pty_id, &snippet) {
                        self.boundary_parsers.remove(&tab_id);
                        self.pty_by_tab.remove(&tab_id);
                        self.pty_sessions.remove(pty_id);
                        self.tabs.close_session(tab_id);
                        return Err(error.into());
                    }
                }

                Ok(tab_id)
            }
            Err(error) => {
                self.tabs.close_session(tab_id);
                Err(error.into())
            }
        }
    }

    pub fn close_tab(&mut self, id: SessionId) -> Result<bool, TabSessionError> {
        if !self.tabs.close_session(id) {
            return Ok(false);
        }

        self.boundary_parsers.remove(&id);
        if let Some(pty_id) = self.pty_by_tab.remove(&id) {
            self.pty_sessions.remove(pty_id);
        }

        Ok(true)
    }

    pub fn set_active_tab(&mut self, id: SessionId) -> bool {
        self.tabs.set_active(id)
    }

    pub fn active_tab_id(&self) -> Option<SessionId> {
        self.tabs.active().map(|tab| tab.id)
    }

    pub fn active_tab(&self) -> Option<&SessionRecord> {
        self.tabs.active()
    }

    pub fn tab(&self, id: SessionId) -> Option<&SessionRecord> {
        self.tabs.by_id(id)
    }

    pub fn tabs(&self) -> Vec<&SessionRecord> {
        self.tabs.all()
    }

    pub fn tab_count(&self) -> usize {
        self.tabs.session_count()
    }

    pub fn write(&mut self, tab_id: SessionId, input: &str) -> Result<(), TabSessionError> {
        let pty_id = self.pty_id_for_tab(tab_id)?;
        self.pty_sessions.write(pty_id, input)?;
        Ok(())
    }

    pub fn write_active(&mut self, input: &str) -> Result<(), TabSessionError> {
        let tab_id = self.active_tab_id().ok_or(TabSessionError::NoActiveTab)?;
        self.write(tab_id, input)
    }

    pub fn read_for(
        &mut self,
        tab_id: SessionId,
        timeout: Duration,
    ) -> Result<String, TabSessionError> {
        Ok(self.read_for_with_boundaries(tab_id, timeout)?.output)
    }

    pub fn read_active_for(&mut self, timeout: Duration) -> Result<String, TabSessionError> {
        Ok(self.read_active_for_with_boundaries(timeout)?.output)
    }

    pub fn read_for_with_boundaries(
        &mut self,
        tab_id: SessionId,
        timeout: Duration,
    ) -> Result<TabReadChunk, TabSessionError> {
        let pty_id = self.pty_id_for_tab(tab_id)?;
        let raw = self.pty_sessions.read_for(pty_id, timeout)?;
        if raw.is_empty() {
            return Ok(TabReadChunk::default());
        }

        let parser = self.boundary_parsers.entry(tab_id).or_default();
        let parsed = parser.ingest(&raw);

        Ok(TabReadChunk {
            output: parsed.output,
            boundary_events: parsed
                .events
                .into_iter()
                .map(|event| TabBoundaryEvent { tab_id, event })
                .collect(),
        })
    }

    pub fn read_active_for_with_boundaries(
        &mut self,
        timeout: Duration,
    ) -> Result<TabReadChunk, TabSessionError> {
        let tab_id = self.active_tab_id().ok_or(TabSessionError::NoActiveTab)?;
        self.read_for_with_boundaries(tab_id, timeout)
    }

    pub fn resize(
        &mut self,
        tab_id: SessionId,
        rows: u16,
        cols: u16,
    ) -> Result<(), TabSessionError> {
        let pty_id = self.pty_id_for_tab(tab_id)?;
        self.pty_sessions.resize(pty_id, rows, cols)?;
        Ok(())
    }

    pub fn kill(&mut self, tab_id: SessionId) -> Result<(), TabSessionError> {
        let pty_id = self.pty_id_for_tab(tab_id)?;
        self.pty_sessions.kill(pty_id)?;
        Ok(())
    }

    pub fn wait(&mut self, tab_id: SessionId) -> Result<u32, TabSessionError> {
        let pty_id = self.pty_id_for_tab(tab_id)?;
        Ok(self.pty_sessions.wait(pty_id)?)
    }

    fn pty_id_for_tab(&self, tab_id: SessionId) -> Result<SessionId, TabSessionError> {
        self.pty_by_tab
            .get(&tab_id)
            .copied()
            .ok_or(TabSessionError::UnknownTab(tab_id.raw()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_session_sets_it_active() {
        let mut manager = SessionManager::default();

        let id = manager.create_session(SessionSpec::new("main", "/bin/zsh", "/tmp"));
        let active = manager.active().expect("active session should exist");

        assert_eq!(active.id, id);
        assert_eq!(active.spec.shell, "/bin/zsh");
    }

    #[test]
    fn closing_active_session_falls_back_to_previous() {
        let mut manager = SessionManager::default();

        let id1 = manager.create_session(SessionSpec::new("main", "/bin/zsh", "/tmp"));
        let id2 = manager.create_session(SessionSpec::new("logs", "/bin/bash", "/tmp"));

        assert!(manager.close_session(id2));
        let active = manager.active().expect("active session should exist");

        assert_eq!(active.id, id1);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn pty_session_supports_spawn_write_resize_and_exit() {
        let mut manager = PtySessionManager::default();
        let id = manager
            .spawn_session("/bin/sh", &[], None, 24, 80)
            .expect("spawn should succeed");

        manager
            .write(id, "echo abro_t01_marker\n")
            .expect("write should succeed");

        let output = manager
            .read_for(id, Duration::from_secs(2))
            .expect("read should succeed");

        assert!(output.contains("abro_t01_marker"));

        manager.resize(id, 40, 120).expect("resize should succeed");

        manager.write(id, "exit 7\n").expect("write should succeed");

        let status = manager.wait(id).expect("wait should succeed");
        assert_eq!(status, 7);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn tab_session_manager_routes_io_by_tab_and_tracks_active() {
        let mut manager = TabSessionManager::default();

        let first = manager
            .open_tab(SessionSpec::new("first", "/bin/sh", "/tmp"), &[], 24, 80)
            .expect("first tab should open");
        let second = manager
            .open_tab(SessionSpec::new("second", "/bin/sh", "/tmp"), &[], 24, 80)
            .expect("second tab should open");

        assert_eq!(manager.tab_count(), 2);
        assert_eq!(
            manager
                .active_tab_id()
                .expect("active tab should exist")
                .raw(),
            second.raw()
        );

        assert!(manager.set_active_tab(first));
        manager
            .write_active("echo abro_tab_one\n")
            .expect("write should succeed");
        let first_output = manager
            .read_active_for(Duration::from_secs(2))
            .expect("read should succeed");
        assert!(first_output.contains("abro_tab_one"));

        manager
            .resize(first, 40, 120)
            .expect("resize should succeed");
        manager
            .write(first, "exit 3\n")
            .expect("exit should succeed");
        let first_status = manager.wait(first).expect("wait should succeed");
        assert_eq!(first_status, 3);

        assert!(manager.close_tab(first).expect("close should succeed"));
        assert_eq!(manager.tab_count(), 1);
        assert_eq!(
            manager
                .active_tab_id()
                .expect("active tab should still exist")
                .raw(),
            second.raw()
        );

        manager
            .write(second, "echo abro_tab_two\n")
            .expect("write should succeed");
        let second_output = manager
            .read_for(second, Duration::from_secs(2))
            .expect("read should succeed");
        assert!(second_output.contains("abro_tab_two"));

        manager
            .write(second, "exit 0\n")
            .expect("exit should succeed");
        let second_status = manager.wait(second).expect("wait should succeed");
        assert_eq!(second_status, 0);
        assert!(manager.close_tab(second).expect("close should succeed"));
        assert_eq!(manager.tab_count(), 0);
        assert!(manager.active_tab().is_none());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn tab_session_manager_emits_boundary_metadata_per_command() {
        let mut manager = TabSessionManager::default();
        let tab = manager
            .open_tab(SessionSpec::new("zsh", "/bin/zsh", "/tmp"), &[], 24, 80)
            .expect("tab should open");

        manager
            .write(tab, "echo abro_boundary_probe\n")
            .expect("write should succeed");

        let deadline = Instant::now() + Duration::from_secs(4);
        let mut saw_start = false;
        let mut output = String::new();

        while Instant::now() < deadline && !(saw_start && output.contains("abro_boundary_probe")) {
            let chunk = manager
                .read_for_with_boundaries(tab, Duration::from_millis(150))
                .expect("read should succeed");
            output.push_str(&chunk.output);

            for boundary in chunk.boundary_events {
                if boundary.event.phase == BoundaryPhase::Start
                    && boundary.event.payload.contains("abro_boundary_probe")
                {
                    saw_start = true;
                }
            }
        }

        assert!(
            saw_start,
            "expected start boundary event for command, output: {output}"
        );
        assert!(
            output.contains("abro_boundary_probe"),
            "expected command output in stream, output: {output}"
        );

        manager.write(tab, "exit 0\n").expect("exit should succeed");
        let _ = manager.wait(tab).expect("wait should succeed");
        assert!(manager.close_tab(tab).expect("close should succeed"));
    }

    #[test]
    fn ansi_snapshot_parses_color_sequences() {
        let mut model = TerminalRenderModel::new(32);

        model.ingest_bytes(b"\x1b[31mred");
        model.ingest_bytes(b"\x1b[0m\nplain\n");

        let snapshot = model.rendered_text();
        assert_eq!(snapshot, "red\nplain");

        let lines = model.styled_lines();
        assert_eq!(lines.len(), 2);
        // First line should have red color (ANSI 31 = named color 1)
        assert_eq!(lines[0][0].style.fg, AnsiColor::Named(1));
        // Second line should have default color after reset
        assert_eq!(lines[1][0].style.fg, AnsiColor::Named(7));
    }

    #[test]
    fn ansi_snapshot_handles_carriage_return_and_backspace() {
        let mut model = TerminalRenderModel::new(32);

        model.ingest_bytes(b"abc\x08d\rXY\n");

        let snapshot = model.rendered_text();
        assert_eq!(snapshot, "XYd");
    }

    #[test]
    fn ansi_snapshot_preserves_crlf_lines() {
        let mut model = TerminalRenderModel::new(32);

        model.ingest_bytes(b"first line\r\nsecond line\r\n");

        let snapshot = model.rendered_text();
        assert_eq!(snapshot, "first line\nsecond line");
    }

    #[test]
    fn ansi_snapshot_enforces_scrollback_limit() {
        let mut model = TerminalRenderModel::new(2);

        model.ingest_bytes(b"one\ntwo\nthree\n");

        let snapshot = model.rendered_text();
        assert_eq!(snapshot, "two\nthree");
    }

    #[test]
    fn ansi_bold_and_italic_styles() {
        let mut model = TerminalRenderModel::new(32);

        model.ingest_bytes(b"\x1b[1mbold\x1b[0m\n");
        model.ingest_bytes(b"\x1b[3mitalic\x1b[0m\n");

        let lines = model.styled_lines();
        assert!(lines[0][0].style.bold);
        assert!(!lines[0][0].style.italic);
        assert!(!lines[1][0].style.bold);
        assert!(lines[1][0].style.italic);
    }

    #[test]
    fn ansi_256_color_support() {
        let mut model = TerminalRenderModel::new(32);

        model.ingest_bytes(b"\x1b[38;5;196mred\x1b[0m\n");

        let lines = model.styled_lines();
        assert_eq!(lines[0][0].style.fg, AnsiColor::Named(196));
    }

    #[test]
    fn ansi_rgb_color_support() {
        let mut model = TerminalRenderModel::new(32);

        model.ingest_bytes(b"\x1b[38;2;255;0;0mred\x1b[0m\n");

        let lines = model.styled_lines();
        assert_eq!(lines[0][0].style.fg, AnsiColor::Rgb(255, 0, 0));
    }
}
