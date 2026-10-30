#!/bin/zsh
# 打 macOS 分发包：.app → dmg + zip，产物落在 release/<version>/
#
# 不用 tauri 内置的 dmg bundler：它的 bundle_dmg.sh 会调 AppleScript 驱动 Finder
# 去摆窗口位置，在没有图形会话（CI / 无头）时会报 -10004 权限违例并整体失败。
# hdiutil 直接造盘更稳，只是没有自定义背景图。
set -e
# zsh 默认 glob 无匹配就报错，清理旧产物时会误伤
setopt NULL_GLOB 2>/dev/null || true
cd "$(dirname "$0")/.."

VERSION=$(node -p "require('./package.json').version")
ARCH=$(uname -m)
export PATH="$HOME/.cargo/bin:$PATH"

echo "==> 构建 JCopy $VERSION ($ARCH)"
npm run app:build -- --bundles app

APP="src-tauri/target/release/bundle/macos/JCopy.app"
[ -d "$APP" ] || { echo "找不到 $APP"; exit 1; }

OUT="release/$VERSION"
mkdir -p "$OUT"
rm -f "$OUT"/JCopy-*.dmg "$OUT"/JCopy-*.zip

echo "==> 生成 dmg"
STAGE=$(mktemp -d)
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
hdiutil create -volname "JCopy" -srcfolder "$STAGE" -ov -format UDZO \
  "$OUT/JCopy-$VERSION-$ARCH.dmg" >/dev/null
rm -rf "$STAGE"

echo "==> 生成 zip"
ditto -c -k --sequesterRsrc --keepParent "$APP" "$OUT/JCopy-$VERSION-$ARCH.zip"

# 未签名包首次打开会被 Gatekeeper 拦，附一句说明省得用户以为坏了
cat > "$OUT/安装说明.txt" <<'TXT'
JCopy 安装说明

1. 把 JCopy.app 拖进「应用程序」文件夹
2. 首次打开会被 Gatekeeper 拦下（安装包未做 Apple 开发者签名）
   解决办法：在「应用程序」里右键 JCopy → 打开
   或执行：xattr -dr com.apple.quarantine /Applications/JCopy.app
3. 启动后菜单栏出现剪贴板图标（没有 Dock 图标，这是有意的）
4. 复制任意文本，按 Command+Shift+V 验证

跨设备同步：
  两台设备登录同一个 iCloud 账号并装好 iCloud 云盘客户端
  A 设备：设置 → 跨设备同步 → 生成配对码
  B 设备：设置 → 输入配对码
TXT

echo "==> 产物"
ls -lh "$OUT"
