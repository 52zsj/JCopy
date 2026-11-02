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
  /** 外观主题：system / light / dark */
  theme: string;
  /** 字号档位：small / medium / large */
  fontSize: string;
  /** 图片同步策略：off / pinned / all */
  imageSync: string;
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

export type PeerFile = {
  id: string;
  size: number;
  updatedAt: number;
  isSelf: boolean;
  decryptable: boolean;
  clips: number;
  /** 其中图片张数 */
  images: number;
  /** 有图但原图文件还没到云盘 */
  missingImages: number;
  /** 解不开且久未更新：上一次配对留下的旧文件，可忽略 */
  stale: boolean;
  error: string | null;
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
  /** 主密钥指纹：两台机器一致才可能互相解密 */
  keyFingerprint: string | null;
  /** 本机发件箱里实际发布了多少条 */
  ownClips: number;
  /** 最近一次发出去：总条数 / 其中图片张数 */
  pushedClips: number;
  pushedImages: number;
  /** 本次启动以来累计收到：总条数 / 其中图片张数 */
  pulledClips: number;
  pulledImages: number;
  /** 有图片条目但原图还没落盘，正常为 0 */
  pendingImages: number;
  peerFiles: PeerFile[];
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
  revealClip: (id: string) => invoke<void>("reveal_clip", { id }),
  updateSettings: (settings: Settings) => invoke<Settings>("update_settings", { settings }),
  setPaused: (paused: boolean) => invoke<boolean>("set_paused", { paused }),
  hidePanel: () => invoke<void>("hide_panel"),
  openAccessibility: () => invoke<void>("open_accessibility_settings"),
  setPanelMode: (mode: "clips" | "settings") => invoke<void>("set_panel_mode", { mode }),

  // deep=false 只读运行时计数，适合每几秒轮询；deep=true 会逐个发件箱解密扫描
  syncStatus: (deep = false) => invoke<SyncStatus>("sync_status", { deep }),
  syncSetDir: (path: string) => invoke<void>("sync_set_dir", { path }),
  syncSetEnabled: (enabled: boolean) => invoke<void>("sync_set_enabled", { enabled }),
  syncCreateInvite: () => invoke<InviteInfo>("sync_create_invite"),
  syncJoin: (code: string) => invoke<string>("sync_join", { code }),
  syncNow: () => invoke<void>("sync_now"),
  syncReset: () => invoke<void>("sync_reset"),
};
