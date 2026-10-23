#!/bin/bash
# 编译并把客户端安装到 /Applications/WorkBuddy 账号切换.app
set -e
export PATH="$HOME/.cargo/bin:$PATH"
cd "$(dirname "$0")"

cargo build --release

APP="/Applications/WorkBuddy 账号切换.app"

# 这个 App 不响应 AppleScript 的 quit，必须先强杀旧实例，
# 否则覆盖二进制后运行中的仍是旧代码（改了界面也不生效）
/usr/bin/pkill -9 -f "WorkBuddy 账号切换.app/Contents/MacOS/wbswitch-gui" 2>/dev/null
sleep 1

mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp -f target/release/wbswitch-gui "$APP/Contents/MacOS/wbswitch-gui"
cp -f Info.plist "$APP/Contents/Info.plist"
if [ -f assets/AppIcon.icns ]; then
  cp -f assets/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"
fi
chmod +x "$APP/Contents/MacOS/wbswitch-gui"
# 让 Finder/Dock 立刻刷新图标
touch "$APP"

echo "✓ 已安装：$APP"
echo "  Spotlight 搜索「账号管家」即可打开"
# 刷新 LaunchServices，保证 Dock 显示新名字与新图标
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "$APP" >/dev/null 2>&1 || true
killall Dock >/dev/null 2>&1 || true
