import { invoke } from "@tauri-apps/api/core";

export type ClipView = {
  id: string;
  kind: string;
  preview: string;
  sourceApp: string | null;
  device: string;
  createdAt: number;
  lastUsedAt: number;
  pinned: boolean;
  bytes: number;
};

export type Settings = {
  deviceId: string;
  deviceName: string;
  autoPaste: boolean;
  maxClips: number;
  maxBytes: number;
  hotkey: string;
  syncEnabled: boolean;
  blacklist: string[];
};

export type Bootstrap = {
  settings: Settings;
  clips: ClipView[];
  paused: boolean;
  canPaste: boolean;
};

export const api = {
  bootstrap: () => invoke<Bootstrap>("bootstrap"),
  copyClip: (id: string) => invoke<void>("copy_clip", { id }),
  deleteClip: (id: string) => invoke<void>("delete_clip", { id }),
  clearAll: () => invoke<void>("clear_all"),
  togglePin: (id: string) => invoke<boolean>("toggle_pin", { id }),
  updateSettings: (settings: Settings) => invoke<Settings>("update_settings", { settings }),
  setPaused: (paused: boolean) => invoke<boolean>("set_paused", { paused }),
  hidePanel: () => invoke<void>("hide_panel"),
  openAccessibility: () => invoke<void>("open_accessibility_settings"),
};
