# WorkBuddy 账号切换器

一个 macOS 小工具：把 WorkBuddy 每个账号的登录态存成快照，换账号时一键还原，不用重复登录。

适合「公司版和个人版用同一个手机号」这种场景——每次切账号都要走一遍登录，
而这个工具只需要每个账号**成功登录一次并保存**，之后就可以来回复用。

> 仅在本机操作文件，不联网、不上传、不修改服务端任何状态。
> 快照里含**真实登录凭据**，请勿外传（默认权限 700/600）。

---

## 它解决什么问题

WorkBuddy 的登录态散落在好几个地方，而且**不同类型账号的载体还不一样**：

| 载体 | 位置 | 谁在用 |
| --- | --- | --- |
| Web session cookie | `~/.workbuddy/app/session/Cookies`（SQLite） | 企业账号有 |
| Local / Session Storage、WebStorage | `~/.workbuddy/app/session/` | 都有 |
| 账号状态目录 | `~/.workbuddy/<uid>/`、`storage/user-*`、`app/users` | 都有 |
| **daemon 凭证** | `~/Library/Application Support/CodeBuddyExtension/Data/Public/auth/workbuddy-desktop.info` | 都有 |

关键坑：**个人账号登录时，cookie 库里根本没有 session cookie**
（只有 `tgw_l7_route` 之类的无关 cookie），票据只写在 daemon 凭证里；
企业账号则是 cookie + daemon 两处都有。所以票据检测必须做
「主 cookie 库 → webview 分区 → daemon 凭证」三级兜底，只查 cookie 会误判成「未登录」。

---

## 功能

| 功能 | 说明 |
| --- | --- |
| 保存当前账号 | 自动生成名字（`昵称(企业名)` / `昵称(个人版)`），可改；WorkBuddy 开着也能存 |
| 账号列表 | 显示名字、uid、票据大小、保存时间；当前登录的那个会高亮 |
| 切换 | 自动让 WorkBuddy 退干净（含 daemon）→ 还原快照 → 再拉起来 |
| 切到当前账号 | **不会做任何动作**，只提示「就是当前登录的账号」，不会白退一次 App |
| 按账号去重 | 同一个 uid 只允许存一份，重复保存会被拒绝并点名已有档案 |
| 登录新账号 | 只清本机凭据，不碰服务端，然后打开登录页 |
| 后悔药 | `last-good` 滚动保留最近一次有效登录态，随时可回滚 |

---

## 使用

```bash
./build_app.sh          # 编译并安装到 /Applications/WorkBuddy 账号切换.app
```

第一次用：

1. 登录账号 A → 打开切换器 → 「＋ 保存当前账号」
2. 切到账号 B（走一次正常登录）→ 再保存一次
3. 之后换号只需在列表里点「切换」

命令行排查（不需要开界面）：

```bash
"/Applications/WorkBuddy 账号切换.app/Contents/MacOS/wbswitch-gui" --probe
"/Applications/WorkBuddy 账号切换.app/Contents/MacOS/wbswitch-gui" --save "档案名"
```

`--probe` 会把三级票据探测链路整条打出来，用于排查
「App 里明明登录着，切换器却显示未登录」这类问题。

---

## 已知边界

1. **切换必须让 WorkBuddy 完全退出**。认证由 daemon sidecar 持有，
   运行中替换文件会被内存里的状态写回覆盖。
2. **不要在 WorkBuddy 里点「退出账号」再指望快照能救回来**。那会走服务端注销，
   票据被吊销后，本机文件还在也登不进去。正确姿势是直接 Cmd+Q。
3. daemon 凭证里的 token 是**本机加密**的，快照只能在同一台机器上用，拷到别的机器无效。
4. 若服务端做了设备绑定，恢复旧快照后仍可能要再登录一次，重新保存覆盖即可。
5. 只支持 macOS（用到 `/usr/bin/open`、`pgrep` 等）。

---

## 构建

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo build --release                   # 编译
cargo test --release -- --test-threads=1   # 自检（9 项）
./build_app.sh                          # 打包成 .app 并装到 /Applications
```

依赖：eframe(egui) 0.37、rusqlite(bundled)、serde、sha2、ttf-parser。
中文字体在启动时从系统字体（PingFang / STHeiti / Hiragino / Arial Unicode）里挑第一个可用的加载。

## 数据

```
~/.workbuddy-account-switch/
├── profiles/<档案名>/{files/, files-ext/auth/, manifest.json}
├── backups/auto-<时间戳>/          # 每次切换前自动备份
└── last-good/                      # 最近一次有效登录态
```

`manifest.json` 里只记 `uid`、票据长度和**哈希指纹**，不存明文票据；
真正的凭据在 `files/` 和 `files-ext/auth/` 里，请当作密码对待。

## License

MIT
