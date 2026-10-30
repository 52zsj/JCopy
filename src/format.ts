const APP_NAMES: Record<string, string> = {
  "com.apple.Safari": "Safari",
  "com.apple.Terminal": "终端",
  "com.apple.finder": "访达",
  "com.apple.Notes": "备忘录",
  "com.apple.mail": "邮件",
  "com.apple.TextEdit": "文本编辑",
  "com.apple.dt.Xcode": "Xcode",
  "com.google.Chrome": "Chrome",
  "com.microsoft.VSCode": "VS Code",
  "com.microsoft.edgemac": "Edge",
  "com.tinyspeck.slackmacgap": "Slack",
  "com.tencent.xinWeChat": "微信",
  "ru.keepcoder.Telegram": "Telegram",
  "notion.id": "Notion",
  "com.figma.Desktop": "Figma",
};

/** bundle id 或进程名 → 可读的应用名。未知时取最后一段并首字母大写。 */
export function appLabel(id: string | null): string {
  if (!id) return "未知应用";
  const known = APP_NAMES[id];
  if (known) return known;
  const seg = id.split(".").filter(Boolean).pop();
  if (!seg) return id;
  return seg.charAt(0).toUpperCase() + seg.slice(1);
}

function pad(n: number): string {
  return n < 10 ? `0${n}` : String(n);
}

/** 相对时间：刚刚 / N 分钟前 / 今天 HH:mm / 昨天 HH:mm / N 天前 / M月D日 */
export function relativeTime(ms: number, now: number = Date.now()): string {
  const diff = Math.max(0, now - ms);
  const MIN = 60_000;
  const HOUR = 3_600_000;

  if (diff < 45_000) return "刚刚";
  if (diff < HOUR) return `${Math.floor(diff / MIN)} 分钟前`;

  const then = new Date(ms);
  const hhmm = `${pad(then.getHours())}:${pad(then.getMinutes())}`;

  const todayStart = new Date(now);
  todayStart.setHours(0, 0, 0, 0);
  const thenStart = new Date(ms);
  thenStart.setHours(0, 0, 0, 0);
  const dayDiff = Math.round((todayStart.getTime() - thenStart.getTime()) / 86_400_000);

  if (dayDiff <= 0) return hhmm;
  if (dayDiff === 1) return `昨天 ${hhmm}`;
  if (dayDiff < 7) return `${dayDiff} 天前`;
  return `${then.getMonth() + 1}月${then.getDate()}日`;
}

/** 体积展示：512 B / 1.2 KB */
export function humanBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}

export function platformName(p: string): string {
  switch (p) {
    case "macos":
      return "macOS";
    case "windows":
      return "Windows";
    case "linux":
      return "Linux";
    default:
      return p || "未知系统";
  }
}
