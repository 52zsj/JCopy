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
# JCOPY_TARGET 用于交叉/CI 构建（如 x86_64-apple-darwin）；留空则用本机默认
TARGET="${JCOPY_TARGET:-}"
if [ -n "$TARGET" ]; then
  case "$TARGET" in
    aarch64-*) ARCH="arm64" ;;
    x86_64-*)  ARCH="x64" ;;
    *)         ARCH="${TARGET%%-*}" ;;
  esac
  OUT_SUFFIX="$TARGET/release"
else
  ARCH=$(uname -m)
  OUT_SUFFIX="release"
fi
export PATH="$HOME/.cargo/bin:$PATH"

echo "==> 构建 JCopy $VERSION ($ARCH)"
# 传 --skip-build 时复用已有的 .app；否则重新构建前端与 Rust
# 注意：zsh 默认不对未加引号的变量做词分割，`$BUILD_ARGS` 会被当成**一个**参数传给
# tauri（报 "unexpected argument '--target aarch64-apple-darwin'"）。这里必须显式分支。
if [ "${JCOPY_SKIP_BUILD:-0}" = "1" ]; then
  echo "    (跳过构建，复用现有 .app)"
elif [ -n "$TARGET" ]; then
  echo "    (交叉构建 $TARGET)"
  npm run app:build -- --bundles app --target "$TARGET"
else
  npm run app:build -- --bundles app
fi

APP="src-tauri/target/$OUT_SUFFIX/bundle/macos/JCopy.app"
[ -d "$APP" ] || { echo "找不到 $APP（若用了 --skip-build，先完整构建一次）"; exit 1; }

# 版本号写在三个地方（package.json / tauri.conf.json / Cargo.toml），漏改一个就会出现
# "文件名叫 0.9.1、里面装的还是 0.9.0" 的情况。这里以 Info.plist 为准校验一次。
APP_VERSION=$(/usr/libexec/PlistBuddy -c "Print :CFBundleShortVersionString" \
  "$APP/Contents/Info.plist" 2>/dev/null || echo "")
if [ -n "$APP_VERSION" ] && [ "$APP_VERSION" != "$VERSION" ]; then
  # 变量必须用 ${} 包起来：紧跟中文全角逗号时，zsh 会把标点当作变量名的一部分
  echo "✗ 版本不一致：文件名叫 ${VERSION}，但 Info.plist 里是 ${APP_VERSION}"
  echo "  检查 src-tauri/tauri.conf.json 与 src-tauri/Cargo.toml 的 version 是否已同步"
  exit 1
fi

# 清掉历史遗留的中间镜像（tauri 内置 dmg 打包器失败会留下 rw.*.dmg，30MB 起步）
rm -f "src-tauri/target/$OUT_SUFFIX/bundle/macos/"rw.*.dmg 2>/dev/null || true

OUT="release/$VERSION"
mkdir -p "$OUT"
# 只清自己这个架构的产物：同一个版本号下可能同时存在 arm64 与 x64，
# 用 JCopy-*.dmg 会把另一架构刚打好的包一并删掉
rm -f "$OUT"/JCopy-"$VERSION"-"$ARCH".dmg "$OUT"/JCopy-"$VERSION"-"$ARCH".zip

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
