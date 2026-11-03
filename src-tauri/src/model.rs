use serde::{Deserialize, Serialize};

/// 发给前端的轻量视图：不含全文，避免面板加载时传输大量文本。
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ClipView {
    pub id: String,
    pub kind: String,
    pub preview: String,
    pub source_app: Option<String>,
    pub device: String,
    pub created_at: i64,
    pub last_used_at: i64,
    pub pinned: bool,
    pub bytes: i64,
    /// 图片条目的缩略图绝对路径（前端用 asset 协议转 URL），文本为空
    pub thumb: Option<String>,
    /// 图片尺寸，形如 "1920×1080"
    pub dims: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Clip {
    pub id: String,
    pub kind: String,
    pub content: String,
    /// 入库时算好的一行摘要：文本是内容前 400 字，图片是 "1920 × 1080"
    pub preview: String,
    pub hash: String,
    pub source_app: Option<String>,
    pub device: String,
    pub created_at: i64,
    pub last_used_at: i64,
    pub pinned: bool,
    pub bytes: i64,
    /// 是否本机产生。同步只发 true 的条目——用布尔标记而不是比对设备名，
    /// 这样改过设备名、或两台机器同名，都不会漏发或反复回传。
    pub local: bool,
}

impl Clip {
    pub fn view(&self) -> ClipView {
        let is_image = self.kind == "image";
        ClipView {
            id: self.id.clone(),
            kind: self.kind.clone(),
            preview: self.preview.clone(),
            source_app: self.source_app.clone(),
            device: self.device.clone(),
            created_at: self.created_at,
            last_used_at: self.last_used_at,
            pinned: self.pinned,
            bytes: self.bytes,
            // 图片条目的 content 是原图路径，缩略图就放在旁边。
            // 用 Path 改扩展名，不要用 replace(".png", ...)：
            // 那样会把目录名里的 .png 一起替换（C:\a.png\id.png → C:\a.t.png\id.t.png）
            thumb: if is_image {
                Some(
                    std::path::Path::new(&self.content)
                        .with_extension("t.png")
                        .to_string_lossy()
                        .into_owned(),
                )
            } else {
                None
            },
            dims: if is_image {
                Some(self.preview.clone())
            } else {
                None
            },
        }
    }
}

const PREVIEW_MAX_CHARS: usize = 400;

/// 单行预览：折叠所有空白，超长截断。前端据此渲染卡片首行。
pub fn preview_of(text: &str) -> String {
    let mut out = String::with_capacity(PREVIEW_MAX_CHARS);
    let mut last_space = false;
    for ch in text.chars() {
        if out.chars().count() >= PREVIEW_MAX_CHARS {
            out.push('…');
            break;
        }
        if ch.is_whitespace() {
            if !last_space && !out.is_empty() {
                out.push(' ');
                last_space = true;
            }
        } else {
            out.push(ch);
            last_space = false;
        }
    }
    out.trim_end().to_string()
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub device_id: String,
    pub device_name: String,
    /// 关闭时仅写回剪贴板，用户自行 Cmd+V
    pub auto_paste: bool,
    pub max_clips: i64,
    /// 字节上限，默认 200MB
    pub max_bytes: i64,
    pub hotkey: String,
    pub sync_enabled: bool,
    /// 同步主密钥（base64）。配对后写入，配置文件权限收紧到 600。
    /// 自定义同步目录；为空时自动探测 iCloud 云盘
    pub sync_dir: Option<String>,
    /// 不记录复制内容的应用（bundle id / 进程名）
    pub blacklist: Vec<String>,
    /// 外观主题：system / light / dark
    pub theme: String,
    /// 字号档位：small / medium / large
    pub font_size: String,
    /// 图片同步策略：off 不同步 / pinned 仅收藏 / all 全部。
    /// 默认 pinned——图片体积大，默认全同步会把 iCloud 塞满。
    pub image_sync: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            device_id: String::new(),
            device_name: String::new(),
            auto_paste: false,
            max_clips: 500,
            max_bytes: 200 * 1024 * 1024,
            hotkey: "CommandOrControl+Shift+V".into(),
            sync_enabled: true,
            sync_dir: None,
            blacklist: default_blacklist(),
            theme: "system".into(),
            font_size: "medium".into(),
            // 默认同步全部图片（12MB 以内）。原来默认 "pinned"（只有收藏过的才发），
            // 结果是用户复制一张图，另一台毫无反应，还以为功能坏了。
            image_sync: "all".into(),
        }
    }
}

/// 内置建议名单（首次启动带出，用户可自行增删）。
fn default_blacklist() -> Vec<String> {
    [
        "com.1password.1password",
        "com.agilebits.onepassword7",
        "com.bitwarden.desktop",
        "com.apple.keychainaccess",
        "org.keepassxc.keepassxc",
        "me.proton.pass",
        "com.lastpass.LastPass",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}
