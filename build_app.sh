#!/bin/bash
# 编译并把客户端安装到 /Applications/WorkBuddy 账号切换.app
set -e
export PATH="$HOME/.cargo/bin:$PATH"
cd "$(dirname "$0")"

cargo build --release

APP="/Applications/WorkBuddy 账号切换.app"
mkdir -p "$APP/Contents/MacOS"
cp -f target/release/wbswitch-gui "$APP/Contents/MacOS/wbswitch-gui"
cp -f Info.plist "$APP/Contents/Info.plist"
chmod +x "$APP/Contents/MacOS/wbswitch-gui"

echo "✓ 已安装：$APP"
echo "  用 Spotlight 搜索「WorkBuddy 账号切换」即可打开"
