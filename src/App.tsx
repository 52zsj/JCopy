import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, type ClipView, type InviteInfo, type Settings, type SyncStatus } from "./api";
import { appLabel, humanBytes, platformName, relativeTime } from "./format";

export default function App() {
  const [clips, setClips] = useState<ClipView[]>([]);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [paused, setPaused] = useState(false);
  const [canPaste, setCanPaste] = useState(false);
  const [query, setQuery] = useState("");
  const [pinnedOnly, setPinnedOnly] = useState(false);
  const [active, setActive] = useState(0);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [toast, setToast] = useState<string | null>(null);
  const [sync, setSync] = useState<SyncStatus | null>(null);
  const [invite, setInvite] = useState<InviteInfo | null>(null);
  const [joinOpen, setJoinOpen] = useState(false);
  const [joinCode, setJoinCode] = useState("");
  const [dirOpen, setDirOpen] = useState(false);
  const [dirPath, setDirPath] = useState("");

  const inputRef = useRef<HTMLInputElement>(null);
  const activeRef = useRef<HTMLElement>(null);
  const railRef = useRef<HTMLDivElement>(null);

  const refresh = useCallback(async () => {
    const data = await api.bootstrap();
    setClips(data.clips);
    setSettings(data.settings);
    setPaused(data.paused);
    setCanPaste(data.canPaste);
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  useEffect(() => {
    const unlisteners: Array<() => void> = [];
    let disposed = false;

    void (async () => {
      const added = await listen<ClipView>("clip-added", (event) => {
        setClips((prev) => [event.payload, ...prev.filter((c) => c.id !== event.payload.id)]);
      });
      const touched = await listen<string>("clip-touched", (event) => {
        setClips((prev) => {
          const target = prev.find((c) => c.id === event.payload);
          if (!target) return prev;
          return [target, ...prev.filter((c) => c.id !== event.payload)];
        });
      });
      const shown = await listen("panel-shown", () => {
        setQuery("");
        setPinnedOnly(false);
        setActive(0);
        setSettingsOpen(false);
        setToast(null);
        void api.syncStatus().then(setSync).catch(() => {});
        requestAnimationFrame(() => inputRef.current?.focus());
      });

      if (disposed) {
        added();
        touched();
        shown();
      } else {
        unlisteners.push(added, touched, shown);
      }
    })();

    return () => {
      disposed = true;
      unlisteners.forEach((fn) => fn());
    };
  }, []);

  useEffect(() => {
    if (!toast) return;
    const timer = setTimeout(() => setToast(null), 3200);
    return () => clearTimeout(timer);
  }, [toast]);

  // 剪贴板是扁条、设置页要更高，切换时让 Rust 侧重排窗口
  useEffect(() => {
    void api.setPanelMode(settingsOpen ? "settings" : "clips");
  }, [settingsOpen]);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    return clips.filter((clip) => {
      if (pinnedOnly && !clip.pinned) return false;
      if (!q) return true;
      if (clip.preview.toLowerCase().includes(q)) return true;
      return appLabel(clip.sourceApp).toLowerCase().includes(q);
    });
  }, [clips, query, pinnedOnly]);

  useEffect(() => {
    setActive((index) => Math.min(index, Math.max(0, filtered.length - 1)));
  }, [filtered.length]);

  useEffect(() => {
    activeRef.current?.scrollIntoView({ block: "nearest", inline: "nearest" });
  }, [active]);

  // 竖向滚轮转成横向滚动：卡片条是横的，用户不会去拖滚动条
  useEffect(() => {
    if (settingsOpen) return;
    const el = railRef.current;
    if (!el) return;
    const onWheel = (event: WheelEvent) => {
      if (Math.abs(event.deltaY) <= Math.abs(event.deltaX)) return;
      event.preventDefault();
      el.scrollLeft += event.deltaY;
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, [settingsOpen, filtered.length]);

  const paste = useCallback(async (id: string) => {
    try {
      await api.copyClip(id);
    } catch (error) {
      setToast(typeof error === "string" ? error : "粘贴失败");
    }
  }, []);

  const togglePin = useCallback(async (id: string) => {
    const pinned = await api.togglePin(id);
    setClips((prev) => prev.map((c) => (c.id === id ? { ...c, pinned } : c)));
  }, []);

  const removeClip = useCallback(async (id: string) => {
    await api.deleteClip(id);
    setClips((prev) => prev.filter((c) => c.id !== id));
  }, []);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.isComposing) return;
      const mod = event.metaKey || event.ctrlKey;

      if (event.key === "Escape") {
        event.preventDefault();
        if (settingsOpen) {
          setSettingsOpen(false);
          requestAnimationFrame(() => inputRef.current?.focus());
        } else {
          void api.hidePanel();
        }
        return;
      }

      if (settingsOpen) return;

      // 卡片是横向的，左右为主；上下也映射过去，免得肌肉记忆落空
      if (event.key === "ArrowRight" || event.key === "ArrowDown") {
        event.preventDefault();
        setActive((i) => Math.min(i + 1, filtered.length - 1));
        return;
      }
      if (event.key === "ArrowLeft" || event.key === "ArrowUp") {
        event.preventDefault();
        setActive((i) => Math.max(i - 1, 0));
        return;
      }
      if (event.key === "Enter") {
        event.preventDefault();
        const target = filtered[active];
        if (target) void paste(target.id);
        return;
      }

      const current = filtered[active];
      if (!current) return;

      if (mod && /^[1-9]$/.test(event.key)) {
        event.preventDefault();
        const target = filtered[Number(event.key) - 1];
        if (target) void paste(target.id);
        return;
      }
      if (mod && event.key.toLowerCase() === "p") {
        event.preventDefault();
        void togglePin(current.id);
        return;
      }
      if (mod && (event.key === "Backspace" || event.key === "Delete")) {
        event.preventDefault();
        void removeClip(current.id);
      }
    };

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [active, filtered, paste, removeClip, settingsOpen, togglePin]);

  // 设置页打开时才轮询同步状态，关闭即停
  useEffect(() => {
    if (!settingsOpen) return;
    let alive = true;
    const load = async () => {
      try {
        const next = await api.syncStatus();
        if (alive) setSync(next);
      } catch {
        // 同步状态读不到不影响本机使用
      }
    };
    void load();
    const timer = setInterval(load, 3000);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, [settingsOpen]);

  const createInvite = useCallback(async () => {
    try {
      setInvite(await api.syncCreateInvite());
    } catch (error) {
      setToast(typeof error === "string" ? error : "生成配对码失败");
    }
  }, []);

  const joinWithCode = useCallback(async () => {
    try {
      const name = await api.syncJoin(joinCode);
      setJoinOpen(false);
      setJoinCode("");
      setInvite(null);
      setToast(`已连接到 ${name}`);
      setSync(await api.syncStatus());
    } catch (error) {
      setToast(typeof error === "string" ? error : "配对失败");
    }
  }, [joinCode]);

  const runSyncNow = useCallback(async () => {
    try {
      await api.syncNow();
      setSync(await api.syncStatus());
      setToast("已同步");
    } catch (error) {
      setToast(typeof error === "string" ? error : "同步失败");
    }
  }, []);

  const resetSync = useCallback(async () => {
    try {
      await api.syncReset();
      setSync(await api.syncStatus());
      setToast("已解除配对");
    } catch (error) {
      setToast(typeof error === "string" ? error : "操作失败");
    }
  }, []);

  const applyDir = useCallback(async () => {
    try {
      await api.syncSetDir(dirPath.trim());
      setSync(await api.syncStatus());
      setDirOpen(false);
      setToast("同步目录已更新");
    } catch (error) {
      setToast(typeof error === "string" ? error : "目录设置失败");
    }
  }, [dirPath]);

  const patchSettings = useCallback(
    async (patch: Partial<Settings>) => {
      if (!settings) return;
      const next = { ...settings, ...patch };
      setSettings(next);
      await api.updateSettings(next);
    },
    [settings],
  );

  const toggleAutoPaste = useCallback(async () => {
    if (!settings) return;
    if (!settings.autoPaste && !canPaste) {
      setToast("需要先授予「辅助功能」权限");
      await api.openAccessibility();
      return;
    }
    await patchSettings({ autoPaste: !settings.autoPaste });
  }, [canPaste, patchSettings, settings]);

  const togglePaused = useCallback(async () => {
    const next = await api.setPaused(!paused);
    setPaused(next);
  }, [paused]);

  const deviceName = settings?.deviceName ?? "";
  const isMac = navigator.userAgent.includes("Mac");

  return (
    <div className="app">
      <div className="surface">
        <div className="grabber" aria-hidden="true" />

        {settingsOpen ? (
          <div className="settings">
            <div className="settings-head">
              <span>设置</span>
              <button
                className="ghost"
                onClick={() => {
                  setSettingsOpen(false);
                  requestAnimationFrame(() => inputRef.current?.focus());
                }}
              >
                完成
              </button>
            </div>

            <div className="settings-body">
              <label className="field">
                <span className="field-label">设备名称</span>
                <input
                  value={deviceName}
                  onChange={(e) => void patchSettings({ deviceName: e.target.value })}
                  placeholder="这台设备的名字"
                />
              </label>

              <button className="row" onClick={() => void toggleAutoPaste()}>
                <span>
                  自动粘贴
                  <em>{canPaste ? "选中后直接送入前台应用" : "需授予系统辅助功能权限"}</em>
                </span>
                <Switch on={settings?.autoPaste ?? false} />
              </button>

              <button className="row" onClick={() => void togglePaused()}>
                <span>
                  暂停监听
                  <em>{paused ? "当前不会记录任何复制内容" : "正在记录剪贴板变化"}</em>
                </span>
                <Switch on={paused} />
              </button>

              <div className="row static">
                <span>
                  历史规模
                  <em>
                    {clips.length} 条 · 上限 {settings?.maxClips ?? 0} 条 /{" "}
                    {humanBytes(settings?.maxBytes ?? 0)}
                  </em>
                </span>
              </div>

              <div className="row static">
                <span>
                  唤起快捷键
                  <em>
                    {settings?.hotkey === "CommandOrControl+Shift+V"
                      ? isMac
                        ? "⌘ ⇧ V"
                        : "Ctrl + Shift + V"
                      : (settings?.hotkey ?? "")}
                  </em>
                </span>
              </div>

              <button
                className="row danger"
                onClick={async () => {
                  await api.clearAll();
                  setClips([]);
                  setToast("已清空本机历史");
                }}
              >
                <span>
                  清空历史
                  <em>仅清除本机，不影响其他设备</em>
                </span>
              </button>

              <div className="group-title">跨设备同步</div>

              {invite && (
                <div className="invite-card">
                  <div className="invite-code">{invite.code}</div>
                  <div className="invite-hint">
                    在另一台设备的 JCopy 里选「输入配对码」，10 分钟内有效
                  </div>
                </div>
              )}

              {joinOpen && (
                <div className="join-row">
                  <input
                    value={joinCode}
                    onChange={(e) => setJoinCode(e.target.value.replace(/\D/g, "").slice(0, 6))}
                    placeholder="输入 6 位配对码"
                    inputMode="numeric"
                    autoFocus
                  />
                  <button
                    className="ghost"
                    onClick={() => void joinWithCode()}
                    disabled={joinCode.length !== 6}
                  >
                    连接
                  </button>
                </div>
              )}

              {sync?.peers.map((peer) => (
                <div className="row static" key={peer.id}>
                  <span>
                    {peer.name}
                    <em>
                      {platformName(peer.platform)} ·{" "}
                      {peer.updatedAt ? relativeTime(peer.updatedAt) : "等待首轮同步"}
                    </em>
                  </span>
                  <span className="peer-dot" />
                </div>
              ))}

              {!sync?.paired ? (
                <>
                  <button className="row" onClick={() => void createInvite()}>
                    <span>
                      生成配对码
                      <em>让另一台设备输入这个码</em>
                    </span>
                    <span className="chev">→</span>
                  </button>
                  <button
                    className="row"
                    onClick={() => {
                      setJoinOpen((v) => !v);
                      setInvite(null);
                    }}
                  >
                    <span>
                      输入配对码
                      <em>已有设备生成了码？在这里输入</em>
                    </span>
                    <span className="chev">→</span>
                  </button>
                </>
              ) : (
                <>
                  <button className="row" onClick={() => void runSyncNow()}>
                    <span>
                      立即同步
                      <em>
                        {sync.lastPullAt
                          ? `上次同步 ${relativeTime(sync.lastPullAt)}`
                          : "尚未同步过"}
                      </em>
                    </span>
                  </button>
                  <button className="row danger" onClick={() => void resetSync()}>
                    <span>
                      解除配对
                      <em>清除本机同步密钥，已复制的历史不受影响</em>
                    </span>
                  </button>
                </>
              )}

              <button
                className="row"
                onClick={() => {
                  if (!dirOpen) setDirPath(sync?.dir ?? "");
                  setDirOpen((v) => !v);
                }}
              >
                <span>
                  同步目录
                  <em className={sync?.dirExists === false ? "bad" : ""}>
                    {sync?.dirExists === false
                      ? "目录不可用，请手动指定"
                      : `${sync?.isIcloud ? "iCloud 云盘" : "自定义"} · ${sync?.dir ?? "检测中…"}`}
                  </em>
                </span>
                <span className="chev">{dirOpen ? "收起" : "更改"}</span>
              </button>

              {dirOpen && (
                <div className="join-row">
                  <input
                    value={dirPath}
                    onChange={(e) => setDirPath(e.target.value)}
                    placeholder="粘贴云盘目录的绝对路径"
                    spellCheck={false}
                  />
                  <button
                    className="ghost"
                    onClick={() => void applyDir()}
                    disabled={!dirPath.trim()}
                  >
                    保存
                  </button>
                </div>
              )}

              {sync?.lastError && (
                <div className="sync-dir">
                  <span className="bad">{sync.lastError}</span>
                </div>
              )}

              <div className="row static">
                <span>
                  局域网直连
                  <em>
                    {sync?.lanPeers
                      ? `已发现 ${sync.lanPeers} 台同网段设备，优先走直连`
                      : "未发现同网段设备，仍会走 iCloud"}
                  </em>
                </span>
              </div>

              <button className="row" onClick={() => void refresh()}>
                <span>
                  重新检测权限
                  <em>{canPaste ? "辅助功能已授权" : "尚未授权"}</em>
                </span>
              </button>
            </div>
          </div>
        ) : (
          <>
            <div className="top-bar">
              <div className="top-side">
                <span className="brand">JCopy</span>
                <span className="count">{filtered.length}</span>
                {paused && <span className="paused-badge">已暂停</span>}
              </div>

              <div className="search-box">
                <svg className="icon-search" viewBox="0 0 16 16" aria-hidden="true">
                  <circle cx="7" cy="7" r="4.6" fill="none" stroke="currentColor" strokeWidth="1.4" />
                  <path
                    d="M10.4 10.4L13.6 13.6"
                    stroke="currentColor"
                    strokeWidth="1.4"
                    strokeLinecap="round"
                  />
                </svg>
                <input
                  ref={inputRef}
                  value={query}
                  onChange={(e) => {
                    setQuery(e.target.value);
                    setActive(0);
                  }}
                  placeholder="搜索剪贴板"
                  spellCheck={false}
                  autoComplete="off"
                />
                {query && (
                  <button className="clear" onClick={() => setQuery("")} title="清除">
                    ×
                  </button>
                )}
              </div>

              <div className="top-side right">
                <button
                  className={`chip ${pinnedOnly ? "on" : ""}`}
                  onClick={() => {
                    setPinnedOnly((v) => !v);
                    setActive(0);
                  }}
                >
                  收藏
                </button>
                <button
                  className={`chip ${paused ? "on" : ""}`}
                  onClick={() => void togglePaused()}
                >
                  {paused ? "已暂停" : "记录中"}
                </button>
                <button className="icon-btn" onClick={() => setSettingsOpen(true)} title="设置">
                  <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true">
                    <circle
                      cx="8"
                      cy="8"
                      r="2.3"
                      fill="none"
                      stroke="currentColor"
                      strokeWidth="1.3"
                    />
                    <path
                      d="M8 1.5v1.7M8 12.8v1.7M1.5 8h1.7M12.8 8h1.7M3.4 3.4l1.2 1.2M11.4 11.4l1.2 1.2M12.6 3.4l-1.2 1.2M4.6 11.4l-1.2 1.2"
                      stroke="currentColor"
                      strokeWidth="1.3"
                      strokeLinecap="round"
                    />
                  </svg>
                </button>
              </div>
            </div>

            <div className="rail" ref={railRef}>
              {filtered.length === 0 ? (
                <div className="rail-empty">
                  <div className="empty-title">{query ? `没有匹配「${query}」` : "还没有记录"}</div>
                  <div className="empty-hint">
                    {query ? "试试更短的关键词" : "复制点什么，它会出现在这里"}
                  </div>
                  {!query && (
                    <div className="empty-kbd">
                      <kbd>{isMac ? "⌘" : "Ctrl"}</kbd>
                      <kbd>{isMac ? "⇧" : "Shift"}</kbd>
                      <kbd>V</kbd> 随时唤起
                    </div>
                  )}
                </div>
              ) : (
                filtered.map((clip, index) => {
                  const remote = Boolean(clip.device) && clip.device !== deviceName;
                  return (
                    <article
                      key={clip.id}
                      ref={index === active ? activeRef : undefined}
                      className={`card ${index === active ? "active" : ""} ${remote ? "remote" : ""} ${
                        clip.pinned ? "pinned" : ""
                      }`}
                      onMouseEnter={() => setActive(index)}
                      onMouseDown={(e) => e.preventDefault()}
                      onClick={() => void paste(clip.id)}
                    >
                      <div className="card-head">
                        <span className="kind">纯文本</span>
                        <span className="time">{relativeTime(clip.createdAt)}</span>
                      </div>
                      <div className="card-sub">
                        <span className={remote ? "app remote" : "app"}>
                          {remote ? `来自 ${clip.device}` : appLabel(clip.sourceApp)}
                        </span>
                      </div>
                      <div className="card-body">
                        <Highlight text={clip.preview} query={query} />
                      </div>
                      <div className="card-foot">
                        <span className="ord">{index + 1}</span>
                        <span className="size">{humanBytes(clip.bytes)}</span>
                        <button
                          className={`act ${clip.pinned ? "on" : ""}`}
                          title={clip.pinned ? "取消收藏" : "收藏"}
                          onClick={(e) => {
                            e.stopPropagation();
                            void togglePin(clip.id);
                          }}
                        >
                          <svg viewBox="0 0 16 16" width="12" height="12" aria-hidden="true">
                            <path
                              d="M8 1.9l1.9 3.8 4.2.6-3.1 3 .7 4.2L8 11.5l-3.7 2 .7-4.2-3.1-3 4.2-.6z"
                              fill={clip.pinned ? "currentColor" : "none"}
                              stroke="currentColor"
                              strokeWidth="1.2"
                              strokeLinejoin="round"
                            />
                          </svg>
                        </button>
                      </div>
                    </article>
                  );
                })
              )}
            </div>
          </>
        )}
      </div>

      {toast && <div className="toast">{toast}</div>}
    </div>
  );
}

function Switch({ on }: { on: boolean }) {
  return <span className={`switch ${on ? "on" : ""}`} aria-hidden="true" />;
}

/** 搜索结果高亮：只标第一处匹配，避免长文本里满屏色块 */
function Highlight({ text, query }: { text: string; query: string }) {
  const needle = query.trim();
  if (!needle) return <>{text}</>;
  const index = text.toLowerCase().indexOf(needle.toLowerCase());
  if (index < 0) return <>{text}</>;
  return (
    <>
      {text.slice(0, index)}
      <mark>{text.slice(index, index + needle.length)}</mark>
      {text.slice(index + needle.length)}
    </>
  );
}
