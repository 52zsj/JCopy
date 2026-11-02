// cocoa crate 已废弃（建议迁 objc2），macOS 分支仍在用它读前台应用，
// 这里局部静音，避免构建刷屏
#![allow(deprecated)]

// 只在 macOS 分支里用得到；不加 cfg 的话 Windows 构建会报 unused import
#[cfg(target_os = "macos")]
use std::ffi::CStr;

/// 当前前台应用的标识，用于卡片上的「来源」标记与黑名单判断。
#[cfg(target_os = "macos")]
pub fn frontmost_app() -> Option<String> {
    use cocoa::base::{id, nil};
    use objc::{class, msg_send, sel, sel_impl};

    unsafe {
        let ws: id = msg_send![class!(NSWorkspace), sharedWorkspace];
        if ws == nil {
            return None;
        }
        let app: id = msg_send![ws, frontmostApplication];
        if app == nil {
            return None;
        }
        let bundle: id = msg_send![app, bundleIdentifier];
        if bundle == nil {
            return None;
        }
        let ptr = cocoa::foundation::NSString::UTF8String(bundle);
        if ptr.is_null() {
            None
        } else {
            Some(CStr::from_ptr(ptr).to_string_lossy().into_owned())
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn frontmost_app() -> Option<String> {
    None
}

#[cfg(target_os = "macos")]
use mac_paste as paste_impl;

#[cfg(target_os = "macos")]
mod mac_paste {
    use std::ffi::c_void;

    type CGEventRef = *mut c_void;

    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn CGEventCreateKeyboardEvent(source: *mut c_void, vk: u16, key_down: bool) -> CGEventRef;
        fn CGEventSetFlags(event: CGEventRef, flags: u64);
        fn CGEventPost(tap: u32, event: CGEventRef);
        fn AXIsProcessTrusted() -> bool;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(cf: *const c_void);
    }

    const KVK_ANSI_V: u16 = 9;
    const FLAG_COMMAND: u64 = 1 << 20;
    const HID_TAP: u32 = 0;

    pub fn accessible() -> bool {
        unsafe { AXIsProcessTrusted() }
    }

    /// 合成 Cmd+V。需要「辅助功能」授权，未授权时返回可读错误。
    pub fn paste() -> Result<(), String> {
        if !accessible() {
            return Err("需要「系统设置 → 隐私与安全性 → 辅助功能」授权".into());
        }
        unsafe {
            let down = CGEventCreateKeyboardEvent(std::ptr::null_mut(), KVK_ANSI_V, true);
            let up = CGEventCreateKeyboardEvent(std::ptr::null_mut(), KVK_ANSI_V, false);
            if down.is_null() || up.is_null() {
                if !down.is_null() {
                    CFRelease(down as *const c_void);
                }
                if !up.is_null() {
                    CFRelease(up as *const c_void);
                }
                return Err("无法创建按键事件".into());
            }
            CGEventSetFlags(down, FLAG_COMMAND);
            CGEventSetFlags(up, FLAG_COMMAND);
            CGEventPost(HID_TAP, down);
            std::thread::sleep(std::time::Duration::from_millis(12));
            CGEventPost(HID_TAP, up);
            CFRelease(down as *const c_void);
            CFRelease(up as *const c_void);
        }
        Ok(())
    }
}

#[cfg(target_os = "windows")]
use win_paste as paste_impl;

#[cfg(target_os = "windows")]
mod win_paste {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
        KEYEVENTF_KEYUP, VIRTUAL_KEY, VK_CONTROL, VK_V,
    };

    pub fn accessible() -> bool {
        true
    }

    /// 合成 Ctrl+V。受 UIPI 限制：目标窗口若以管理员权限运行，注入会被系统丢弃。
    pub fn paste() -> Result<(), String> {
        unsafe {
            let inputs = [
                key(VK_CONTROL, false),
                key(VK_V, false),
                key(VK_V, true),
                key(VK_CONTROL, true),
            ];
            let sent = SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
            if sent == 0 {
                return Err("模拟按键被系统拦截（目标窗口可能以管理员权限运行）".into());
            }
        }
        Ok(())
    }

    fn key(vk: VIRTUAL_KEY, up: bool) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    wScan: 0,
                    dwFlags: if up {
                        KEYEVENTF_KEYUP
                    } else {
                        KEYBD_EVENT_FLAGS(0)
                    },
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }
}

/// 是否具备模拟按键的能力（macOS 需辅助功能授权）。
pub fn accessible() -> bool {
    paste_impl::accessible()
}

/// 向当前前台应用发送粘贴快捷键。
pub fn paste() -> Result<(), String> {
    paste_impl::paste()
}

#[cfg(target_os = "linux")]
use linux_paste as paste_impl;

#[cfg(target_os = "linux")]
mod linux_paste {
    pub fn accessible() -> bool {
        true
    }

    /// X11 下可用 XTest 实现，Wayland 基本不可用；先返回明确提示，让用户手动粘贴。
    pub fn paste() -> Result<(), String> {
        Err("当前平台暂不支持自动粘贴，请手动粘贴".into())
    }
}

/// 剪贴板变更计数：只要这个数没变，就说明剪贴板没被改过，完全不必读取内容。
/// 监听线程每 350ms 轮询一次，如果每次都去 `get_image()`，一张截图会被反复
/// 拷进内存再算 SHA-256——那是白烧 CPU 和电。
pub fn clipboard_seq() -> Option<i64> {
    #[cfg(target_os = "macos")]
    {
        use cocoa::base::id;
        use objc::{class, msg_send, sel, sel_impl};
        unsafe {
            let pasteboard: id = msg_send![class!(NSPasteboard), generalPasteboard];
            if pasteboard.is_null() {
                return None;
            }
            let count: i64 = msg_send![pasteboard, changeCount];
            Some(count)
        }
    }

    #[cfg(target_os = "windows")]
    {
        use windows::Win32::System::DataExchange::GetClipboardSequenceNumber;
        let seq = unsafe { GetClipboardSequenceNumber() };
        // 0 表示拿不到（例如没有剪贴板访问权限），交给上层退回内容比对
        if seq == 0 {
            None
        } else {
            Some(seq as i64)
        }
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        None
    }
}
