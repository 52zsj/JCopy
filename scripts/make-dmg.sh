#!/bin/bash
# 用 hdiutil 造 dmg。
#
# 不用 `tauri build --bundles dmg`：它的 bundle_dmg.sh 会调 AppleScript 驱动 Finder
# 摆放窗口位置，在没有图形会话（CI / 无头）时报 -10004 权限违例并整体失败。
#
# 用法：scripts/make-dmg.sh <JCopy.app 路径> [文件名后缀，默认 uname -m]
set -euo pipefail
cd "$(dirname "$0")/.."

APP="${1:?用法: scripts/make-dmg.sh <JCopy.app 路径> [后缀]}"
SUFFIX="${2:-$(uname -m)}"
VERSION=$(node -p "require('./package.json').version")
OUT="release/$VERSION"

[ -d "$APP" ] || { echo "找不到 $APP"; exit 1; }
mkdir -p "$OUT"

STAGE=$(mktemp -d)
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"

hdiutil create -volname "JCopy" -srcfolder "$STAGE" -ov -format UDZO \
  "$OUT/JCopy-$VERSION-$SUFFIX.dmg" >/dev/null
rm -rf "$STAGE"

echo "$OUT/JCopy-$VERSION-$SUFFIX.dmg"
