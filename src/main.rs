//! WorkBuddy 账号切换器 —— 图形界面客户端
//!
//! 功能：保存当前登录状态（名称自动生成，可编辑）/ 账号列表（可改名）/ 删除 / 切换 / 后悔药还原
//! 设计定调：工业实用 —— 紧凑密度、发丝边框、单一青绿强调色；红/琥珀仅用于状态语义

mod core;

use core::Profile;
use eframe::egui;

const CJK_FONT_CANDIDATES: [&str; 6] = [
    "/System/Library/Fonts/PingFang.ttc",
    "/System/Library/Fonts/STHeiti Light.ttc",
    "/System/Library/Fonts/STHeiti Medium.ttc",
    "/System/Library/Fonts/Hiragino Sans GB.ttc",
    "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
    "/Library/Fonts/Arial Unicode.ttf",
];

fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    for path in CJK_FONT_CANDIDATES {
        if let Ok(bytes) = std::fs::read(path) {
            if ttf_parser::Face::parse(&bytes, 0).is_ok() {
                fonts
                    .font_data
                    .insert("cjk".to_owned(), egui::FontData::from_owned(bytes).into());
                if let Some(f) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
                    f.push("cjk".to_owned());
                }
                if let Some(f) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
                    f.push("cjk".to_owned());
                }
                eprintln!("[wbswitch] 已加载中文字体：{}", path);
                break;
            }
        }
    }
    ctx.set_fonts(fonts);
}

// ------------------------------------------------------------------ 配色

/// 语义调色板：一套青绿主色 + 红/琥珀两个状态色，深浅色自适应
#[derive(Clone, Copy)]
struct Pal {
    accent: egui::Color32,       // 强调色（文字/描边）
    accent_solid: egui::Color32, // 强调色实心（主按钮底）
    accent_fill: egui::Color32,  // 强调色浅底（当前账号卡）
    danger: egui::Color32,
    danger_fill: egui::Color32,
    warn: egui::Color32,
    muted: egui::Color32,
    card: egui::Color32,
    border: egui::Color32,
    on_solid: egui::Color32, // 主按钮上的文字色
}

impl Pal {
    fn of(ui: &egui::Ui) -> Self {
        let dark = ui.visuals().dark_mode;
        if dark {
            Pal {
                accent: egui::Color32::from_rgb(0x5D, 0xCA, 0xA5),
                accent_solid: egui::Color32::from_rgb(0x1D, 0x9E, 0x75),
                accent_fill: egui::Color32::from_rgba_unmultiplied(0x1D, 0x9E, 0x75, 46),
                danger: egui::Color32::from_rgb(0xF0, 0x95, 0x95),
                danger_fill: egui::Color32::from_rgba_unmultiplied(0xE2, 0x4B, 0x4A, 46),
                warn: egui::Color32::from_rgb(0xFA, 0xC7, 0x75),
                muted: egui::Color32::from_rgb(0x9C, 0x98, 0x92),
                card: egui::Color32::from_rgba_unmultiplied(0xFF, 0xFF, 0xFF, 8),
                border: egui::Color32::from_rgba_unmultiplied(0xFF, 0xFF, 0xFF, 30),
                on_solid: egui::Color32::from_rgb(0x04, 0x15, 0x0F),
            }
        } else {
            Pal {
                accent: egui::Color32::from_rgb(0x0F, 0x6E, 0x56),
                accent_solid: egui::Color32::from_rgb(0x0F, 0x6E, 0x56),
                accent_fill: egui::Color32::from_rgb(0xE1, 0xF5, 0xEE),
                danger: egui::Color32::from_rgb(0xA3, 0x2D, 0x2D),
                danger_fill: egui::Color32::from_rgb(0xFC, 0xEB, 0xEB),
                warn: egui::Color32::from_rgb(0x85, 0x4F, 0x0B),
                muted: egui::Color32::from_rgb(0x6E, 0x6B, 0x64),
                card: egui::Color32::WHITE,
                border: egui::Color32::from_rgb(0xDC, 0xD9, 0xD2),
                on_solid: egui::Color32::WHITE,
            }
        }
    }
}

// ------------------------------------------------------------------ 状态

#[derive(Clone)]
struct Toast {
    text: String,
    is_error: bool,
    until: f64,
}

#[derive(Clone, Copy, PartialEq)]
enum PendingAction {
    Switch,
    RestoreLastGood,
    /// 「登录新账号」：仅本地清除凭证，不触发服务端注销
    LocalLogout,
}

#[derive(Clone)]
struct PendingSwitch {
    name: String,
    started: f64,
    quit_sent: bool,
    killed: bool,
    action: PendingAction,
}

#[derive(Clone)]
struct SwitchDialog {
    name: String,
    message: String,
}

struct App {
    profiles: Vec<Profile>,
    // 保存表单
    adding: bool,
    new_name: String,
    save_error: Option<String>,
    // 改名
    editing: Option<String>,
    rename_draft: String,
    // 弹窗与提示
    toast: Option<Toast>,
    confirm_delete: Option<String>,
    confirm_local_logout: bool,
    switch: Option<SwitchDialog>,
    pending: Option<PendingSwitch>,
    // 当前登录态
    current_uid: Option<String>,
    current_fp: Option<String>,
    current_session: usize,
    current_src: &'static str,
    last_good: Option<Profile>,
    snapshot_done: bool,
    auto_launch: bool,
    last_refresh: f64,
}

impl App {
    fn new() -> Self {
        let mut app = Self {
            profiles: vec![],
            adding: false,
            new_name: String::new(),
            save_error: None,
            editing: None,
            rename_draft: String::new(),
            toast: None,
            confirm_delete: None,
            confirm_local_logout: false,
            switch: None,
            pending: None,
            current_uid: None,
            current_fp: None,
            current_session: 0,
            current_src: "-",
            last_good: None,
            snapshot_done: false,
            auto_launch: true,
            last_refresh: 0.0,
        };
        app.refresh();
        app
    }

    fn refresh(&mut self) {
        self.profiles = core::list_profiles();
        self.current_uid = core::detect_uid();
        self.current_fp = core::current_fingerprint();
        self.current_session = core::current_session_len().unwrap_or(0);
        self.current_src = core::session_source_kind();
        self.last_good = core::last_good_info();
    }

    fn do_save(&mut self, ctx: &egui::Context) {
        match core::save_profile(self.new_name.trim()) {
            Ok(p) => {
                let tip = format!(
                    "已保存「{}」（登录票据 {}B）",
                    p.name,
                    p.session_len.unwrap_or(0)
                );
                self.set_toast(ctx, tip, false);
                self.adding = false;
                self.save_error = None;
                self.refresh();
            }
            Err(e) => {
                // 校验失败：错误留在表单里，不关面板
                self.save_error = Some(e);
            }
        }
    }

    fn do_switch(&mut self, ctx: &egui::Context, name: &str) {
        // 目标就是当前登录的这个账号：不退出 App、不还原、不重启，只给一句提示
        if core::is_current_profile(name) {
            self.set_toast(ctx, format!("「{}」就是当前登录的账号，无需切换", name), false);
            self.refresh();
            return;
        }
        match core::switch_to(name) {
            Ok(msg) => {
                self.set_toast(ctx, msg, false);
                self.refresh();
                if self.auto_launch {
                    if let Err(e) = core::launch_workbuddy() {
                        self.set_toast(ctx, format!("切换成功，但启动失败：{}", e), true);
                    }
                }
            }
            Err(e) => {
                if e.contains("正在运行") {
                    self.switch = Some(SwitchDialog {
                        name: name.to_string(),
                        message: e,
                    });
                } else {
                    // 校验失败（无票据等）：直接提示
                    self.set_toast(ctx, e, true);
                }
            }
        }
    }

    fn do_restore(&mut self, ctx: &egui::Context) {
        match core::restore_last_good() {
            Ok(msg) => {
                self.set_toast(ctx, msg, false);
                self.refresh();
                if self.auto_launch {
                    if let Err(e) = core::launch_workbuddy() {
                        self.set_toast(ctx, format!("还原成功，但启动失败：{}", e), true);
                    }
                }
            }
            Err(e) => self.set_toast(ctx, format!("还原失败：{}", e), true),
        }
    }

    fn do_local_logout(&mut self, ctx: &egui::Context) {
        match core::local_logout() {
            Ok(msg) => {
                self.refresh();
                // 「登录新账号」的目的就是打开登录页，所以无论是否勾选自动启动都要拉起
                match core::launch_workbuddy() {
                    Ok(()) => self.set_toast(ctx, format!("{}，已打开 WorkBuddy 登录页", msg), false),
                    Err(e) => {
                        self.set_toast(ctx, format!("{}，但启动失败：{}", msg, e), true)
                    }
                }
            }
            Err(e) => self.set_toast(ctx, format!("操作失败：{}", e), true),
        }
    }

    fn set_toast(&mut self, ctx: &egui::Context, text: impl Into<String>, is_error: bool) {
        let until = ctx.input(|i| i.time) + 4.0;
        self.toast = Some(Toast {
            text: text.into(),
            is_error,
            until,
        });
    }
}

// ------------------------------------------------------------------ 绘制

fn primary_btn(text: &str, pal: &Pal) -> egui::Button<'static> {
    egui::Button::new(egui::RichText::new(text.to_owned()).color(pal.on_solid))
        .fill(pal.accent_solid)
}

/// 幽灵按钮：描边 + 着色文字，比默认按钮醒目得多
fn ghost_btn(text: &str, color: egui::Color32, pal: &Pal) -> egui::Button<'static> {
    egui::Button::new(egui::RichText::new(text.to_owned()).color(color).strong())
        .fill(pal.card)
        .stroke(egui::Stroke::new(1.0, color))
}

fn card_frame(pal: &Pal, current: bool) -> egui::Frame {
    egui::Frame::new()
        .fill(if current { pal.accent_fill } else { pal.card })
        .stroke(egui::Stroke::new(0.5, pal.border))
        .corner_radius(egui::CornerRadius::same(8))
        .inner_margin(egui::Margin::symmetric(10, 7))
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);

        // —— 处理「点切换后自动等 WorkBuddy 退出」的挂起任务
        if let Some(p) = self.pending.clone() {
            if !core::wb_running() {
                let name = p.name.clone();
                let action = p.action;
                self.pending = None;
                match action {
                    PendingAction::RestoreLastGood => self.do_restore(&ctx),
                    PendingAction::LocalLogout => self.do_local_logout(&ctx),
                    PendingAction::Switch => self.do_switch(&ctx, &name),
                }
            } else if !p.quit_sent {
                let _ = core::quit_workbuddy();
                if let Some(pp) = self.pending.as_mut() {
                    pp.quit_sent = true;
                }
                ctx.request_repaint_after(std::time::Duration::from_millis(300));
            } else if now - p.started > 12.0 && !p.killed {
                for pid in core::wb_pids() {
                    let _ = std::process::Command::new("/bin/kill")
                        .args(["-9", &pid.to_string()])
                        .status();
                }
                if let Some(pp) = self.pending.as_mut() {
                    pp.killed = true;
                }
                ctx.request_repaint_after(std::time::Duration::from_millis(300));
            } else if now - p.started > 30.0 {
                self.pending = None;
                self.set_toast(&ctx, "WorkBuddy 无法自动退出，请手动 Cmd+Q 后再点切换", true);
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(300));
            }
        }

        // —— 每 3 秒刷新一次当前登录态
        if now - self.last_refresh > 3.0 {
            self.last_refresh = now;
            let uid = core::detect_uid();
            let fp = core::current_fingerprint();
            let sess = core::current_session_len().unwrap_or(0);
            if uid != self.current_uid || fp != self.current_fp || sess != self.current_session {
                self.current_uid = uid;
                self.current_fp = fp;
                self.current_session = sess;
                self.profiles = core::list_profiles();
                self.last_good = core::last_good_info();
            }
        }
        // —— 后台滚动留存「最近一次有效登录态」（后悔药）
        if !self.snapshot_done {
            self.snapshot_done = true;
            std::thread::spawn(|| {
                core::maybe_update_last_good();
            });
        }
        if let Some(t) = &self.toast {
            if now > t.until {
                self.toast = None;
            }
        }

        let mut to_switch: Option<String> = None;
        let mut to_delete: Option<String> = None;
        let mut do_refresh = false;
        let mut do_restore_last_good = false;
        let mut do_save = false;
        let mut rename_apply: Option<String> = None;

        egui::CentralPanel::default().show(ui, |ui| {
            let pal = Pal::of(ui);
            ui.add_space(10.0);

            // ══════════ 顶栏 ══════════
            ui.horizontal(|ui| {
                ui.heading("WorkBuddy 账号切换");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.checkbox(&mut self.auto_launch, egui::RichText::new("自动启动").strong());
                    if ui.add(ghost_btn("刷新", pal.accent, &pal)).clicked() {
                        do_refresh = true;
                    }
                });
            });

            ui.add_space(8.0);

            // ══════════ 当前登录状态条 ══════════
            egui::Frame::new()
                .fill(pal.card)
                .stroke(egui::Stroke::new(0.5, pal.border))
                .corner_radius(egui::CornerRadius::same(8))
                .inner_margin(egui::Margin::symmetric(10, 7))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let logged_in = self.current_session > 0;
                        let (status, color) = if logged_in {
                            ("已登录", pal.accent)
                        } else {
                            ("未登录", pal.danger)
                        };
                        ui.label(egui::RichText::new("●").color(color));
                        ui.label(
                            egui::RichText::new(status).color(color).strong().size(15.0),
                        );
                        ui.add_space(6.0);
                        let fp_short = self
                            .current_fp
                            .clone()
                            .map(|f| f[..8.min(f.len())].to_string())
                            .unwrap_or_else(|| "-".into());
                        ui.label(
                            egui::RichText::new(format!(
                                "uid {} · fp {} · 票据 {}",
                                self.current_uid
                                    .as_ref()
                                    .map(|u| u[..8.min(u.len())].to_string())
                                    .unwrap_or_else(|| "-".into()),
                                fp_short, self.current_src
                            ))
                            .monospace()
                            .size(12.0)
                            .color(pal.muted),
                        );
                        // 当前账号命中哪个档案：列表里已高亮，这里只在放得下时才补一句
                        if let Some(p) = self.profiles.iter().find(|p| p.is_current) {
                            let mut n = p.name.clone();
                            if n.chars().count() > 10 {
                                n = n.chars().take(9).collect::<String>() + "…";
                            }
                            ui.label(
                                egui::RichText::new(format!("＝ {}", n))
                                    .size(12.0)
                                    .color(pal.accent),
                            );
                        }

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .add(primary_btn("＋ 保存当前账号", &pal))
                                .clicked()
                            {
                                self.adding = !self.adding;
                                self.save_error = None;
                                if self.adding {
                                    // 名称自动生成：昵称(企业名)，可再手动编辑
                                    self.new_name = core::suggested_name();
                                }
                            }
                            if ui
                                .add(ghost_btn("登录新账号", pal.warn, &pal))
                                .clicked()
                            {
                                self.confirm_local_logout = true;
                            }
                        });
                    });
                });

            // ══════════ 后悔药横幅（当前没票据时） ══════════
            if self.current_session == 0 {
                ui.add_space(6.0);
                egui::Frame::new()
                    .fill(pal.danger_fill)
                    .corner_radius(egui::CornerRadius::same(8))
                    .inner_margin(egui::Margin::symmetric(10, 7))
                    .show(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(
                                egui::RichText::new("当前未登录").color(pal.danger).strong(),
                            );
                            match &self.last_good {
                                Some(g) => {
                                    ui.label(format!(
                                        "已留存 {} 的登录态（票据 {}B）",
                                        g.created_at,
                                        g.session_len.unwrap_or(0)
                                    ));
                                    if ui.add(primary_btn("↺ 恢复上次登录", &pal)).clicked() {
                                        do_restore_last_good = true;
                                    }
                                }
                                None => {
                                    ui.label(
                                        egui::RichText::new("没有可恢复的登录态，请重新登录。")
                                            .color(pal.muted),
                                    );
                                }
                            }
                        });
                    });
            }

            // ══════════ 等待退出进度 ══════════
            if let Some(p) = &self.pending {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new());
                    ui.colored_label(
                        pal.warn,
                        egui::RichText::new(format!(
                            "等待 WorkBuddy 退出… {} 秒",
                            (now - p.started) as i32
                        ))
                        .strong(),
                    );
                });
            }

            // ══════════ 保存表单 ══════════
            if self.adding {
                ui.add_space(6.0);
                egui::Frame::new()
                    .fill(pal.accent_fill)
                    .corner_radius(egui::CornerRadius::same(8))
                    .inner_margin(egui::Margin::symmetric(10, 8))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new("名称").strong());
                            let resp = ui.add(
                                egui::TextEdit::singleline(&mut self.new_name)
                                    .hint_text("如：张三(某某公司)")
                                    .desired_width(240.0),
                            );
                            // 回车即保存
                            if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                do_save = true;
                            }
                            if ui.add(primary_btn("保存", &pal)).clicked() {
                                do_save = true;
                            }
                            if ui.add(ghost_btn("取消", pal.muted, &pal)).clicked() {
                                self.adding = false;
                                self.save_error = None;
                            }
                        });
                        // 校验失败提示
                        if let Some(err) = &self.save_error {
                            ui.add_space(4.0);
                            ui.label(
                                egui::RichText::new(format!("⚠ {}", err))
                                    .color(pal.danger),
                            );
                        } else {
                            ui.label(
                                egui::RichText::new("名称已自动生成，可修改；保存无需退出 App")
                                    .color(pal.muted),
                            );
                        }
                    });
            }

            ui.add_space(10.0);

            // ══════════ 档案列表 ══════════
            ui.label(
                egui::RichText::new(format!("已保存的账号（{}）", self.profiles.len()))
                    .strong(),
            );
            ui.add_space(4.0);

            if self.profiles.is_empty() {
                ui.add_space(14.0);
                ui.label("还没有保存任何账号。");
                ui.add_space(2.0);
                ui.label(
                    egui::RichText::new("在 WorkBuddy 里登录后，点「保存当前账号」即可。")
                        .color(pal.muted),
                );
            }

            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    let snapshot: Vec<Profile> = self.profiles.clone();
                    for p in snapshot {
                        let is_editing = self.editing.as_deref() == Some(&p.name);
                        let mut switch_clicked = false;
                        let mut delete_clicked = false;
                        let mut rename_clicked = false;
                        let mut rename_ok = false;
                        let mut rename_cancel = false;

                        card_frame(&pal, p.is_current).show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.set_min_height(30.0);
                                // 状态点
                                ui.label(if p.is_current {
                                    egui::RichText::new("●").color(pal.accent)
                                } else {
                                    egui::RichText::new("○").color(pal.muted)
                                });
                                ui.add_space(4.0);

                                // 名称（或改名输入框）
                                if is_editing {
                                    let resp = ui.add(
                                        egui::TextEdit::singleline(&mut self.rename_draft)
                                            .desired_width(200.0),
                                    );
                                    if resp.lost_focus()
                                        && ui.input(|i| i.key_pressed(egui::Key::Enter))
                                    {
                                        rename_ok = true;
                                    }
                                    if resp.lost_focus()
                                        && ui.input(|i| i.key_pressed(egui::Key::Escape))
                                    {
                                        rename_cancel = true;
                                    }
                                    if ui.add(ghost_btn("✓", pal.accent, &pal)).clicked() {
                                        rename_ok = true;
                                    }
                                    if ui.add(ghost_btn("✗", pal.danger, &pal)).clicked() {
                                        rename_cancel = true;
                                    }
                                } else {
                                    let mut nm = p.name.clone();
                                    if nm.chars().count() > 16 {
                                        nm = nm.chars().take(15).collect::<String>() + "…";
                                    }
                                    ui.label(egui::RichText::new(nm).strong().size(15.0));
                                }

                                // 元信息（一行小字，不抢空间）
                                ui.add_space(8.0);
                                let size_txt = if p.size_kb >= 1024 {
                                    format!("{:.0}MB", p.size_kb as f64 / 1024.0)
                                } else {
                                    format!("{}KB", p.size_kb)
                                };
                                let meta = format!(
                                    "{} · {} · {}",
                                    p.uid
                                        .as_ref()
                                        .map(|u| u[..8.min(u.len())].to_string())
                                        .unwrap_or_else(|| "-".into()),
                                    match p.session_len {
                                        Some(n) if n > 0 => format!("票据{}B", n),
                                        _ => "无票据".into(),
                                    },
                                    size_txt
                                );
                                ui.label(egui::RichText::new(meta).size(12.0).color(pal.muted));

                                // 操作按钮
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui
                                            .add(ghost_btn("删除", pal.danger, &pal))
                                            .clicked()
                                        {
                                            delete_clicked = true;
                                        }
                                        if !is_editing
                                            && ui.add(ghost_btn("改名", pal.accent, &pal)).clicked()
                                        {
                                            rename_clicked = true;
                                        }
                                        if ui.add(primary_btn("切换", &pal)).clicked() {
                                            switch_clicked = true;
                                        }
                                    },
                                );
                            });
                        });
                        ui.add_space(4.0);

                        if switch_clicked {
                            to_switch = Some(p.name.clone());
                        }
                        if delete_clicked {
                            to_delete = Some(p.name.clone());
                        }
                        if rename_clicked {
                            self.editing = Some(p.name.clone());
                            self.rename_draft = p.name.clone();
                        }
                        if rename_cancel {
                            self.editing = None;
                        }
                        if rename_ok {
                            rename_apply = Some(p.name.clone());
                        }
                    }
                });

            // ══════════ 底栏 ══════════
            ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new("数据在 ~/.workbuddy-account-switch/，含登录凭据，勿外传")
                        .color(pal.muted),
                );
                ui.label(
                    egui::RichText::new(
                        "换账号点「切换」；登第二个号点「登录新账号」（不注销旧账号）",
                    )
                    .color(pal.warn),
                );
                if let Some(t) = &self.toast {
                    ui.add_space(4.0);
                    let color = if t.is_error { pal.danger } else { pal.accent };
                    egui::Frame::new()
                        .fill(if t.is_error { pal.danger_fill } else { pal.accent_fill })
                        .corner_radius(egui::CornerRadius::same(6))
                        .inner_margin(egui::Margin::symmetric(8, 5))
                        .show(ui, |ui| {
                            ui.label(egui::RichText::new(&t.text).color(color).strong());
                        });
                }
            });
        });

        // ══════════ 收尾动作 ══════════
        if do_refresh {
            self.refresh();
        }
        if do_save {
            self.do_save(&ctx);
        }
        if let Some(old) = rename_apply {
            let new = self.rename_draft.trim().to_string();
            self.editing = None;
            if new != old {
                match core::rename_profile(&old, &new) {
                    Ok(_) => {
                        self.set_toast(&ctx, format!("已改名为「{}」", new), false);
                        self.refresh();
                    }
                    Err(e) => self.set_toast(&ctx, format!("改名失败：{}", e), true),
                }
            }
        }
        if let Some(name) = to_delete {
            self.confirm_delete = Some(name);
        }
        if let Some(name) = to_switch {
            if core::wb_running() {
                self.pending = Some(PendingSwitch {
                    name,
                    started: now,
                    quit_sent: false,
                    killed: false,
                    action: PendingAction::Switch,
                });
                ctx.request_repaint_after(std::time::Duration::from_millis(300));
            } else {
                let n = name.clone();
                self.do_switch(&ctx, &n);
            }
        }
        if do_restore_last_good {
            if core::wb_running() {
                self.pending = Some(PendingSwitch {
                    name: "最近一次有效登录态".into(),
                    started: now,
                    quit_sent: false,
                    killed: false,
                    action: PendingAction::RestoreLastGood,
                });
                ctx.request_repaint_after(std::time::Duration::from_millis(300));
            } else {
                self.do_restore(&ctx);
            }
        }

        // ══════════ 删除确认弹窗 ══════════
        if let Some(name) = self.confirm_delete.clone() {
            let mut open = true;
            let mut do_delete = false;
            let mut do_close = false;
            egui::Window::new("确认删除")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .open(&mut open)
                .show(&ctx, |ui| {
                    let pal = Pal::of(ui);
                    ui.label(format!("确定删除档案「{}」？", name));
                    ui.label(
                        egui::RichText::new("删除后不可撤销（含其中的登录票据）。")
                            .color(pal.muted),
                    );
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new("删除").color(pal.on_solid).strong(),
                                )
                                .fill(pal.danger),
                            )
                            .clicked()
                        {
                            do_delete = true;
                        }
                        if ui.add(ghost_btn("取消", pal.accent, &pal)).clicked() {
                            do_close = true;
                        }
                    });
                });
            if do_close {
                open = false;
            }
            if do_delete || !open {
                if do_delete {
                    match core::delete_profile(&name) {
                        Ok(_) => {
                            self.set_toast(&ctx, format!("已删除「{}」", name), false);
                            self.refresh();
                        }
                        Err(e) => self.set_toast(&ctx, format!("删除失败：{}", e), true),
                    }
                }
                self.confirm_delete = None;
            }
        }

        // ══════════ 「登录新账号」确认弹窗 ══════════
        if self.confirm_local_logout {
            let mut open = true;
            let mut ok = false;
            let mut do_close = false;
            egui::Window::new("登录新账号")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .open(&mut open)
                .show(&ctx, |ui| {
                    let pal = Pal::of(ui);
                    ui.label("将清除本机登录信息并回到登录页，然后就能登录第二个账号。");
                    ui.label(
                        egui::RichText::new(
                            "只删本地凭证、不通知服务端——已保存的旧账号档案仍然有效。",
                        )
                        .color(pal.accent),
                    );
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.add(primary_btn("继续", &pal)).clicked() {
                            ok = true;
                        }
                        if ui.add(ghost_btn("取消", pal.muted, &pal)).clicked() {
                            do_close = true;
                        }
                    });
                });
            if do_close {
                open = false;
            }
            if ok {
                self.confirm_local_logout = false;
                if core::wb_running() {
                    self.pending = Some(PendingSwitch {
                        name: "登录新账号".into(),
                        started: now,
                        quit_sent: false,
                        killed: false,
                        action: PendingAction::LocalLogout,
                    });
                    ctx.request_repaint_after(std::time::Duration::from_millis(300));
                } else {
                    self.do_local_logout(&ctx);
                }
            } else if !open {
                self.confirm_local_logout = false;
            }
        }

        // ══════════ 切换受阻弹窗（WorkBuddy 还在运行） ══════════
        if let Some(dlg) = self.switch.clone() {
            let mut open = true;
            let mut retry = false;
            let mut quit = false;
            let mut do_close = false;
            egui::Window::new("暂时无法切换")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .open(&mut open)
                .show(&ctx, |ui| {
                    let pal = Pal::of(ui);
                    ui.label(&dlg.message);
                    ui.label("请在 WorkBuddy 中按 Cmd+Q 完全退出后再切换。");
                    ui.horizontal(|ui| {
                        if ui.add(primary_btn("帮我退出", &pal)).clicked() {
                            quit = true;
                        }
                        if ui.add(ghost_btn("重试", pal.accent, &pal)).clicked() {
                            retry = true;
                        }
                        if ui.add(ghost_btn("关闭", pal.muted, &pal)).clicked() {
                            do_close = true;
                        }
                    });
                });
            if do_close {
                open = false;
            }
            if quit {
                let _ = core::quit_workbuddy();
            }
            if retry && core::is_current_profile(&dlg.name) {
                self.set_toast(&ctx, format!("「{}」就是当前登录的账号，无需切换", dlg.name), false);
                self.refresh();
                self.switch = None;
            } else if retry {
                match core::switch_to(&dlg.name) {
                    Ok(msg) => {
                        self.set_toast(&ctx, msg, false);
                        self.refresh();
                        if self.auto_launch {
                            let _ = core::launch_workbuddy();
                        }
                        self.switch = None;
                    }
                    Err(e) => {
                        self.switch = Some(SwitchDialog {
                            name: dlg.name.clone(),
                            message: e,
                        });
                    }
                }
            }
            if !open && self.switch.is_some() && !retry {
                self.switch = None;
            }
        }
    }
}

/// 命令行自检：`wbswitch-gui --probe` 把票据探测链路整个打一遍然后退出。
/// 用来排查「App 里明明登录了，切换器却显示未登录」——不用开界面也能看清卡在哪一级。
fn probe() {
    println!("== 环境 ==");
    println!("HOME            = {}", core::home().display());
    println!("~/.workbuddy    = {}", core::wb_dir().display());
    println!("daemon 凭证目录 = {}", core::ext_auth_dir().display());

    println!("\n== 第一级：主 cookie 库 ==");
    let main = core::wb_dir().join("app/session/Cookies");
    println!("{} 存在={}", main.display(), main.exists());
    println!("session 票据长度 = {:?}", core::session_len_of(&main));

    println!("\n== 第二级：webview 分区 ==");
    let parts = core::wb_dir().join("app/session/Partitions");
    match std::fs::read_dir(&parts) {
        Ok(rd) => {
            for e in rd.flatten() {
                let p = e.path().join("Cookies");
                println!("  {} -> {:?}", e.file_name().to_string_lossy(), core::session_len_of(&p));
            }
        }
        Err(e) => println!("  读取失败：{}", e),
    }

    println!("\n== 第三级：daemon 凭证 ==");
    match std::fs::read_dir(core::ext_auth_dir()) {
        Ok(rd) => {
            for e in rd.flatten() {
                println!("  {} ({}B)", e.file_name().to_string_lossy(), e.metadata().map(|m| m.len()).unwrap_or(0));
            }
        }
        Err(e) => println!("  读取失败：{}", e),
    }
    println!("活凭证 = {:?}", core::live_cred_path());
    println!("凭证身份 = {:?}", core::current_daemon_cred());

    println!("\n== 汇总 ==");
    println!("票据长度 = {:?}", core::current_session_len());
    println!("票据来源 = {}", core::session_source_kind());
    println!("指纹     = {:?}", core::current_fingerprint());
    println!("uid      = {:?}", core::detect_uid());
    println!("\n== 档案 ==");
    for p in core::list_profiles() {
        println!(
            "  {}  票据={:?} uid={:?} 当前={}",
            p.name,
            p.session_len,
            p.uid.as_deref().map(|u| &u[..8.min(u.len())]),
            p.is_current
        );
    }
}

/// 命令行覆盖式保存：`wbswitch-gui --save "<档案名>"`
/// （同名先删；依赖 daemon 凭证刷新后及时更新档案里的票据，否则可能留下已轮换的旧 token）
fn cli_save(name: &str) -> Result<(), String> {
    let _ = core::delete_profile(name);
    let p = core::save_profile(name)?;
    println!(
        "已保存「{}」 uid={:?} 票据={:?} 来源={}",
        p.name,
        p.uid,
        p.session_len,
        core::session_source_kind()
    );
    Ok(())
}

fn main() -> eframe::Result<()> {
    let argv: Vec<String> = std::env::args().collect();
    if argv.iter().any(|a| a == "--probe") {
        probe();
        return Ok(());
    }
    if let Some(i) = argv.iter().position(|a| a == "--save") {
        let name = argv.get(i + 1).cloned().unwrap_or_default();
        if name.is_empty() {
            eprintln!("用法：wbswitch-gui --save \"<档案名>\"");
            std::process::exit(2);
        }
        match cli_save(&name) {
            Ok(()) => return Ok(()),
            Err(e) => {
                eprintln!("保存失败：{}", e);
                std::process::exit(1);
            }
        }
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([720.0, 470.0])
            .with_min_inner_size([640.0, 400.0]),
        ..Default::default()
    };
    eframe::run_native(
        "WorkBuddy 账号切换器",
        options,
        Box::new(|cc| {
            setup_fonts(&cc.egui_ctx);
            // 全局字号调大：正文/按钮 15，标题 20，小字 12.5
            let apply_style = |s: &mut egui::Style| {
                use egui::{FontFamily, FontId, TextStyle};
                s.text_styles
                    .insert(TextStyle::Heading, FontId::new(20.0, FontFamily::Proportional));
                s.text_styles
                    .insert(TextStyle::Body, FontId::new(15.0, FontFamily::Proportional));
                s.text_styles
                    .insert(TextStyle::Button, FontId::new(15.0, FontFamily::Proportional));
                s.text_styles
                    .insert(TextStyle::Small, FontId::new(12.5, FontFamily::Proportional));
                s.text_styles
                    .insert(TextStyle::Monospace, FontId::new(13.0, FontFamily::Monospace));
                s.spacing.item_spacing = egui::vec2(8.0, 6.0);
            };
            cc.egui_ctx.style_mut_of(egui::Theme::Light, apply_style);
            cc.egui_ctx.style_mut_of(egui::Theme::Dark, apply_style);
            let app: Box<dyn eframe::App> = Box::new(App::new());
            Ok(app)
        }),
    )
}
