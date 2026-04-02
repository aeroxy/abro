import React, { useState, useEffect } from 'react';
import { Terminal, X, Check } from 'lucide-react';

interface ToolCallModalProps {
  explanation: string;
  command: string;
  onApprove: (editedCommand?: string) => void;
  onReject: () => void;
  onClose: () => void;
}

export const ToolCallModal: React.FC<ToolCallModalProps> = ({
  explanation,
  command,
  onApprove,
  onReject,
  onClose,
}) => {
  const [isEditing, setIsEditing] = useState(false);
  const [editedCommand, setEditedCommand] = useState(command);
  const [mouseDownOnBackdrop, setMouseDownOnBackdrop] = useState(false);

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        handleReject();
      }
      if (e.key === 'Enter' && !isEditing && (e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        handleApprove();
      }
    };

    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [isEditing]);

  const handleApprove = () => {
    onApprove(isEditing ? editedCommand : undefined);
    onClose();
  };

  const handleReject = () => {
    onReject();
    onClose();
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) setMouseDownOnBackdrop(true);
      }}
      onMouseUp={(e) => {
        if (e.target === e.currentTarget && mouseDownOnBackdrop) {
          handleReject();
        }
        setMouseDownOnBackdrop(false);
      }}
    >
      <div className="bg-[#161b22] border border-[#30363d] rounded-xl shadow-2xl w-[520px]">
        {/* Header */}
        <div className="flex items-center justify-between px-5 py-4 border-b border-[#30363d]">
          <div className="flex items-center space-x-2">
            <Terminal className="w-4 h-4 text-[#f0883e]" />
            <span className="text-sm font-medium text-[#c9d1d9]">AI wants to run a command</span>
          </div>
          <button onClick={handleReject} className="p-1 rounded-md text-[#8b949e] hover:text-[#c9d1d9]">
            <X className="w-4 h-4" />
          </button>
        </div>

        {/* Body */}
        <div className="px-5 py-4 space-y-4">
          <div>
            <p className="text-sm text-[#c9d1d9] mb-3">{explanation}</p>
          </div>

          <div>
            <label className="block text-xs text-[#8b949e] mb-1">Command</label>
            {isEditing ? (
              <textarea
                value={editedCommand}
                onChange={(e) => setEditedCommand(e.target.value)}
                className="w-full bg-[#0d1117] border border-[#30363d] rounded-lg px-3 py-2 text-sm font-mono text-[#c9d1d9] focus:outline-none focus:ring-1 focus:ring-[#58a6ff]"
                rows={3}
                autoFocus
              />
            ) : (
              <div className="bg-[#0d1117] border border-[#30363d] rounded-lg px-3 py-2 text-sm font-mono text-[#c9d1d9]">
                {command}
              </div>
            )}
          </div>
        </div>

        {/* Footer */}
        <div className="flex items-center justify-end space-x-2 px-5 py-3 border-t border-[#30363d]">
          <button
            onClick={handleReject}
            className="px-3 py-1.5 text-sm text-[#c9d1d9] border border-[#30363d] rounded-lg hover:bg-[#21262d]"
          >
            Reject
          </button>
          <button
            onClick={() => setIsEditing(!isEditing)}
            className="px-3 py-1.5 text-sm text-[#58a6ff] border border-[#30363d] rounded-lg hover:bg-[#21262d]"
          >
            {isEditing ? 'Cancel Edit' : 'Edit'}
          </button>
          <button
            onClick={handleApprove}
            className="flex items-center space-x-1.5 px-3 py-1.5 text-sm text-white bg-[#238636] border border-[#2ea043] rounded-lg hover:bg-[#2ea043]"
          >
            <Check className="w-3.5 h-3.5" />
            <span>Approve</span>
          </button>
        </div>
      </div>
    </div>
  );
};
