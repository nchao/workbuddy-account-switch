#!/bin/bash
# 编译并把客户端安装到 /Applications/WbAccSwitch.app
#
#   ./build_app.sh              本机架构编译 + 安装
#   ./build_app.sh zip          安装 + 打包 dist/ 下的 zip
#   ./build_app.sh universal    x86_64 + arm64 合成通用二进制后安装（发版用）
#   ./build_app.sh universal zip  （等价：universal 已自带 zip）
set -e
export PATH="$HOME/.cargo/bin:$PATH"
cd "$(dirname "$0")"

MODE="${1:-}"
APP="/Applications/WbAccSwitch.app"
OLD="/Applications/WorkBuddy 账号切换.app"

if [ "$MODE" = "universal" ]; then
  rustup target add aarch64-apple-darwin >/dev/null 2>&1 || true
  cargo build --release --target x86_64-apple-darwin
  cargo build --release --target aarch64-apple-darwin
  mkdir -p target/universal
  lipo -create -output target/universal/wbswitch-gui \
    target/x86_64-apple-darwin/release/wbswitch-gui \
    target/aarch64-apple-darwin/release/wbswitch-gui
  BIN="target/universal/wbswitch-gui"
  ZIPNAME="WbAccSwitch-macos-universal.zip"
  DO_ZIP=1
else
  cargo build --release
  BIN="target/release/wbswitch-gui"
  ZIPNAME="WbAccSwitch-macos.zip"
  DO_ZIP=0
  [ "$MODE" = "zip" ] && DO_ZIP=1
fi

# 这个 App 不响应 AppleScript 的 quit，必须先强杀旧实例，
# 否则覆盖二进制后运行中的仍是旧代码（改了界面也不生效）
/usr/bin/pkill -9 -f "Contents/MacOS/wbswitch-gui" 2>/dev/null || true
sleep 1

# 老安装路径迁移过来，避免 /Applications 里留两份
if [ -d "$OLD" ] && [ ! -d "$APP" ]; then
  mv "$OLD" "$APP"
fi

mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp -f "$BIN" "$APP/Contents/MacOS/wbswitch-gui"
cp -f Info.plist "$APP/Contents/Info.plist"
if [ -f assets/AppIcon.icns ]; then
  cp -f assets/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"
fi
chmod +x "$APP/Contents/MacOS/wbswitch-gui"
# 通用二进制必须签名（ad-hoc 即可），否则 arm64 原生可能被 Gatekeeper 拦下
codesign --force --deep --sign - "$APP" >/dev/null 2>&1 || true

# 刷新 LaunchServices（新名字/新图标立刻生效）并重启 Dock
touch "$APP"
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "$APP" >/dev/null 2>&1 || true
killall Dock >/dev/null 2>&1 || true

echo "✓ 已安装：$APP  ($(file -b "$APP/Contents/MacOS/wbswitch-gui" | cut -c1-70))"
echo "  Spotlight 搜索「WbAccSwitch」即可打开"

if [ "$DO_ZIP" = "1" ]; then
  mkdir -p dist
  rm -f "dist/$ZIPNAME"
  ditto -c -k --keepParent "$APP" "dist/$ZIPNAME"
  echo "✓ 已打包：dist/$ZIPNAME ($(du -h "dist/$ZIPNAME" | cut -f1))"
fi
