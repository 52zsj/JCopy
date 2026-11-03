import {
  type CSSProperties,
  type MutableRefObject,
  memo,
  useCallback,
  useDeferredValue,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { api, type ClipView, type Settings, type SyncStatus } from "./api";
import {
  appLabel,
  detectKind,
  humanBytes,
  KIND_META,
  platformName,
  relativeTime,
} from "./format";

export default function App() {
  const [clips, setClips] = useState<ClipView[]>([]);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [paused, setPaused] = useState(false);
  const [canPaste, setCanPaste] = useState(false);
  const [query, setQuery] = useState("");
  const [pinnedOnly, setPinnedOnly] = useState(false);
  const [active, setActive] = useState(0);
  // 卡片是 memo 的，不随父组件重渲染；"3 分钟前"要自己走时，否则会停住
  const [now, setNow] = useState(() => Date.now());
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [toast, setToast] = useState<string | null>(null);
  // 卡片的右键菜单：位置 + 作用对象
  const [menu, setMenu] = useState<{ x: number; y: number; clip: ClipView } | null>(null);
  const [sync, setSync] = useState<SyncStatus | null>(null);
  const [dirOpen, setDirOpen] = useState(false);
  const [dirPath, setDirPath] = useState("");

  const inputRef = useRef<HTMLInputElement>(null);
  const activeRef = useRef<HTMLElement | null>(null);
  const railRef = useRef<HTMLDivElement>(null);
  const maxClipsRef = useRef(500);

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
        setClips((prev) => {
          const next = [event.payload, ...prev.filter((c) => c.id !== event.payload.id)];
          // 面板长期隐藏时事件仍在累积，不裁剪的话数组会越滚越大，之后每次渲染都更慢
          return next.length > maxClipsRef.current ? next.slice(0, maxClipsRef.current) : next;
        });
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
        // active 本来就是 0 时 scrollIntoView 不会触发，滚动条会停在上一次的位置
        // （拉到最右后重开，卡片就全在屏幕外了），这里显式归零
        if (railRef.current) railRef.current.scrollLeft = 0;
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

  // 主题与字号写到 <html> 上，CSS 只认 data-*；system 交给媒体查询，
  // 这里监听系统主题变化实时跟随
  useEffect(() => {
    const root = document.documentElement;
    const media = window.matchMedia("(prefers-color-scheme: dark)");

    const apply = () => {
      const theme = settings?.theme ?? "system";
      const resolved = theme === "system" ? (media.matches ? "dark" : "light") : theme;
      root.dataset.theme = resolved;
      root.dataset.font = settings?.fontSize ?? "medium";
    };

    apply();
    media.addEventListener("change", apply);
    return () => media.removeEventListener("change", apply);
  }, [settings?.theme, settings?.fontSize]);

  // 设置弹窗打开时把焦点从搜索框挪走，否则用户打字会打进被遮住的输入框
  useEffect(() => {
    if (settingsOpen) {
      inputRef.current?.blur();
    } else {
      requestAnimationFrame(() => inputRef.current?.focus());
    }
  }, [settingsOpen]);

  // 相对时间每 30 秒走一次（卡片是 memo 的，不会因为别处重渲染而顺带刷新）
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 30_000);
    return () => clearInterval(timer);
  }, []);

  // 事件回调只注册一次，用 ref 取最新的上限值
  useEffect(() => {
    maxClipsRef.current = settings?.maxClips ?? 500;
  }, [settings?.maxClips]);

  // 搜索用延迟值：敲字时输入框立刻响应，列表过滤排到下一帧，
  // 否则每按一个键都要对 500 条做一次 toLowerCase + 全量重渲染
  const deferredQuery = useDeferredValue(query);

  const filtered = useMemo(() => {
    const q = deferredQuery.trim().toLowerCase();
    return clips.filter((clip) => {
      if (pinnedOnly && !clip.pinned) return false;
      if (!q) return true;
      if (clip.preview.toLowerCase().includes(q)) return true;
      return appLabel(clip.sourceApp).toLowerCase().includes(q);
    });
  }, [clips, deferredQuery, pinnedOnly]);

  useEffect(() => {
    setActive((index) => Math.min(index, Math.max(0, filtered.length - 1)));
  }, [filtered.length]);

  useEffect(() => {
    activeRef.current?.scrollIntoView({ block: "nearest", inline: "nearest" });
  }, [active]);

  // 竖向滚轮转成横向滚动：卡片条是横的，用户不会去拖滚动条。
  // 触控板一次滚动会连发几十个 wheel 事件，这里累加后按帧提交，
  // 否则每个事件都直接写 scrollLeft，滚动条会抖
  useEffect(() => {
    if (settingsOpen) return;
    const el = railRef.current;
    if (!el) return;

    let pending = 0;
    let frame = 0;
    const flush = () => {
      if (pending !== 0) {
        el.scrollLeft += pending;
        pending = 0;
      }
      frame = 0;
    };
    const onWheel = (event: WheelEvent) => {
      if (Math.abs(event.deltaY) <= Math.abs(event.deltaX)) return;
      event.preventDefault();
      pending += event.deltaY;
      if (!frame) frame = requestAnimationFrame(flush);
    };

    el.addEventListener("wheel", onWheel, { passive: false });
    return () => {
      el.removeEventListener("wheel", onWheel);
      if (frame) cancelAnimationFrame(frame);
    };
  }, [settingsOpen]);

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

  const revealFile = useCallback(async (id: string) => {
    try {
      await api.revealClip(id);
    } catch (error) {
      setToast(typeof error === "string" ? error : "打开失败");
    }
  }, []);

  const openMenu = useCallback((clip: ClipView, x: number, y: number) => {
    setMenu({ clip, x, y });
  }, []);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.isComposing) return;
      const mod = event.metaKey || event.ctrlKey;

      if (event.key === "Escape") {
        event.preventDefault();
        if (menu) {
          setMenu(null);
          return;
        }
        if (settingsOpen) {
          setSettingsOpen(false);
          requestAnimationFrame(() => inputRef.current?.focus());
        } else {
          void api.hidePanel();
        }
        return;
      }

      if (settingsOpen) return;
      // 空列表时别再走方向键逻辑：Math.min(i+1, -1) 会把 active 打成 -1，
      // 之后 filtered[-1] 恒为 undefined，回车就永远不响应
      if (!filtered.length) return;

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
  }, [active, filtered, menu, paste, removeClip, settingsOpen, togglePin]);

  // 同步状态常驻轮询：主界面的状态条要能实时反映"发出去几条、收到几条、几张图"。
  // deep 只在设置页打开时为 true——它要逐个发件箱解密，不能每 3 秒都做。
  useEffect(() => {
    let alive = true;
    const load = async () => {
      try {
        const next = await api.syncStatus(settingsOpen);
        if (alive) setSync(next);
      } catch {
        // 同步状态读不到不影响本机使用
      }
    };
    void load();
    const timer = setInterval(load, settingsOpen ? 3000 : 5000);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, [settingsOpen]);

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

  const runSyncNow = useCallback(async () => {
    try {
      await api.syncNow();
      setSync(await api.syncStatus(true));
      setToast("已同步");
    } catch (error) {
      setToast(typeof error === "string" ? error : "同步失败");
    }
  }, []);

  const deviceName = settings?.deviceName ?? "";
  const isMac = navigator.userAgent.includes("Mac");

  return (
    <div className="app">
      <div className="surface">
        <div className="grabber" aria-hidden="true" />

        {settingsOpen && (
          <div
            className="modal-backdrop"
            onMouseDown={(e) => {
              if (e.target === e.currentTarget) {
                setSettingsOpen(false);
                requestAnimationFrame(() => inputRef.current?.focus());
              }
            }}
          >
          <div className="modal">
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
              <div className="row static">
                <span>
                  外观
                  <em>主题切换即时生效，跟随系统会随深浅色自动变化</em>
                </span>
                <Seg
                  value={settings?.theme ?? "system"}
                  options={[
                    { value: "system", label: "跟随系统" },
                    { value: "light", label: "浅色" },
                    { value: "dark", label: "深色" },
                  ]}
                  onChange={(v) => void patchSettings({ theme: v })}
                />
              </div>

              <div className="row static">
                <span>
                  字号
                  <em>卡片正文与元信息的显示大小</em>
                </span>
                <Seg
                  value={settings?.fontSize ?? "medium"}
                  options={[
                    { value: "small", label: "小" },
                    { value: "medium", label: "中" },
                    { value: "large", label: "大" },
                  ]}
                  onChange={(v) => void patchSettings({ fontSize: v })}
                />
              </div>

              <div className="row static">
                <span>
                  图片同步
                  <em>默认同步 12MB 以内的图片；怕占云盘空间可以改成「仅收藏」</em>
                </span>
                <Seg
                  value={settings?.imageSync ?? "all"}
                  options={[
                    { value: "off", label: "不同步" },
                    { value: "pinned", label: "仅收藏" },
                    { value: "all", label: "全部" },
                  ]}
                  onChange={(v) => void patchSettings({ imageSync: v })}
                />
              </div>

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

              <button className="row" onClick={() => void runSyncNow()}>
                <span>
                  立即同步
                  <em>
                    {sync?.lastPullAt
                      ? `上次同步 ${relativeTime(sync.lastPullAt)}`
                      : "尚未同步过"}
                  </em>
                </span>
              </button>

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

              <div className="row static">
                <span>
                  共享目录图片
                  <em>
                    程序已放入 {sync?.sharedImages ?? 0} 张
                    {sync?.sharedImages
                      ? " · 到另一台的云盘文件夹里数一数这个数对不对，就知道云盘有没有传过去"
                      : " · 复制一张图试试，这里应该立刻 +1"}
                  </em>
                </span>
              </div>

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

              {sync && (
                <div className="sync-diag">
                  <div className="diag-head">
                    <span>同步详情</span>
                    <span className="diag-hint">
                      本机发布 {sync.ownClips} 条 · 共享目录图片 {sync.sharedImages} 张
                    </span>
                  </div>

                  {(sync.peerFiles ?? []).map((file) => (
                    <div
                      key={file.id}
                      className={`diag-line${
                        file.decryptable ? (file.missingImages > 0 ? " pending" : "") : " bad"
                      }`}
                    >
                      <span className="diag-id">{file.isSelf ? "本机" : file.id.slice(0, 8)}</span>
                      <span className="diag-meta">
                        {file.decryptable
                          ? `${file.clips} 条${
                              file.images > 0 ? ` · ${file.images} 张图` : ""
                            }${
                              file.missingImages > 0
                                ? ` · ${file.missingImages} 张原图未到云盘`
                                : ""
                            }`
                          : (file.error ?? "解不开")}
                      </span>
                      <span className="diag-time">
                        {file.updatedAt ? relativeTime(file.updatedAt) : "—"}
                      </span>
                    </div>
                  ))}

                  <div className="diag-note">
                    云盘目录剩余 {sync.dirFreeMb > 1_000_000 ? "未知" : `${sync.dirFreeMb} MB`}
                    {sync.dirFreeMb < 500
                      ? " · 空间不足时 iCloud 会拒绝上传，文件只留在本机，另一台永远收不到"
                      : " · 本机写入后由 iCloud 负责上传，通常 5–60 秒到达另一台"}
                  </div>

                  {sync.skippedOversize > 0 && (
                    <div className="diag-note">
                      有 {sync.skippedOversize} 条超过同步体积上限被跳过（图片 12MB / 文本 256KB）。
                      它们只在本机，不会同步到其他设备。
                    </div>
                  )}

                  {(sync.peerFiles ?? []).some((f) => f.missingImages > 0) && (
                    <div className="diag-note">
                      发件箱里已经有图片条目，但原图还在云盘上传/下载中。
                      图片要单独传原图，比文本慢，几 MB 通常 10 秒到 2 分钟。
                    </div>
                  )}
                </div>
              )}

              <button className="row" onClick={() => void refresh()}>
                <span>
                  重新检测权限
                  <em>{canPaste ? "辅助功能已授权" : "尚未授权"}</em>
                </span>
              </button>
            </div>
          </div>
          </div>
          </div>
        )}
        <>
            <div className="top-bar">
              <div className="top-side">
                <span className="brand">JCopy</span>
                <span className="count">{filtered.length}</span>
                {paused && <span className="paused-badge">已暂停</span>}
                {/* 同步状态条：无论开没开、配没配都要出现，
                    否则用户根本不知道同步处于什么状态（"以为在同步，其实没配对"） */}
                {sync && (
                  <button
                    className={`sync-chip ${
                      !sync.enabled
                        ? "off"
                        : sync.lastError
                          ? "bad"
                          : sync.waitingImages > 0 || sync.pendingImages > 0
                            ? "pending"
                            : sync.dirFreeMb < 200
                              ? "bad"
                              : "ok"
                    }`}
                    title={
                      !sync.enabled
                        ? "跨设备同步未开启 · 点击去开启"
                        : (sync.lastError ??
                              `发出 ${sync.pushedClips} 条（${sync.pushedImages} 图）· 收到 ${sync.pulledClips} 条（${sync.pulledImages} 图）` +
                                (sync.pendingImages > 0
                                  ? ` · ${sync.pendingImages} 张原图待传`
                                  : "") +
                                ` · 云盘目录剩余 ${sync.dirFreeMb} MB · 点击打开同步设置`)
                    }
                    onClick={() => setSettingsOpen(true)}
                  >
                    <i />
                    <span>
                      {!sync.enabled
                        ? "同步未开启"
                        : sync.lastError
                          ? "同步异常"
                          : sync.waitingImages > 0
                            ? `${sync.waitingImages} 张图待传`
                            : sync.dirFreeMb < 200
                              ? "磁盘将满"
                              : `↑${sync.pushedClips} ↓${sync.pulledClips}`}
                    </span>
                    {sync.enabled && !sync.lastError && (
                      <em>
                        图 {sync.pushedImages}/{sync.pulledImages}
                      </em>
                    )}
                  </button>
                )}
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
                  className={`chip pin ${pinnedOnly ? "on" : ""}`}
                  onClick={() => {
                    setPinnedOnly((v) => !v);
                    setActive(0);
                  }}
                >
                  收藏
                </button>
                <button
                  className={`chip rec ${paused ? "paused" : ""}`}
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
                filtered.map((clip, index) => (
                  <ClipCard
                    key={clip.id}
                    clip={clip}
                    index={index}
                    active={index === active}
                    query={deferredQuery}
                    remote={Boolean(clip.device) && clip.device !== deviceName}
                    activeRef={activeRef}
                    now={now}
                    onHover={setActive}
                    onPaste={paste}
                    onPin={togglePin}
                    onMenu={openMenu}
                  />
                ))
              )}
            </div>
        </>
      </div>

      {toast && <div className="toast">{toast}</div>}

      {menu && (
        <>
          <div
            className="ctx-mask"
            onMouseDown={() => setMenu(null)}
            onContextMenu={(event) => {
              event.preventDefault();
              setMenu(null);
            }}
          />
          <div
            className="ctx-menu"
            style={{
              // 贴着边缘右键时往回收，免得菜单被窗口裁掉
              left: Math.min(menu.x, window.innerWidth - 172),
              top: Math.min(menu.y, window.innerHeight - 190),
            }}
            onMouseDown={(event) => event.stopPropagation()}
          >
            <button
              onClick={() => {
                void paste(menu.clip.id);
                setMenu(null);
              }}
            >
              复制
            </button>
            <button
              onClick={() => {
                void togglePin(menu.clip.id);
                setMenu(null);
              }}
            >
              {menu.clip.pinned ? "取消收藏" : "收藏"}
            </button>
            {menu.clip.kind === "image" && (
              <button
                onClick={() => {
                  void revealFile(menu.clip.id);
                  setMenu(null);
                }}
              >
                在文件夹中显示
              </button>
            )}
            <span className="ctx-sep" />
            <button
              className="danger"
              onClick={() => {
                void removeClip(menu.clip.id);
                setMenu(null);
              }}
            >
              删除
            </button>
          </div>
        </>
      )}
    </div>
  );
}

/** 分段选择器：设置里切主题 / 字号这种二选一到四选一的场景 */
function Seg({
  value,
  options,
  onChange,
}: {
  value: string;
  options: Array<{ value: string; label: string }>;
  onChange: (value: string) => void;
}) {
  return (
    <div className="seg" role="group">
      {options.map((option) => (
        <button
          key={option.value}
          className={option.value === value ? "on" : ""}
          onClick={() => onChange(option.value)}
        >
          {option.label}
        </button>
      ))}
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

type ClipCardProps = {
  clip: ClipView;
  index: number;
  active: boolean;
  query: string;
  remote: boolean;
  activeRef: MutableRefObject<HTMLElement | null>;
  now: number;
  onHover: (index: number) => void;
  onPaste: (id: string) => void;
  onPin: (id: string) => void;
  onMenu: (clip: ClipView, x: number, y: number) => void;
};

/**
 * 单张卡片。抽成 memo 组件的原因：面板打开时 active / sync / toast 都在变，
 * 内联渲染会把 500 张卡连同其中的闭包、SVG 全部重建一遍。
 * 现在只有真正影响这张卡片的 props 变了才会重绘。
 */
const ClipCard = memo(function ClipCard({
  clip,
  index,
  active,
  query,
  remote,
  activeRef,
  now,
  onHover,
  onPaste,
  onPin,
  onMenu,
}: ClipCardProps) {
  const kind = detectKind(clip.kind, clip.preview);
  const meta = KIND_META[kind];
  // 内容本身是色值时，色带直接用那个颜色
  const band =
    kind === "color" && /^#/.test(clip.preview.trim())
      ? clip.preview.trim().slice(0, 7)
      : meta.color;
  const style = { "--kind": band, "--kind-soft": meta.soft } as CSSProperties;

  // 用回调 ref 而不是条件 ref：memo 组件里直接给对象 ref 会有类型/派发问题，
  // 这样 active 变化时 React 会先把旧的置空、再挂新的
  const attachRef = useCallback(
    (el: HTMLElement | null) => {
      if (active) activeRef.current = el;
    },
    [active, activeRef],
  );

  return (
    <article
      ref={attachRef}
      className={`card ${active ? "active" : ""} ${remote ? "remote" : ""} ${
        clip.pinned ? "pinned" : ""
      }`}
      style={style}
      onMouseEnter={() => onHover(index)}
      onMouseDown={(e) => e.preventDefault()}
      onContextMenu={(e) => {
        e.preventDefault();
        onMenu(clip, e.clientX, e.clientY);
      }}
      onClick={() => void onPaste(clip.id)}
    >
      <div className="card-head">
        <span className="kind">{meta.label}</span>
        <span className="time">{relativeTime(clip.createdAt, now)}</span>
      </div>
      <div className="card-sub">
        <span className={remote ? "app remote" : "app"}>
          {remote ? `来自 ${clip.device}` : appLabel(clip.sourceApp)}
        </span>
      </div>
      {clip.kind === "image" && clip.thumb ? (
        <div className="card-media">
          <img
            src={convertFileSrc(clip.thumb)}
            alt={clip.dims ?? "图片"}
            loading="lazy"
            decoding="async"
          />
        </div>
      ) : (
        <div className="card-body">
          <Highlight text={clip.preview} query={query} />
        </div>
      )}
      <div className="card-foot">
        <span className="ord">{index + 1}</span>
        <span className="size">
          {clip.kind === "image" && clip.dims ? clip.dims : humanBytes(clip.bytes)}
        </span>
        <button
          className={`act ${clip.pinned ? "on" : ""}`}
          title={clip.pinned ? "取消收藏" : "收藏"}
          onClick={(e) => {
            e.stopPropagation();
            void onPin(clip.id);
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
});
