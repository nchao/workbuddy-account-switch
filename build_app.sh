#!/bin/bash
# 编译并把客户端安装到 /Applications/WbAccSwitch.app
# 可选：./build_app.sh zip   —— 额外打包出 dist/WbAccSwitch-macos.zip（用于发 Release）
set -e
export PATH="$HOME/.cargo/bin:$PATH"
cd "$(dirname "$0")"

cargo build --release

APP="/Applications/WbAccSwitch.app"
OLD="/Applications/WorkBuddy 账号切换.app"

# 这个 App 不响应 AppleScript 的 quit，必须先强杀旧实例，
# 否则覆盖二进制后运行中的仍是旧代码（改了界面也不生效）
/usr/bin/pkill -9 -f "Contents/MacOS/wbswitch-gui" 2>/dev/null || true
sleep 1

# 老安装路径迁移过来，避免 /Applications 里留两份
if [ -d "$OLD" ] && [ ! -d "$APP" ]; then
  mv "$OLD" "$APP"
fi

mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp -f target/release/wbswitch-gui "$APP/Contents/MacOS/wbswitch-gui"
cp -f Info.plist "$APP/Contents/Info.plist"
if [ -f assets/AppIcon.icns ]; then
  cp -f assets/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"
fi
chmod +x "$APP/Contents/MacOS/wbswitch-gui"

# 刷新 LaunchServices（新名字/新图标立刻生效）并重启 Dock
touch "$APP"
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "$APP" >/dev/null 2>&1 || true
killall Dock >/dev/null 2>&1 || true

echo "✓ 已安装：$APP"
echo "  Spotlight 搜索「WbAccSwitch」即可打开"

if [ "${1:-}" = "zip" ]; then
  mkdir -p dist
  rm -f dist/WbAccSwitch-macos.zip
  ditto -c -k --keepParent "$APP" dist/WbAccSwitch-macos.zip
  echo "✓ 已打包：dist/WbAccSwitch-macos.zip ($(du -h dist/WbAccSwitch-macos.zip | cut -f1))"
fi
