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
  thumb: string | null;
  dims: string | null;
};

export type Settings = {
  deviceId: string;
  deviceName: string;
  autoPaste: boolean;
  maxClips: number;
  maxBytes: number;
  hotkey: string;
  syncEnabled: boolean;
  syncKey: string | null;
  syncDir: string | null;
  blacklist: string[];
};

export type Bootstrap = {
  settings: Settings;
  clips: ClipView[];
  paused: boolean;
  canPaste: boolean;
};

export type PeerStatus = {
  id: string;
  name: string;
  platform: string;
  updatedAt: number;
  received: number;
  online: boolean;
};

export type SyncStatus = {
  enabled: boolean;
  paired: boolean;
  dir: string;
  dirExists: boolean;
  isIcloud: boolean;
  deviceId: string;
  deviceName: string;
  peers: PeerStatus[];
  lastPushAt: number;
  lastPullAt: number;
  lastError: string | null;
  lanPeers: number;
};

export type InviteInfo = {
  code: string;
  expiresAt: number;
  path: string;
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
  setPanelMode: (mode: "clips" | "settings") => invoke<void>("set_panel_mode", { mode }),

  syncStatus: () => invoke<SyncStatus>("sync_status"),
  syncSetDir: (path: string) => invoke<void>("sync_set_dir", { path }),
  syncSetEnabled: (enabled: boolean) => invoke<void>("sync_set_enabled", { enabled }),
  syncCreateInvite: () => invoke<InviteInfo>("sync_create_invite"),
  syncJoin: (code: string) => invoke<string>("sync_join", { code }),
  syncNow: () => invoke<void>("sync_now"),
  syncReset: () => invoke<void>("sync_reset"),
};
