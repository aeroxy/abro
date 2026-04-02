use anyhow::Result;
use portable_pty::{CommandBuilder, MasterPty, NativePtySystem, PtySize, PtySystem};
use serde::{Deserialize, Serialize};
use shell_integration::{
    hook_file_path, install_snippet_content, BoundaryParser, BoundaryPhase, ShellKind,
};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::thread;
use tauri::{AppHandle, Emitter};

#[cfg(unix)]
use std::os::unix::io::RawFd;

#[derive(Clone, Serialize, Deserialize)]
pub struct PtyOutputEvent {
    pub id: String,
    pub data: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CmdStartEvent {
    pub id: String,
    pub command: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CmdEndEvent {
    pub id: String,
    pub exit_code: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CmdCwdEvent {
    pub id: String,
    pub cwd: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SessionEndedEvent {
    pub id: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct PasswordModeEvent {
    pub id: String,
    pub active: bool,
}

pub struct PtyManager {
    master: Arc<Mutex<Option<Box<dyn MasterPty + Send>>>>,
    writer: Arc<Mutex<Option<Box<dyn Write + Send>>>>,
    pub id: String,
}

impl PtyManager {
    pub fn new() -> Self {
        Self {
            master: Arc::new(Mutex::new(None)),
            writer: Arc::new(Mutex::new(None)),
            id: uuid::Uuid::new_v4().to_string(),
        }
    }

    pub fn spawn(&self, app: AppHandle) -> Result<()> {
        let pty_system = NativePtySystem::default();
        let pair = pty_system.openpty(PtySize {
            rows: 24,
            cols: 120, // Make it wider by default to avoid wrapping
            pixel_width: 0,
            pixel_height: 0,
        })?;

        // Determine default shell (default to zsh on Mac)
        let shell_path = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
        let mut cmd = CommandBuilder::new(&shell_path);
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");

        // Determine shell kind based on path
        let file_name = std::path::Path::new(&shell_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();

        let shell_kind = match file_name.as_str() {
            "bash" => ShellKind::Bash,
            "fish" => ShellKind::Fish,
            _ => ShellKind::Zsh, // Default to zsh
        };

        let hook_path = hook_file_path(shell_kind);
        let home_dir = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("~"));
        let expanded_path = if hook_path.starts_with("~/") {
            home_dir.join(hook_path.trim_start_matches("~/"))
        } else {
            std::path::PathBuf::from(&hook_path)
        };

        if let Some(parent) = expanded_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&expanded_path, install_snippet_content(shell_kind));

        // Proxy injection to avoid adding injection commands to shell history
        let abro_dir = home_dir.join(".abro");
        match shell_kind {
            ShellKind::Zsh => {
                let zsh_dir = abro_dir.join("zsh");
                let _ = std::fs::create_dir_all(&zsh_dir);
                for file in [".zshenv", ".zprofile", ".zlogin", ".zshrc"] {
                    let mut content = format!(
                        "if [ -f \"$HOME/{}\" ]; then\n    ZDOTDIR=$HOME source \"$HOME/{}\"\nfi\n",
                        file, file
                    );
                    if file == ".zshrc" {
                        content.push_str(&format!(
                            "source \"$HOME/.abro/hooks-{}.sh\"\n",
                            shell_kind.as_str()
                        ));
                    }
                    let _ = std::fs::write(zsh_dir.join(file), content);
                }
                cmd.env("ZDOTDIR", zsh_dir);
            }
            ShellKind::Bash => {
                let bash_dir = abro_dir.join("bash");
                let _ = std::fs::create_dir_all(&bash_dir);
                let bashrc_path = bash_dir.join(".bashrc");
                let content = format!(
                    "if [ -f \"$HOME/.bashrc\" ]; then\n    source \"$HOME/.bashrc\"\nfi\nsource \"$HOME/.abro/hooks-{}.sh\"\n",
                    shell_kind.as_str()
                );
                let _ = std::fs::write(&bashrc_path, content);
                cmd.arg("--rcfile");
                cmd.arg(bashrc_path);
            }
            _ => {
                // Future fish support
            }
        }

        let mut child = pair.slave.spawn_command(cmd)?;
        let mut reader = pair.master.try_clone_reader()?;

        // Grab the master's raw fd before moving it, for ECHO flag detection
        #[cfg(unix)]
        let master_fd = pair.master.as_raw_fd();

        let writer = pair.master.take_writer()?;

        // Store writer and master for writing and resizing
        *self.writer.lock().unwrap() = Some(writer);
        *self.master.lock().unwrap() = Some(pair.master);

        let id = self.id.clone();

        // Output reader thread
        thread::spawn(move || {
            let mut buf = [0u8; 8192];
            let mut parser = BoundaryParser::default();
            #[cfg(unix)]
            let mut prev_echo_off = false;
            #[cfg(unix)]
            let mut inside_command = false;

            loop {
                match reader.read(&mut buf) {
                    Ok(n) if n > 0 => {
                        let chunk_str = String::from_utf8_lossy(&buf[..n]);
                        let parsed = parser.ingest(&chunk_str);

                        // Emit Start events first so the frontend block exists
                        // before output or End events arrive.
                        for event in &parsed.events {
                            if event.phase == BoundaryPhase::Start {
                                #[cfg(unix)]
                                {
                                    inside_command = true;
                                }
                                let _ = app.emit(
                                    "pty-cmd-start",
                                    CmdStartEvent {
                                        id: id.clone(),
                                        command: event.payload.clone(),
                                    },
                                );
                            }
                        }

                        // Then emit command output while the block is still active.
                        if !parsed.output.is_empty() {
                            let _ = app.emit(
                                "pty-output",
                                PtyOutputEvent {
                                    id: id.clone(),
                                    data: parsed.output,
                                },
                            );
                        }

                        // Finally emit End/Cwd events to close the block.
                        for event in &parsed.events {
                            match event.phase {
                                BoundaryPhase::End => {
                                    #[cfg(unix)]
                                    {
                                        inside_command = false;
                                        // Reset ECHO tracking when command ends
                                        if prev_echo_off {
                                            prev_echo_off = false;
                                            let _ = app.emit(
                                                "pty-password-mode",
                                                PasswordModeEvent {
                                                    id: id.clone(),
                                                    active: false,
                                                },
                                            );
                                        }
                                    }
                                    let _ = app.emit(
                                        "pty-cmd-end",
                                        CmdEndEvent {
                                            id: id.clone(),
                                            exit_code: event.payload.clone(),
                                        },
                                    );
                                }
                                BoundaryPhase::Cwd => {
                                    let _ = app.emit(
                                        "pty-cmd-cwd",
                                        CmdCwdEvent {
                                            id: id.clone(),
                                            cwd: event.payload.clone(),
                                        },
                                    );
                                }
                                BoundaryPhase::Start => {}
                            }
                        }

                        // Only check ECHO flag while a command is actively running.
                        // During shell init or between commands, ECHO may be off
                        // without it meaning a password prompt.
                        #[cfg(unix)]
                        if inside_command {
                            if let Some(fd) = master_fd {
                                let echo_off = is_echo_disabled(fd);
                                if echo_off != prev_echo_off {
                                    prev_echo_off = echo_off;
                                    let _ = app.emit(
                                        "pty-password-mode",
                                        PasswordModeEvent {
                                            id: id.clone(),
                                            active: echo_off,
                                        },
                                    );
                                }
                            }
                        }
                    }
                    _ => break, // EOF or Error
                }
            }

            let _ = child.wait();
            let _ = app.emit("pty-session-ended", SessionEndedEvent { id: id.clone() });
        });

        Ok(())
    }

    pub fn write(&self, data: String) -> Result<()> {
        if let Some(writer) = self.writer.lock().unwrap().as_mut() {
            writer.write_all(data.as_bytes())?;
        }
        Ok(())
    }

    pub fn resize(&self, rows: u16, cols: u16) -> Result<()> {
        if let Some(master) = self.master.lock().unwrap().as_ref() {
            master.resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })?;
        }
        Ok(())
    }

    pub fn kill(&self) {
        // Drop writer first to close the pipe
        self.writer.lock().unwrap().take();
        // Dropping the master closes the PTY and signals the child
        self.master.lock().unwrap().take();
    }
}

/// Check if the terminal's ECHO flag is disabled (indicates password input mode).
#[cfg(unix)]
fn is_echo_disabled(fd: RawFd) -> bool {
    unsafe {
        let mut termios: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(fd, &mut termios) == 0 {
            (termios.c_lflag & libc::ECHO) == 0
        } else {
            false
        }
    }
}
