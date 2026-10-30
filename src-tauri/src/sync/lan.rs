//! 局域网直连：mDNS 发现 + 短连接推送。
//!
//! 刻意不维护长连接——剪贴板事件是低频的，为每条消息建一次 TCP（同网段 <20ms）
//! 比维护心跳、断线重连、连接池简单得多，出问题也更好排查。
//!
//! iCloud 通道始终保留：局域网只负责把常见场景从"几秒"拉回"几十毫秒"，
//! 连不上就静默放弃，不影响可用性。

use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use super::crypto;
use super::protocol::{self, DeviceInfo, SyncClip};
use crate::db;
use crate::state::AppState;

const SERVICE_TYPE: &str = "_jcopy._tcp.local.";
const ID_LEN: usize = 16;
const MAX_FRAME: usize = 4 * 1024 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_millis(700);

#[derive(Serialize, Deserialize)]
struct LanPayload {
    v: u32,
    device: DeviceInfo,
    clips: Vec<SyncClip>,
}

pub fn spawn(app: AppHandle, state: Arc<AppState>) {
    thread::spawn(move || {
        let listener = match TcpListener::bind("0.0.0.0:0") {
            Ok(l) => l,
            Err(e) => {
                eprintln!("[jcopy] 局域网监听失败: {e}");
                return;
            }
        };
        let Ok(addr) = listener.local_addr() else { return };
        {
            let mut lan = state.lan.lock().unwrap();
            lan.port = addr.port();
        }

        // 服务端：每连接一个线程
        {
            let app = app.clone();
            let state = state.clone();
            thread::spawn(move || serve(app, state, listener));
        }

        match start_discovery(state.clone(), addr.port()) {
            Ok(daemon) => {
                let mut lan = state.lan.lock().unwrap();
                lan.daemon = Some(daemon);
                eprintln!("[jcopy] 局域网直连已启动，端口 {}", addr.port());
            }
            Err(e) => eprintln!("[jcopy] mDNS 启动失败（仅影响局域网加速）: {e}"),
        }
    });
}

fn serve(app: AppHandle, state: Arc<AppState>, listener: TcpListener) {
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let app = app.clone();
        let state = state.clone();
        thread::spawn(move || {
            if let Err(e) = handle_conn(&app, &state, stream) {
                // 同网段扫端口的、没配对的都会走到这里，不值得当错误刷屏
                let _ = e;
            }
        });
    }
}

fn handle_conn(app: &AppHandle, state: &Arc<AppState>, mut stream: TcpStream) -> Result<(), String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| e.to_string())?;

    let mut id_buf = [0u8; ID_LEN];
    stream
        .read_exact(&mut id_buf)
        .map_err(|e| e.to_string())?;
    let sender_id = String::from_utf8_lossy(&id_buf)
        .trim_end_matches('\0')
        .to_string();
    if sender_id.is_empty() {
        return Err("缺少设备标识".into());
    }

    let mut len_buf = [0u8; 4];
    stream
        .read_exact(&mut len_buf)
        .map_err(|e| e.to_string())?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len == 0 || len > MAX_FRAME {
        return Err("帧长度异常".into());
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).map_err(|e| e.to_string())?;

    let master = {
        let rt = state.sync.lock().map_err(|e| e.to_string())?;
        rt.key.ok_or("尚未配对")?
    };

    // 能解开就说明对方持有主密钥，等价于完成了身份校验
    let key = crypto::lan_key(&master, &sender_id);
    let plain = crypto::decrypt(&key, &buf).ok_or("解密失败")?;
    let payload: LanPayload = serde_json::from_slice(&plain).map_err(|e| e.to_string())?;

    let mut received = 0usize;
    for clip in payload.clips {
        let saved = {
            let conn = state.db.lock().map_err(|e| e.to_string())?;
            db::insert_remote(
                &conn,
                &clip.text,
                clip.source_app.as_deref(),
                &payload.device.name,
                &clip.hash,
                clip.created_at,
            )
            .map_err(|e| e.to_string())?
        };
        if let Some(saved) = saved {
            received += 1;
            let _ = app.emit("clip-added", saved.view());
        }
    }

    if received > 0 {
        eprintln!(
            "[jcopy] 局域网收到 {received} 条，来自 {}",
            payload.device.name
        );
    }
    Ok(())
}

/// 把新复制的条目推给同网段对端。失败静默——iCloud 通道会兜底。
pub fn broadcast_one(app: AppHandle, state: Arc<AppState>, clip: SyncClip) {
    thread::spawn(move || {
        let targets: Vec<SocketAddr> = {
            let lan = match state.lan.lock() {
                Ok(l) => l,
                Err(_) => return,
            };
            lan.peers.values().copied().collect()
        };
        if targets.is_empty() {
            return;
        }

        let master = {
            let rt = match state.sync.lock() {
                Ok(r) => r,
                Err(_) => return,
            };
            match rt.key {
                Some(k) => k,
                None => return,
            }
        };
        let device = {
            let Ok(settings) = state.settings.lock() else {
                return;
            };
            DeviceInfo {
                id: settings.device_id.clone(),
                name: settings.device_name.clone(),
                platform: protocol::current_platform().to_string(),
            }
        };

        let _ = app;
        for addr in targets {
            let _ = send_to(addr, &master, &device, &clip);
        }
    });
}

fn send_to(
    addr: SocketAddr,
    master: &crypto::Key,
    device: &DeviceInfo,
    clip: &SyncClip,
) -> Result<(), String> {
    let payload = LanPayload {
        v: protocol::PROTOCOL_VERSION,
        device: device.clone(),
        clips: vec![clip.clone()],
    };
    let json = serde_json::to_vec(&payload).map_err(|e| e.to_string())?;
    let key = crypto::lan_key(master, &device.id);
    let sealed = crypto::encrypt(&key, &json).ok_or("加密失败")?;

    let mut stream =
        TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT).map_err(|e| e.to_string())?;
    stream.set_write_timeout(Some(Duration::from_secs(2))).ok();

    let mut id_buf = [0u8; ID_LEN];
    let bytes = device.id.as_bytes();
    let n = bytes.len().min(ID_LEN);
    id_buf[..n].copy_from_slice(&bytes[..n]);

    stream.write_all(&id_buf).map_err(|e| e.to_string())?;
    stream
        .write_all(&(sealed.len() as u32).to_be_bytes())
        .map_err(|e| e.to_string())?;
    stream.write_all(&sealed).map_err(|e| e.to_string())?;
    let _ = stream.flush();
    Ok(())
}

fn start_discovery(state: Arc<AppState>, port: u16) -> Result<ServiceDaemon, String> {
    let settings = state
        .settings
        .lock()
        .map_err(|e| e.to_string())?
        .clone();

    let ip = local_ip().ok_or("无法确定本机局域网地址")?;
    let host = format!("{}.local.", sanitize_host(&settings.device_name));

    let info = ServiceInfo::new(
        SERVICE_TYPE,
        &settings.device_id,
        &host,
        ip.to_string().as_str(),
        port,
        &[("v", "1")][..],
    )
    .map_err(|e| e.to_string())?;

    let daemon = ServiceDaemon::new().map_err(|e| e.to_string())?;
    daemon.register(info).map_err(|e| e.to_string())?;

    let receiver = daemon.browse(SERVICE_TYPE).map_err(|e| e.to_string())?;
    let own_id = settings.device_id.clone();

    thread::spawn(move || {
        while let Ok(event) = receiver.recv() {
            let ServiceEvent::ServiceResolved(info) = event else {
                continue;
            };
            let peer_id = info
                .get_fullname()
                .split('.')
                .next()
                .unwrap_or_default()
                .to_string();
            if peer_id.is_empty() || peer_id == own_id {
                continue;
            }
            let Some(ip) = info.get_addresses().iter().find(|a| a.is_ipv4()).copied() else {
                continue;
            };
            let addr = SocketAddr::new(ip, info.get_port());
            if let Ok(mut lan) = state.lan.lock() {
                let is_new = lan.peers.insert(peer_id.clone(), addr).is_none();
                if is_new {
                    eprintln!("[jcopy] 发现同网段设备 {} @{}", peer_id, addr);
                }
            }
        }
    });

    Ok(daemon)
}

/// 不实际发包，只是让内核选出默认出口网卡的地址
fn local_ip() -> Option<IpAddr> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    Some(socket.local_addr().ok()?.ip())
}

fn sanitize_host(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "jcopy".into()
    } else {
        trimmed
    }
}
