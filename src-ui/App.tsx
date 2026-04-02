import React, { useState, useEffect, useRef } from 'react';
import { TerminalSquare, Plus, X, Settings } from 'lucide-react';
import { TerminalSession } from './TerminalSession';
import { SettingsModal } from './SettingsModal';
import { getCurrentWindow, PhysicalPosition } from '@tauri-apps/api/window';

interface Tab {
  id: string;
  cwd: string;
}

export const App = () => {
  const [tabs, setTabs] = useState<Tab[]>([{ id: Date.now().toString(), cwd: '~' }]);
  const [activeTabId, setActiveTabId] = useState<string>(tabs[0].id);
  const [showSettings, setShowSettings] = useState(false);
  const isDragging = useRef(false);
  const lastCursorPos = useRef({ x: 0, y: 0 });

  useEffect(() => {
    const handleDragStart = (e: MouseEvent) => {
      const target = e.target as HTMLElement;
      if (target.closest('.no-drag')) return;

      if (target.closest('[data-app-drag-region]') && e.button === 0) {
        e.preventDefault();
        e.stopPropagation();

        // Blur any focused element BEFORE starting drag to prevent focus conflict
        if (document.activeElement instanceof HTMLElement) {
          document.activeElement.blur();
        }

        isDragging.current = true;
        lastCursorPos.current = { x: e.screenX, y: e.screenY };
        console.log('[drag] start at', e.screenX, e.screenY);
      }
    };

    const handleDragMove = async (e: MouseEvent) => {
      if (!isDragging.current) return;

      const dx = e.screenX - lastCursorPos.current.x;
      const dy = e.screenY - lastCursorPos.current.y;

      if (dx === 0 && dy === 0) return;

      lastCursorPos.current = { x: e.screenX, y: e.screenY };

      try {
        const appWindow = getCurrentWindow();
        const currentPos = await appWindow.outerPosition();
        const newX = Math.round(currentPos.x + dx);
        const newY = Math.round(currentPos.y + dy);
        console.log('[drag] move dx:', dx, 'dy:', dy, 'to:', newX, newY, 'from:', currentPos.x, currentPos.y);
        await appWindow.setPosition(new PhysicalPosition(newX, newY));
      } catch (err) {
        console.error('[drag] setPosition error:', err);
      }
    };

    const handleDragEnd = () => {
      if (isDragging.current) console.log('[drag] end');
      isDragging.current = false;
    };

    document.addEventListener('mousedown', handleDragStart, { capture: true });
    document.addEventListener('mousemove', handleDragMove);
    document.addEventListener('mouseup', handleDragEnd);
    
    const handleKeyDown = (e: KeyboardEvent) => {
      // Cmd+T on Mac, Ctrl+T on Windows/Linux
      if ((e.metaKey || e.ctrlKey) && e.key === 't') {
        e.preventDefault();
        const newId = Date.now().toString();
        // Fallback cwd to ~
        setTabs(prev => [...prev, { id: newId, cwd: '~' }]);
        setActiveTabId(newId);
      }
      
      // Cmd+, to open settings
      if ((e.metaKey || e.ctrlKey) && e.key === ',') {
        e.preventDefault();
        setShowSettings(prev => !prev);
      }

      // Cmd+W to close active tab
      if ((e.metaKey || e.ctrlKey) && e.key === 'w') {
        e.preventDefault();
        if (tabs.length === 1) {
          getCurrentWindow().close();
          return;
        }
        setTabs(prev => {
          const newTabs = prev.filter(t => t.id !== activeTabId);
          setActiveTabId(newTabs[newTabs.length - 1].id);
          return newTabs;
        });
      }
    };

    window.addEventListener('keydown', handleKeyDown);
    return () => {
      window.removeEventListener('keydown', handleKeyDown);
      document.removeEventListener('mousedown', handleDragStart, { capture: true });
      document.removeEventListener('mousemove', handleDragMove);
      document.removeEventListener('mouseup', handleDragEnd);
    };
  }, [activeTabId]);

  const handleCloseTab = (e: React.MouseEvent, tabId: string) => {
    e.stopPropagation();
    
    if (tabs.length === 1) {
      getCurrentWindow().close();
      return;
    }

    const newTabs = tabs.filter(t => t.id !== tabId);
    setTabs(newTabs);
    
    // If we closed the active tab, switch to the last available tab
    if (activeTabId === tabId) {
      setActiveTabId(newTabs[newTabs.length - 1].id);
    }
  };

  const handleNewTab = () => {
    const newId = Date.now().toString();
    setTabs(prev => [...prev, { id: newId, cwd: '~' }]);
    setActiveTabId(newId);
  };

  return (
    <div className="flex flex-col h-screen w-screen bg-[#0d1117] text-[#c9d1d9] font-sans">
      {/* Tab bar (drag region) */}
      <div 
        data-app-drag-region
        className="h-[38px] bg-[#161b22] border-b border-[#30363d] flex items-center pr-2 pl-20 shrink-0 select-none space-x-1"
      >
        {tabs.map((tab) => {
          const isActive = activeTabId === tab.id;
          // Extract just the folder name for the tab title, e.g., 'abro' from '~/Documents/abro'
          const folderName = tab.cwd === '~' ? '~' : tab.cwd.split('/').filter(Boolean).pop() || '~';

          return (
            <div
              key={tab.id}
              onClick={() => setActiveTabId(tab.id)}
              className={`group flex items-center space-x-2 px-3 py-1 rounded-md border cursor-pointer text-[13px] no-drag
                ${isActive
                  ? 'bg-[#0d1117] border-[#30363d] text-[#c9d1d9] shadow-sm'
                  : 'bg-transparent border-transparent text-[#8b949e] hover:bg-[#21262d]'}`}
            >
              <TerminalSquare className="w-3.5 h-3.5" />
              <span className="max-w-[120px] truncate">{folderName}</span>
              
              <div 
                className={`p-0.5 rounded-md hover:bg-[#30363d] hover:text-[#c9d1d9] ${isActive ? 'text-[#8b949e]' : 'text-transparent group-hover:text-[#8b949e]'}`}
                onClick={(e) => handleCloseTab(e, tab.id)}
              >
                <X className="w-3.5 h-3.5" />
              </div>
            </div>
          );
        })}
        
        {/* New Tab Button */}
        <div
          onClick={handleNewTab}
          className="p-1.5 ml-1 rounded-md text-[#8b949e] hover:bg-[#21262d] hover:text-[#c9d1d9] cursor-pointer no-drag"
          title="New Tab (Cmd+T)"
        >
          <Plus className="w-3.5 h-3.5" />
        </div>

        {/* Empty space drag region */}
        <div data-app-drag-region className="flex-1 h-full" />

        {/* Settings Button */}
        <div
          onClick={() => setShowSettings(true)}
          className="p-1.5 rounded-md text-[#8b949e] hover:bg-[#21262d] hover:text-[#c9d1d9] cursor-pointer no-drag"
          title="Settings (Cmd+,)"
        >
          <Settings className="w-3.5 h-3.5" />
        </div>
      </div>

      {/* Main Terminal View */}
      <div className="flex-1 flex flex-col relative overflow-hidden">
        {tabs.map(tab => (
          <TerminalSession
            key={tab.id}
            isActive={tab.id === activeTabId}
            onCwdChange={(newCwd) => {
              setTabs(prev => prev.map(t => (t.id === tab.id ? { ...t, cwd: newCwd } : t)));
            }}
            onSessionEnd={() => handleCloseTab({ stopPropagation: () => {} } as React.MouseEvent, tab.id)}
          />
        ))}
      </div>

      {showSettings && <SettingsModal onClose={() => setShowSettings(false)} />}
    </div>
  );
};
