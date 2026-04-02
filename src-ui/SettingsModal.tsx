import React, { useState, useEffect } from 'react';
import { X, Settings, Save, RefreshCw } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';

interface AiProviderConfig {
  provider: string;
  project_id: string;
  location: string;
  model: string;
  credentials_path: string | null;
  api_key: string | null;
}

interface AbroConfig {
  ai: AiProviderConfig | null;
}

interface SettingsModalProps {
  onClose: () => void;
}

export const SettingsModal: React.FC<SettingsModalProps> = ({ onClose }) => {
  const [provider, setProvider] = useState('vertex');
  const [projectId, setProjectId] = useState('');
  const [location, setLocation] = useState('asia-southeast1');
  const [model, setModel] = useState('gemini-2.5-flash');
  const [credentialsPath, setCredentialsPath] = useState('');
  const [apiKey, setApiKey] = useState('');
  const [saving, setSaving] = useState(false);
  const [status, setStatus] = useState<string | null>(null);
  const [availableModels, setAvailableModels] = useState<string[]>([]);
  const [fetchingModels, setFetchingModels] = useState(false);
  const [mouseDownOnBackdrop, setMouseDownOnBackdrop] = useState(false);

  useEffect(() => {
    invoke<AbroConfig>('get_config')
      .then((config) => {
        if (config.ai) {
          setProvider(config.ai.provider || 'vertex');
          setProjectId(config.ai.project_id || '');
          setLocation(config.ai.location || 'asia-southeast1');
          setModel(config.ai.model || 'gemini-2.5-flash');
          setCredentialsPath(config.ai.credentials_path || '');
          setApiKey(config.ai.api_key || '');
        }
      })
      .catch(console.error);
  }, []);

  const handleProviderChange = (newProvider: string) => {
    setProvider(newProvider);
    setAvailableModels([]);
    // Reset model to a sensible default for each provider
    if (newProvider === 'openrouter') {
      setModel('openai/gpt-4o-mini');
    } else {
      setModel('gemini-2.5-flash');
    }
  };

  const handleFetchModels = async () => {
    if (provider === 'openrouter') {
      if (!apiKey.trim()) {
        setStatus('Error: API key required');
        return;
      }
    } else {
      if (!projectId.trim() || !location.trim()) {
        setStatus('Error: Project ID and Location required');
        return;
      }
    }

    setFetchingModels(true);
    setStatus(null);
    try {
      const models = provider === 'openrouter'
        ? await invoke<string[]>('list_openrouter_models', { apiKey })
        : await invoke<string[]>('list_vertex_models', {
            projectId,
            location,
            credentialsPath: credentialsPath || null,
          });
      setAvailableModels(models);
      if (models.length > 0 && !model) {
        setModel(models[0]);
      }
      setStatus(`Found ${models.length} models`);
      setTimeout(() => setStatus(null), 2000);
    } catch (e: any) {
      setStatus('Error: ' + (e?.toString() || 'unknown'));
      setAvailableModels([]);
    } finally {
      setFetchingModels(false);
    }
  };

  const handleSave = async () => {
    setSaving(true);
    setStatus(null);
    try {
      const config: AbroConfig = {
        ai: {
          provider,
          project_id: projectId,
          location,
          model,
          credentials_path: credentialsPath || null,
          api_key: apiKey || null,
        },
      };
      await invoke('save_config', { config });
      setStatus('Saved');
      setTimeout(() => onClose(), 600);
    } catch (e: any) {
      setStatus('Error: ' + (e?.toString() || 'unknown'));
    } finally {
      setSaving(false);
    }
  };

  const isSaveDisabled =
    saving ||
    (provider === 'vertex' && !projectId.trim()) ||
    (provider === 'openrouter' && !apiKey.trim());

  const isOpenRouter = provider === 'openrouter';

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) {
          setMouseDownOnBackdrop(true);
        }
      }}
      onMouseUp={(e) => {
        if (e.target === e.currentTarget && mouseDownOnBackdrop) {
          onClose();
        }
        setMouseDownOnBackdrop(false);
      }}
    >
      <div
        className="bg-[#161b22] border border-[#30363d] rounded-xl shadow-2xl w-[480px] max-h-[90vh] overflow-y-auto"
        onClick={(e) => e.stopPropagation()}
      >
        {/* Header */}
        <div className="flex items-center justify-between px-5 py-4 border-b border-[#30363d]">
          <div className="flex items-center space-x-2">
            <Settings className="w-4 h-4 text-[#8b949e]" />
            <span className="text-sm font-medium text-[#c9d1d9]">Settings</span>
          </div>
          <button
            onClick={onClose}
            className="p-1 rounded-md text-[#8b949e] hover:text-[#c9d1d9] hover:bg-[#21262d] transition-colors"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        {/* Body */}
        <div className="px-5 py-4 space-y-4">
          <div className="text-xs font-medium text-[#8b949e] uppercase tracking-wider">AI Provider</div>

          {/* Provider */}
          <div>
            <label className="block text-xs text-[#8b949e] mb-1">Provider</label>
            <select
              value={provider}
              onChange={(e) => handleProviderChange(e.target.value)}
              className="w-full bg-[#0d1117] border border-[#30363d] rounded-lg px-3 py-2 text-sm text-[#c9d1d9] focus:outline-none focus:ring-1 focus:ring-[#58a6ff]"
            >
              <option value="vertex">Vertex AI (Google)</option>
              <option value="openrouter">OpenRouter</option>
            </select>
          </div>

          {/* OpenRouter: API Key */}
          {isOpenRouter && (
            <div>
              <label className="block text-xs text-[#8b949e] mb-1">API Key</label>
              <input
                type="password"
                value={apiKey}
                onChange={(e) => setApiKey(e.target.value)}
                placeholder="sk-or-..."
                autoComplete="off"
                autoCorrect="off"
                autoCapitalize="off"
                spellCheck={false}
                className="w-full bg-[#0d1117] border border-[#30363d] rounded-lg px-3 py-2 text-sm font-mono text-[#c9d1d9] focus:outline-none focus:ring-1 focus:ring-[#58a6ff] placeholder:text-[#484f58]"
              />
              <p className="mt-1 text-xs text-[#484f58]">Your OpenRouter API key from openrouter.ai/keys</p>
            </div>
          )}

          {/* Vertex: Project ID */}
          {!isOpenRouter && (
            <div>
              <label className="block text-xs text-[#8b949e] mb-1">Project ID</label>
              <input
                type="text"
                value={projectId}
                onChange={(e) => setProjectId(e.target.value)}
                placeholder="my-gcp-project"
                autoComplete="off"
                autoCorrect="off"
                autoCapitalize="off"
                spellCheck={false}
                className="w-full bg-[#0d1117] border border-[#30363d] rounded-lg px-3 py-2 text-sm font-mono text-[#c9d1d9] focus:outline-none focus:ring-1 focus:ring-[#58a6ff] placeholder:text-[#484f58]"
              />
            </div>
          )}

          {/* Vertex: Location */}
          {!isOpenRouter && (
            <div>
              <label className="block text-xs text-[#8b949e] mb-1">Location</label>
              <input
                type="text"
                value={location}
                onChange={(e) => setLocation(e.target.value)}
                placeholder="asia-southeast1"
                autoComplete="off"
                autoCorrect="off"
                autoCapitalize="off"
                spellCheck={false}
                className="w-full bg-[#0d1117] border border-[#30363d] rounded-lg px-3 py-2 text-sm font-mono text-[#c9d1d9] focus:outline-none focus:ring-1 focus:ring-[#58a6ff] placeholder:text-[#484f58]"
              />
            </div>
          )}

          {/* Model */}
          <div>
            <div className="flex items-center justify-between mb-1">
              <label className="block text-xs text-[#8b949e]">Model</label>
              <button
                type="button"
                onClick={handleFetchModels}
                disabled={
                  fetchingModels ||
                  (isOpenRouter ? !apiKey.trim() : !projectId.trim() || !location.trim())
                }
                className="flex items-center space-x-1 px-2 py-0.5 text-xs text-[#58a6ff] hover:text-[#79c0ff] disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
              >
                <RefreshCw className={`w-3 h-3 ${fetchingModels ? 'animate-spin' : ''}`} />
                <span>{fetchingModels ? 'Fetching...' : 'Fetch Models'}</span>
              </button>
            </div>
            {availableModels.length > 0 ? (
              <select
                value={model}
                onChange={(e) => setModel(e.target.value)}
                className="w-full bg-[#0d1117] border border-[#30363d] rounded-lg px-3 py-2 text-sm font-mono text-[#c9d1d9] focus:outline-none focus:ring-1 focus:ring-[#58a6ff]"
              >
                {availableModels.map((m) => (
                  <option key={m} value={m}>
                    {m}
                  </option>
                ))}
              </select>
            ) : (
              <input
                type="text"
                value={model}
                onChange={(e) => setModel(e.target.value)}
                placeholder={isOpenRouter ? 'openai/gpt-4o-mini' : 'gemini-2.5-flash'}
                autoComplete="off"
                autoCorrect="off"
                autoCapitalize="off"
                spellCheck={false}
                className="w-full bg-[#0d1117] border border-[#30363d] rounded-lg px-3 py-2 text-sm font-mono text-[#c9d1d9] focus:outline-none focus:ring-1 focus:ring-[#58a6ff] placeholder:text-[#484f58]"
              />
            )}
          </div>

          {/* Vertex: Credentials Path */}
          {!isOpenRouter && (
            <div>
              <label className="block text-xs text-[#8b949e] mb-1">Credentials File (optional)</label>
              <input
                type="text"
                value={credentialsPath}
                onChange={(e) => setCredentialsPath(e.target.value)}
                placeholder="~/.config/gcloud/application_default_credentials.json"
                autoComplete="off"
                autoCorrect="off"
                autoCapitalize="off"
                spellCheck={false}
                className="w-full bg-[#0d1117] border border-[#30363d] rounded-lg px-3 py-2 text-sm font-mono text-[#c9d1d9] focus:outline-none focus:ring-1 focus:ring-[#58a6ff] placeholder:text-[#484f58]"
              />
              <p className="mt-1 text-xs text-[#484f58]">Leave empty to use default gcloud credentials</p>
            </div>
          )}
        </div>

        {/* Footer */}
        <div className="border-t border-[#30363d]">
          {status && (
            <div className="px-5 py-2 border-b border-[#30363d]">
              <p className={`text-xs break-words ${status.startsWith('Error') ? 'text-[#f85149]' : 'text-[#3fb950]'}`}>
                {status}
              </p>
            </div>
          )}
          <div className="flex items-center justify-end space-x-2 px-5 py-3">
            <button
              onClick={onClose}
              className="px-3 py-1.5 text-sm text-[#c9d1d9] border border-[#30363d] rounded-lg hover:bg-[#21262d] transition-colors"
            >
              Cancel
            </button>
            <button
              onClick={handleSave}
              disabled={isSaveDisabled}
              className="flex items-center space-x-1.5 px-3 py-1.5 text-sm text-white bg-[#238636] border border-[#2ea043] rounded-lg hover:bg-[#2ea043] transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
            >
              <Save className="w-3.5 h-3.5" />
              <span>{saving ? 'Saving...' : 'Save'}</span>
            </button>
          </div>
        </div>
      </div>
    </div>
  );
};
