import React, { useState, useRef, useEffect } from 'react';
import { RefreshCw, ChevronRight, Folder, Lock, Shield, Eye, EyeOff, Sparkles, TerminalSquare, Terminal, Square } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { ToolCallModal } from './ToolCallModal';

interface Block {
  id: string;
  command: string;
  output: string;
  status: 'running' | 'success' | 'error';
  timestamp: number;
  cwd?: string;
  duration?: number;
  type: 'command' | 'ai';
  toolCall?: {
    name: string;
    args: { command: string; explanation: string };
    state: 'pending' | 'approved' | 'rejected' | 'completed';
    outputSplitIndex: number; // where in output the tool call was inserted
    result?: {
      stdout: string;
      stderr: string;
      exitCode: number;
    };
  };
}

interface AiResponseChunkEvent {
  session_id: string;
  block_id: string;
  delta: string;
}

interface AiResponseDoneEvent {
  session_id: string;
  block_id: string;
}

interface AiResponseErrorEvent {
  session_id: string;
  block_id: string;
  error: string;
}

interface AiToolCallEvent {
  session_id: string;
  block_id: string;
  tool_call: {
    name: string;
    args: Record<string, any>;
  };
}

interface PtyOutputEvent {
  id: string;
  data: string;
}

interface CmdStartEvent {
  id: string;
  command: string;
}

interface CmdEndEvent {
  id: string;
  exit_code: string;
}

interface CmdCwdEvent {
  id: string;
  cwd: string;
}

interface SessionEndedEvent {
  id: string;
}

interface PasswordModeEvent {
  id: string;
  active: boolean;
}

interface TerminalSessionProps {
  isActive: boolean;
  onCwdChange: (cwd: string) => void;
  onSessionEnd?: () => void;
}

export const TerminalSession: React.FC<TerminalSessionProps> = ({ isActive, onCwdChange, onSessionEnd }) => {
  const [ptyId, setPtyId] = useState<string | null>(null);
  const [blocks, setBlocks] = useState<Block[]>([]);
  const [input, setInput] = useState('');
  const [cwd, setCwd] = useState<string>('~');
  
  const [history, setHistory] = useState<string[]>([]);
  const [showHistory, setShowHistory] = useState(false);
  const [historyIndex, setHistoryIndex] = useState(-1);
  const historyListRef = useRef<HTMLDivElement>(null);
  
  const [completions, setCompletions] = useState<string[]>([]);
  const [showCompletions, setShowCompletions] = useState(false);
  const [completionIndex, setCompletionIndex] = useState(-1);
  const completionsListRef = useRef<HTMLDivElement>(null);

  const [isAiMode, setIsAiMode] = useState(false);
  const [isPasswordMode, setIsPasswordMode] = useState(false);
  const [passwordInput, setPasswordInput] = useState('');
  const [passwordVisible, setPasswordVisible] = useState(false);
  const passwordInputRef = useRef<HTMLInputElement>(null);
  const [showToolCallModal, setShowToolCallModal] = useState<{
    blockId: string;
    toolCall: { name: string; args: any };
  } | null>(null);

  const endRef = useRef<HTMLDivElement>(null);
  const activeBlockIdRef = useRef<string | null>(null);
  const outputContainerRef = useRef<HTMLDivElement>(null);
  const cwdRef = useRef<string>('~');
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const unlistenersRef = useRef<UnlistenFn[]>([]);
  const ptyIdRef = useRef<string | null>(null);

  const updateCwd = (newCwd: string) => {
    setCwd(newCwd);
    cwdRef.current = newCwd;
    onCwdChange(newCwd);
  };

  useEffect(() => {
    if (isActive) {
      endRef.current?.scrollIntoView({ behavior: 'smooth' });
      // When becoming active, focus the textarea
      setTimeout(() => {
        textareaRef.current?.focus();
      }, 0);
    }
  }, [blocks, isActive]);

  // Handle history scrolling
  useEffect(() => {
    if (showHistory && historyListRef.current) {
      const activeEl = historyListRef.current.children[historyIndex] as HTMLElement;
      if (activeEl) {
        activeEl.scrollIntoView({ block: 'nearest' });
      }
    }
  }, [historyIndex, showHistory]);

  // Handle completion scrolling
  useEffect(() => {
    if (showCompletions && completionsListRef.current) {
      const activeEl = completionsListRef.current.children[completionIndex] as HTMLElement;
      if (activeEl) {
        activeEl.scrollIntoView({ block: 'nearest' });
      }
    }
  }, [completionIndex, showCompletions]);

  // Handle window resize to adjust PTY columns
  useEffect(() => {
    if (!ptyId || !outputContainerRef.current || !isActive) return;

    const resizeObserver = new ResizeObserver(entries => {
      for (let entry of entries) {
        // Approximate columns based on width and average monospace char width (e.g. 8.4px)
        const width = entry.contentRect.width;
        const cols = Math.max(80, Math.floor((width - 64) / 8.4)); // subtract paddings
        const rows = 24; // We aren't doing full-screen curses apps yet so rows matter less
        invoke('resize_pty', { id: ptyId, rows, cols }).catch(console.error);
      }
    });

    resizeObserver.observe(outputContainerRef.current);
    return () => resizeObserver.disconnect();
  }, [ptyId, isActive]);

  // Load shell history on mount
  useEffect(() => {
    invoke<string[]>('get_shell_history')
      .then(hist => {
        if (hist && hist.length > 0) {
          setHistory(hist);
        }
      })
      .catch(console.error);
  }, []);

  // Spawn PTY on mount
  useEffect(() => {
    let unlisteners: UnlistenFn[] = [];

    const initPty = async () => {
      try {
        const id = await invoke<string>('spawn_pty');
        setPtyId(id);
        ptyIdRef.current = id;

        const unlistenOutput = await listen<PtyOutputEvent>('pty-output', (event) => {
          if (event.payload.id !== id) return;
          const activeId = activeBlockIdRef.current;

          if (activeId) {
            setBlocks(prev => prev.map(b =>
              b.id === activeId ? { ...b, output: b.output + event.payload.data } : b
            ));
          }
        });

        const unlistenStart = await listen<CmdStartEvent>('pty-cmd-start', (event) => {
          if (event.payload.id !== id) return;
          const blockId = Date.now().toString();

          let displayCmd = event.payload.command;

          // Clean up ZSH escaping if present (ZSH escapes spaces in preexec sometimes)
          displayCmd = displayCmd.replace(/\\ /g, ' ');

          // Deduplicate start events.
          // Remote SSH shells might echo the injection string multiple times causing fake blocks.
          // If the command is an exact duplicate of the actively running command, ignore it.
          // Or if it contains our base64 payload, intercept it and format it cleanly.
          if (displayCmd.includes('echo ') && displayCmd.includes(' | base64') && displayCmd.includes('--rcfile /tmp/.abro_rc_')) {
             const match = displayCmd.match(/^(ssh.*?) -t '/);
             if (match) {
                 displayCmd = match[1];
             }
          } else if (displayCmd.includes('__abro_hooks_installed')) {
             // Ignore raw bootstrap script echoes from being treated as start events
             return;
          }

          // Set the ref immediately (outside setBlocks) so that
          // pty-output events arriving before React flushes the
          // batched state update can find the active block id.
          const previousActiveId = activeBlockIdRef.current;
          activeBlockIdRef.current = blockId;

          setBlocks(prev => {
            // Prevent duplicate blocks from multiple preexec calls firing simultaneously for the same command
            if (previousActiveId) {
                const activeBlock = prev.find(b => b.id === previousActiveId);
                // Also ignore consecutive identical commands if they're still marked as running
                // (e.g. from local preexec + remote preexec double-firing)
                if (activeBlock && activeBlock.command === displayCmd && activeBlock.status === 'running') {
                    activeBlockIdRef.current = previousActiveId;
                    return prev;
                }
            }

            return [...prev, {
              id: blockId,
              command: displayCmd,
              output: '',
              status: 'running',
              timestamp: Date.now(),
              cwd: cwdRef.current,
              type: 'command' as const,
            }];
          });
        });

        const unlistenEnd = await listen<CmdEndEvent>('pty-cmd-end', (event) => {
          if (event.payload.id !== id) return;
          const activeId = activeBlockIdRef.current;

          if (activeId) {
            setBlocks(prev => prev.map(b =>
              b.id === activeId ? {
                ...b,
                // Trim trailing newline from output for cleaner display
                output: b.output.replace(/\n$/, ''),
                status: event.payload.exit_code === '0' ? 'success' : 'error',
                duration: (Date.now() - b.timestamp) / 1000
              } : b
            ));
            activeBlockIdRef.current = null;
          }
          setIsPasswordMode(false);
        });

        const unlistenCwd = await listen<CmdCwdEvent>('pty-cmd-cwd', (event) => {
          if (event.payload.id !== id) return;
          // Format home dir as ~
          const home = '\/Users\/' + event.payload.cwd.split('/')[2];
          let displayCwd = event.payload.cwd;
          if (displayCwd.startsWith(home)) {
            displayCwd = '~' + displayCwd.slice(home.length);
          }
          updateCwd(displayCwd);
        });

        unlisteners.push(unlistenOutput, unlistenStart, unlistenEnd, unlistenCwd);

        const unlistenSessionEnd = await listen<SessionEndedEvent>('pty-session-ended', (event) => {
          if (event.payload.id !== id) return;
          activeBlockIdRef.current = null;
          setIsPasswordMode(false);
          onSessionEnd?.();
        });

        const unlistenPasswordMode = await listen<PasswordModeEvent>('pty-password-mode', (event) => {
          if (event.payload.id !== id) return;
          setIsPasswordMode(event.payload.active);
          if (event.payload.active) {
            setPasswordInput('');
            setPasswordVisible(false);
            setTimeout(() => passwordInputRef.current?.focus(), 0);
          }
        });

        unlisteners.push(unlistenSessionEnd, unlistenPasswordMode);

        // AI response event listeners
        const unlistenAiChunk = await listen<AiResponseChunkEvent>('ai-response-chunk', (event) => {
          if (event.payload.session_id !== id) return;
          setBlocks(prev => prev.map(b =>
            b.id === event.payload.block_id ? { ...b, output: b.output + event.payload.delta } : b
          ));
        });

        const unlistenAiDone = await listen<AiResponseDoneEvent>('ai-response-done', (event) => {
          if (event.payload.session_id !== id) return;
          setBlocks(prev => prev.map(b => {
            if (b.id !== event.payload.block_id) return b;
            // Don't mark as success if there's a pending/approved tool call
            // (still waiting for user approval or command execution)
            if (b.toolCall && (b.toolCall.state === 'pending' || b.toolCall.state === 'approved')) {
              return b;
            }
            return {
              ...b,
              status: 'success' as const,
              duration: (Date.now() - b.timestamp) / 1000
            };
          }));
        });

        const unlistenAiError = await listen<AiResponseErrorEvent>('ai-response-error', (event) => {
          if (event.payload.session_id !== id) return;
          setBlocks(prev => prev.map(b =>
            b.id === event.payload.block_id ? {
              ...b,
              output: b.output + '\n\nError: ' + event.payload.error,
              status: 'error' as const,
              duration: (Date.now() - b.timestamp) / 1000
            } : b
          ));
        });

        const unlistenToolCall = await listen<AiToolCallEvent>('ai-tool-call', (event) => {
          if (event.payload.session_id !== id) return;
          setBlocks(prev => prev.map(b =>
            b.id === event.payload.block_id ? {
              ...b,
              toolCall: {
                name: event.payload.tool_call.name,
                args: {
                  command: event.payload.tool_call.args.command || '',
                  explanation: event.payload.tool_call.args.explanation || '',
                },
                state: 'pending' as const,
                outputSplitIndex: b.output.length, // mark where tool call appears in output
              },
            } : b
          ));
          setShowToolCallModal({ blockId: event.payload.block_id, toolCall: event.payload.tool_call });
        });

        unlisteners.push(unlistenAiChunk, unlistenAiDone, unlistenAiError, unlistenToolCall);
        unlistenersRef.current = unlisteners;
      } catch (e) {
        console.error("Failed to spawn PTY", e);
      }
    };

    initPty();

    return () => {
      unlisteners.forEach(unlisten => unlisten());
      // Close PTY when component unmounts (tab closed)
      if (ptyIdRef.current) {
        invoke('close_pty', { id: ptyIdRef.current }).catch(() => {});
      }
    };
  }, []);

  const applyCompletion = (completion: string) => {
    // If the input ends with a space, we don't pop the last token (because there isn't an incomplete one to pop!)
    const isNewToken = input.endsWith(' ');
    const tokens = input.trimEnd().split(' ');
    
    if (!isNewToken) {
        tokens.pop(); // remove incomplete token
    }
    
    const newCmd = tokens.join(' ') + (tokens.length > 0 ? ' ' : '') + completion;
    setInput(newCmd);
    setShowCompletions(false);
    setCompletionIndex(-1);
    
    // Maintain focus
    setTimeout(() => {
      textareaRef.current?.focus();
    }, 0);
  };

  const handleTab = async (e: React.KeyboardEvent) => {
    e.preventDefault();
    if (showHistory) setShowHistory(false);

    if (showCompletions && completions.length > 0) {
      const nextIndex = (completionIndex + 1) % completions.length;
      setCompletionIndex(nextIndex);
      return;
    }

    if (!input.trim()) return;

    try {
      const res = await invoke<string[]>('get_completions', { cwd: cwdRef.current, input });
      if (res && res.length === 1) {
        applyCompletion(res[0]);
      } else if (res && res.length > 1) {
        setCompletions(res);
        setShowCompletions(true);
        setCompletionIndex(0);
      }
    } catch (e) {
      console.error('Failed to get completions', e);
    }
  };

  const hasRunningCommand = blocks.some(b => b.type === 'command' && b.status === 'running');

  // Ctrl+C to abort running command
  useEffect(() => {
    if (!hasRunningCommand || !ptyId) return;
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'c' && (e.ctrlKey || e.metaKey) && !e.shiftKey) {
        e.preventDefault();
        invoke('write_pty', { id: ptyId, data: '\x03' });
      }
    };
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [hasRunningCommand, ptyId]);

  const handleStopCommand = () => {
    if (ptyId) {
      invoke('write_pty', { id: ptyId, data: '\x03' });
    }
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!input.trim() || !ptyId || hasRunningCommand) return;

    let cmd = input;
    setInput(''); // Clear input immediately for snappy UX

    if (cmd.trim()) {
      setHistory(prev => {
        const filtered = prev.filter(c => c !== cmd.trim());
        const updated = [...filtered, cmd.trim()];
        return updated.length > 200 ? updated.slice(updated.length - 200) : updated;
      });
    }
    setShowHistory(false);
    setHistoryIndex(-1);
    setShowCompletions(false);
    setCompletionIndex(-1);

    // SSH Wrapper Logic
    if (cmd.trim().startsWith('ssh ') && !cmd.includes('"') && !cmd.includes("'")) {
      const hookContent = `set +H 2>/dev/null; if [[ -z "\${__abro_hooks_installed:-}" ]]; then __abro_hooks_installed=1; __abro_preexec() { local cmd="$BASH_COMMAND"; printf '__ABRO_BOUNDARY__:bash:start:%s\\n' "$cmd"; }; trap '__abro_preexec' DEBUG; __abro_precmd() { local exit_code=$?; printf '__ABRO_BOUNDARY__:bash:end:%s\\n' "$exit_code"; printf '__ABRO_BOUNDARY__:bash:cwd:%s\\n' "$PWD"; }; PROMPT_COMMAND="__abro_precmd\${PROMPT_COMMAND:+;$PROMPT_COMMAND}"; fi`;
      const hookFile = '~/.abro/hooks-bash.sh';
      const hookB64 = btoa(hookContent);
      const remotePayload = `
mkdir -p ~/.abro
echo ${hookB64} | base64 -d > ${hookFile} 2>/dev/null || echo ${hookB64} | base64 --decode > ${hookFile} 2>/dev/null
 source ${hookFile}
if [ -f /run/motd.dynamic ]; then cat /run/motd.dynamic; fi
if [ -f /etc/motd ]; then cat /etc/motd; fi
if [ -f ~/.bashrc ]; then source ~/.bashrc; fi
bash -i
`;
      const payloadB64 = btoa(remotePayload);
      cmd = ` ${cmd.trim()} -t 'echo ${payloadB64} | base64 -d | bash 2>/dev/null || echo ${payloadB64} | base64 --decode | bash 2>/dev/null'`;
    }

    try {
      // Just send the command. The backend hooks (preexec) will trigger the block creation!
      await invoke('write_pty', { id: ptyId, data: cmd + '\n' });
    } catch (e) {
      console.error("Failed to write to PTY", e);
    }
  };

  const handleAiSubmit = async (prompt: string) => {
    if (!prompt.trim() || !ptyId) return;

    const blockId = Date.now().toString();

    // Collect recent history (last 20 blocks)
    const history = buildHistory(blocks);

    setBlocks(prev => [...prev, {
      id: blockId,
      command: prompt,
      output: '',
      status: 'running',
      timestamp: Date.now(),
      cwd,
      type: 'ai',
    }]);

    setInput('');
    setShowHistory(false);
    setHistoryIndex(-1);
    setShowCompletions(false);
    setCompletionIndex(-1);

    try {
      await invoke('send_ai_message', {
        sessionId: ptyId,
        blockId,
        prompt,
        history,
      });
    } catch (e: any) {
      setBlocks(prev => prev.map(b =>
        b.id === blockId ? { ...b, output: 'Error: ' + (e?.toString() || 'unknown error'), status: 'error' as const, duration: 0 } : b
      ));
    }
  };

  const buildHistory = (sourceBlocks: Block[]) => {
    return sourceBlocks.slice(-20).map(b => ({
      command: b.command,
      output: b.output,
      type: b.type,
      toolCall: b.toolCall ? {
        name: b.toolCall.name,
        args: b.toolCall.args,
        state: b.toolCall.state,
        result: b.toolCall.result,
      } : undefined,
    }));
  };

  const handlePasswordSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!ptyId) return;
    try {
      await invoke('write_pty', { id: ptyId, data: passwordInput + '\n' });
    } catch (e) {
      console.error("Failed to write password to PTY", e);
    }
    setPasswordInput('');
  };

  const handleToolApprove = async (editedCommand?: string) => {
    if (!showToolCallModal || !ptyId) return;

    const { blockId, toolCall } = showToolCallModal;
    const commandToRun = editedCommand || toolCall.args.command;

    // Update block state to approved
    setBlocks(prev => prev.map(b =>
      b.id === blockId && b.toolCall
        ? { ...b, toolCall: { ...b.toolCall, state: 'approved' } }
        : b
    ));

    setShowToolCallModal(null);

    // Execute the command directly (not through PTY) to reliably capture output
    try {
      const result = await invoke<{ stdout: string; stderr: string; exit_code: number }>('execute_tool_command', {
        command: commandToRun,
        cwd: cwd,
      });

      // Update block with completed tool call result
      setBlocks(prev => prev.map(b =>
        b.id === blockId && b.toolCall
          ? {
              ...b,
              toolCall: {
                ...b.toolCall,
                state: 'completed' as const,
                result: {
                  stdout: result.stdout,
                  stderr: result.stderr,
                  exitCode: result.exit_code,
                },
              },
            }
          : b
      ));

      // Send result back to AI (outside setBlocks to avoid React side-effect issues)
      const history = buildHistory(blocks);
      await invoke('continue_ai_with_tool_result', {
        sessionId: ptyId,
        blockId,
        toolName: toolCall.name,
        toolResult: {
          stdout: result.stdout,
          stderr: result.stderr,
          exitCode: result.exit_code,
        },
        history,
      });
    } catch (e: any) {
      // Command execution or AI continuation failed - show error in block
      setBlocks(prev => prev.map(b =>
        b.id === blockId
          ? {
              ...b,
              output: b.output + '\n\nError: ' + (e?.toString() || 'unknown error'),
              status: 'error' as const,
              toolCall: b.toolCall
                ? {
                    ...b.toolCall,
                    state: 'completed' as const,
                    result: b.toolCall.result || {
                      stdout: '',
                      stderr: e?.toString() || 'Command execution failed',
                      exitCode: -1,
                    },
                  }
                : undefined,
            }
          : b
      ));
    }
  };

  const handleToolReject = async () => {
    if (!showToolCallModal || !ptyId) return;

    const { blockId, toolCall } = showToolCallModal;

    // Update block state to rejected
    setBlocks(prev => prev.map(b =>
      b.id === blockId && b.toolCall
        ? { ...b, toolCall: { ...b.toolCall, state: 'rejected' } }
        : b
    ));

    // Send rejection back to AI so it can respond
    const history = buildHistory(blocks);

    try {
      await invoke('continue_ai_with_tool_result', {
        sessionId: ptyId,
        blockId,
        toolName: toolCall.name,
        toolResult: {
          stdout: '',
          stderr: 'User rejected this command',
          exitCode: -1,
        },
        history,
      });
    } catch (e) {
      console.error('Failed to send rejection to AI:', e);
    }

    setShowToolCallModal(null);
  };

  return (
    <div className={`flex-col flex-1 overflow-hidden ${isActive ? 'flex' : 'hidden'}`}>
      <div ref={outputContainerRef} className="flex-1 overflow-y-auto p-4 space-y-6">
        {blocks.length === 0 && ptyId && (
          <div className="text-center text-[#8b949e] mt-10 font-mono text-xl opacity-50">
            <p>&gt;abro_</p>
          </div>
        )}
        {blocks.map((block) => (
          <div key={block.id} className={`group relative border rounded-lg overflow-hidden ${block.type === 'ai' ? 'border-[#bc8cff]/30 bg-[#161b22]' : 'border-[#30363d] bg-[#161b22]'}`}>
            <div className="flex justify-between items-center px-4 pt-2 pb-1 bg-[#21262d]">
              <div className="flex items-center space-x-2 text-xs text-[#8b949e]">
                <Folder className="w-3.5 h-3.5" />
                <span>{block.cwd || '~'}</span>
              </div>
              {block.duration !== undefined && (
                <span className="text-xs text-[#8b949e]">({block.duration.toFixed(3)}s)</span>
              )}
            </div>

            <div className={`flex items-center px-4 pb-2 pt-1 bg-[#21262d] border-b ${block.type === 'ai' ? 'border-[#bc8cff]/20' : 'border-[#30363d]'}`}>
              {block.type === 'ai' ? (
                <Sparkles className="w-4 h-4 mr-2 text-[#bc8cff]" />
              ) : (
                <ChevronRight className="w-4 h-4 mr-2 text-[#58a6ff]" />
              )}
              <div className="font-mono text-sm text-[#e6edf3] flex-1">{block.command}</div>

              <div className="flex items-center space-x-2 text-xs text-[#8b949e] opacity-0 group-hover:opacity-100 transition-opacity">
                {block.status === 'running' && <RefreshCw className="w-3 h-3 animate-spin" />}
                {block.status === 'success' && <div className={`w-2 h-2 rounded-full ${block.type === 'ai' ? 'bg-[#bc8cff]' : 'bg-[#3fb950]'}`} />}
                {block.status === 'error' && <div className="w-2 h-2 rounded-full bg-[#f85149]" />}
                <span>{new Date(block.timestamp).toLocaleTimeString([], { hour12: false })}</span>
              </div>
            </div>

            {(() => {
              const tc = block.toolCall;
              const outputBefore = tc ? block.output.slice(0, tc.outputSplitIndex) : block.output;
              const outputAfter = tc ? block.output.slice(tc.outputSplitIndex) : '';

              return (
                <>
                  {outputBefore && (
                    <div className={`p-4 bg-[#0d1117] mono-text text-[13px] leading-tight ${block.type === 'ai' ? 'ai-text text-[#c9d1d9]' : 'text-[#8b949e] overflow-x-auto'}`}>
                      {outputBefore}
                    </div>
                  )}

                  {tc && (
                    <div className={`m-2 p-3 rounded-lg border ${
                      tc.state === 'pending' ? 'border-[#f0883e] bg-[#f0883e]/10' :
                      tc.state === 'approved' ? 'border-[#3fb950] bg-[#3fb950]/10' :
                      tc.state === 'rejected' ? 'border-[#f85149] bg-[#f85149]/10' :
                      'border-[#30363d]'
                    }`}>
                      <div className="flex items-center space-x-2 mb-1">
                        <Terminal className="w-3.5 h-3.5" />
                        <span className="text-xs font-medium">
                          {tc.state === 'pending' && 'Waiting for approval'}
                          {tc.state === 'approved' && 'Executing...'}
                          {tc.state === 'rejected' && 'Rejected by user'}
                          {tc.state === 'completed' && 'Completed'}
                        </span>
                      </div>
                      <div className="mt-1 px-2 py-1 bg-[#0d1117] rounded text-xs font-mono text-[#8b949e]">
                        $ {tc.args.command}
                      </div>
                      {tc.result && (tc.result.stdout || tc.result.stderr) && (
                        <pre className="mt-1 px-2 py-1 bg-[#0d1117] rounded text-xs font-mono text-[#c9d1d9] whitespace-pre-wrap max-h-48 overflow-y-auto">
                          {tc.result.stdout}
                          {tc.result.stderr && (
                            <span className="text-[#f85149]">{tc.result.stderr}</span>
                          )}
                        </pre>
                      )}
                    </div>
                  )}

                  {outputAfter && (
                    <div className={`p-4 bg-[#0d1117] mono-text text-[13px] leading-tight ${block.type === 'ai' ? 'ai-text text-[#c9d1d9]' : 'text-[#8b949e] overflow-x-auto'}`}>
                      {outputAfter}
                    </div>
                  )}
                </>
              );
            })()}
          </div>
        ))}
        <div ref={endRef} />
      </div>

      {/* Input area with CWD indicator */}
      <div className="px-3 pb-3 pt-1.5 bg-[#161b22] border-t border-[#30363d] shrink-0">
        {isPasswordMode ? (
          <>
            <div className="flex items-center space-x-2 mb-1 px-1">
              <Lock className="w-3.5 h-3.5 text-[#f0883e]" />
              <span className="text-xs text-[#f0883e] font-medium">Password Required</span>
            </div>
            <form onSubmit={handlePasswordSubmit} className="flex flex-col space-y-2 relative">
              <div className="flex items-center relative">
                <div className="absolute left-3 top-3 text-[#f0883e]">
                  <Shield className="w-5 h-5" />
                </div>
                <input
                  ref={passwordInputRef}
                  type={passwordVisible ? 'text' : 'password'}
                  value={passwordInput}
                  onChange={(e) => setPasswordInput(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === 'Escape') {
                      invoke('write_pty', { id: ptyId!, data: '\x03' });
                      setIsPasswordMode(false);
                      setPasswordInput('');
                    }
                  }}
                  placeholder="Enter password..."
                  autoFocus
                  autoComplete="off"
                  autoCapitalize="none"
                  autoCorrect="off"
                  spellCheck={false}
                  className="w-full bg-[#0d1117] border border-[#30363d] rounded-lg py-3 pl-10 pr-12 text-sm font-mono text-[#c9d1d9] focus:outline-none focus:ring-1 focus:ring-[#f0883e] focus:border-[#f0883e]"
                />
                <button
                  type="button"
                  onClick={() => setPasswordVisible(!passwordVisible)}
                  className="absolute right-3 top-3 text-[#8b949e] hover:text-[#c9d1d9] transition-colors"
                >
                  {passwordVisible ? <EyeOff className="w-5 h-5" /> : <Eye className="w-5 h-5" />}
                </button>
              </div>
            </form>
          </>
        ) : (
          <>
            <div className="flex items-center space-x-2 mb-1 px-1">
              <Folder className="w-3.5 h-3.5 text-[#8b949e]" />
              <span className="text-xs text-[#8b949e] font-medium">{cwd}</span>
            </div>
            <form onSubmit={handleSubmit} className="flex flex-col space-y-2 relative">
                {showHistory && history.length > 0 && (
                <div
                  ref={historyListRef}
                  className="absolute bottom-full left-0 w-full mb-2 bg-[#161b22] border border-[#30363d] rounded-lg shadow-lg overflow-y-auto max-h-48 z-10 py-1"
                >
                  {history.map((cmd, idx) => (
                    <div
                      key={idx}
                      className={`px-4 py-2 text-sm font-mono cursor-pointer truncate ${idx === historyIndex ? 'bg-[#21262d] text-[#58a6ff]' : 'text-[#c9d1d9] hover:bg-[#21262d]'}`}
                      onClick={() => {
                        setInput(cmd);
                        setShowHistory(false);
                        setHistoryIndex(-1);
                        setTimeout(() => {
                          textareaRef.current?.focus();
                        }, 0);
                      }}
                      onMouseDown={(e) => e.preventDefault()}
                      onMouseEnter={() => setHistoryIndex(idx)}
                    >
                      {cmd}
                    </div>
                  ))}
                </div>
              )}
              {showCompletions && completions.length > 0 && (
                <div
                  ref={completionsListRef}
                  className="absolute bottom-full left-0 w-full mb-2 bg-[#161b22] border border-[#30363d] rounded-lg shadow-lg overflow-y-auto max-h-48 z-10 py-1 flex flex-wrap gap-2 px-2"
                >
                  {completions.map((comp, idx) => (
                    <div
                      key={idx}
                      className={`px-3 py-1 text-sm font-mono cursor-pointer rounded-md ${idx === completionIndex ? 'bg-[#58a6ff] text-[#0d1117] font-bold' : 'text-[#c9d1d9] hover:bg-[#21262d]'}`}
                      onClick={() => applyCompletion(comp)}
                      onMouseDown={(e) => e.preventDefault()}
                      onMouseEnter={() => setCompletionIndex(idx)}
                    >
                      {comp}
                    </div>
                  ))}
                </div>
              )}
              <div className="flex items-center relative">
                <button
                  type="button"
                  onClick={() => setIsAiMode(!isAiMode)}
                  className={`absolute left-3 top-3 z-10 transition-colors ${isAiMode ? 'text-[#bc8cff] hover:text-[#d4b3ff]' : 'text-[#3fb950] hover:text-[#56d364]'}`}
                  title={isAiMode ? 'AI Mode (click for Terminal)' : 'Terminal Mode (click for AI)'}
                >
                  {isAiMode ? <Sparkles className="w-5 h-5" /> : <TerminalSquare className="w-5 h-5" />}
                </button>
                <textarea
                  ref={textareaRef}
                  value={input}
                  onChange={(e) => {
                    setInput(e.target.value);
                    if (showHistory) setShowHistory(false);
                    if (showCompletions) setShowCompletions(false);
                  }}
                  onKeyDown={(e) => {
                    if (e.key === 'Tab') {
                      handleTab(e);
                      return;
                    }

                    if (e.key === 'ArrowUp') {
                      if (showCompletions) {
                        e.preventDefault();
                        setCompletionIndex(prev => (prev - 1 + completions.length) % completions.length);
                      } else if (!showHistory && history.length > 0) {
                        e.preventDefault();
                        setShowHistory(true);
                        setHistoryIndex(history.length - 1);
                      } else if (showHistory) {
                        e.preventDefault();
                        setHistoryIndex(prev => Math.max(prev - 1, 0));
                      }
                    } else if (e.key === 'ArrowDown') {
                      if (showCompletions) {
                        e.preventDefault();
                        setCompletionIndex(prev => (prev + 1) % completions.length);
                      } else if (showHistory) {
                        e.preventDefault();
                        if (historyIndex < history.length - 1) {
                          setHistoryIndex(prev => prev + 1);
                        } else {
                          setShowHistory(false);
                          setHistoryIndex(-1);
                        }
                      }
                    } else if (e.key === 'Enter') {
                      if (e.metaKey || e.ctrlKey) {
                        // Cmd+Enter always sends to AI
                        e.preventDefault();
                        handleAiSubmit(input);
                      } else if (showCompletions && completionIndex >= 0) {
                        e.preventDefault();
                        applyCompletion(completions[completionIndex]);
                      } else if (showHistory && historyIndex >= 0) {
                        e.preventDefault();
                        setInput(history[historyIndex]);
                        setShowHistory(false);
                        setHistoryIndex(-1);
                      } else if (!e.shiftKey) {
                        e.preventDefault();
                        if (isAiMode) {
                          handleAiSubmit(input);
                        } else {
                          handleSubmit(e);
                        }
                      }
                    } else if (e.key === 'Escape') {
                      if (showHistory) {
                        e.preventDefault();
                        setShowHistory(false);
                        setHistoryIndex(-1);
                      }
                      if (showCompletions) {
                        e.preventDefault();
                        setShowCompletions(false);
                        setCompletionIndex(-1);
                      }
                    }
                  }}
                  onFocus={(e) => {
                    const target = e.target;
                    const length = target.value.length;
                    // Defer the selection range update so WKWebView's default focus behavior doesn't override it
                    setTimeout(() => {
                      target.setSelectionRange(length, length);
                    }, 0);
                  }}
                  placeholder={hasRunningCommand ? "Running... (Ctrl+C to stop)" : isAiMode ? "Ask AI anything... (Enter to send)" : "Type a command... (Cmd+Enter for AI)"}
                  disabled={hasRunningCommand}
                  autoCapitalize="none"
                  autoComplete="off"
                  autoCorrect="off"
                  spellCheck={false}
                  className={`w-full bg-[#0d1117] border border-[#30363d] rounded-lg py-3 pl-10 ${hasRunningCommand ? 'pr-12' : 'pr-4'} text-sm font-mono text-[#c9d1d9] focus:outline-none focus:ring-1 focus:ring-[#58a6ff] focus:border-transparent resize-none whitespace-pre ${hasRunningCommand ? 'opacity-50 cursor-not-allowed' : ''}`}
                  rows={Math.min(10, input.split('\n').length || 1)}
                />
                {hasRunningCommand && (
                  <button
                    type="button"
                    onClick={handleStopCommand}
                    className="absolute right-3 top-3 z-10 p-0.5 rounded text-[#f85149] hover:text-[#ff7b72] hover:bg-[#f85149]/10 transition-colors"
                    title="Stop command (Ctrl+C)"
                  >
                    <Square className="w-4 h-4 fill-current" />
                  </button>
                )}
              </div>
            </form>
          </>
        )}
      </div>

      {showToolCallModal && (
        <ToolCallModal
          explanation={showToolCallModal.toolCall.args.explanation || 'No explanation provided'}
          command={showToolCallModal.toolCall.args.command || ''}
          onApprove={handleToolApprove}
          onReject={handleToolReject}
          onClose={() => setShowToolCallModal(null)}
        />
      )}
    </div>
  );
};
