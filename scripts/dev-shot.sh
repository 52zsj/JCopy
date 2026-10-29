#!/bin/zsh
# 本地调试：起 Vite、拉起 debug 版 JCopy、带上启动即展开面板
cd "$(dirname "$0")/.." || exit 1

pkill -f "target/debug/jcopy" 2>/dev/null
sleep 0.5

nohup npm run dev >/tmp/jcopy-vite.log 2>&1 &
for _ in {1..60}; do
  if curl -s -o /dev/null http://localhost:1420; then break; fi
  sleep 0.25
done
echo "vite status: $(curl -s -o /dev/null -w '%{http_code}' http://localhost:1420)"

JCOPY_NO_AUTOHIDE=1 JCOPY_SHOW_ON_START=1 nohup ./src-tauri/target/debug/jcopy >/tmp/jcopy-app.log 2>&1 &
APP_PID=$!

sleep 6
if kill -0 "$APP_PID" 2>/dev/null; then echo "app alive: yes"; else echo "app alive: no"; fi

echo "--- app log ---"
cat /tmp/jcopy-app.log

# 截屏需要「屏幕录制」权限，未授权时 macOS 只返回桌面壁纸
if screencapture -x /tmp/jcopy-shot.png 2>/dev/null; then
  echo "shot: /tmp/jcopy-shot.png"
fi
