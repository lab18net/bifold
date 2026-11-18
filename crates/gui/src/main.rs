//! Bifold desktop front end (egui/eframe).
//!
//! Same [`Session`] as the terminal build; only the rendering differs.

use std::path::PathBuf;
use std::time::Duration;

use bifold_core::config::Config;
use bifold_core::error::Result;
use bifold_core::format::{ellipsis_left, ellipsis_right, human_size, short_time};
use bifold_core::fsops::{self, DeleteMode};
use bifold_core::job::Phase;
use bifold_core::preview::{self, Preview};
use bifold_core::session::{LEFT, RIGHT, Session};
use bifold_core::{APP_NAME, VERSION};
use eframe::egui;

const ACCENT: egui::Color32 = egui::Color32::from_rgb(96, 208, 192);
const ACCENT_BG: egui::Color32 = egui::Color32::from_rgb(28, 58, 56);
const TEXT: egui::Color32 = egui::Color32::from_rgb(222, 226, 230);
const DIM: egui::Color32 = egui::Color32::from_rgb(132, 138, 144);
const FAINT: egui::Color32 = egui::Color32::from_rgb(96, 102, 108);
const DIR: egui::Color32 = egui::Color32::from_rgb(122, 172, 255);
const LINK: egui::Color32 = egui::Color32::from_rgb(186, 146, 232);
const MARK: egui::Color32 = egui::Color32::from_rgb(232, 200, 122);
const MARK_BG: egui::Color32 = egui::Color32::from_rgb(44, 40, 26);
const ERROR: egui::Color32 = egui::Color32::from_rgb(232, 112, 112);

const ROW_HEIGHT: f32 = 21.0;

fn main() -> eframe::Result {
    let args = match parse_args() {
        Ok(Some(args)) => args,
        Ok(None) => return Ok(()),
        Err(message) => {
            eprintln!("{APP_NAME}: {message}");
            std::process::exit(2);
        }
    };

    let mut config = if args.no_config {
        Config::default()
    } else {
        Config::load()
    };
    if let Some(path) = &args.path {
        let text = path.display().to_string();
        config.left = text.clone();
        config.right = text;
    }
    if let Some(left) = &args.left {
        config.left = left.display().to_string();
    }
    if let Some(right) = &args.right {
        config.right = right.display().to_string();
    }

    let no_config = args.no_config;
    let session = Session::new(config);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1240.0, 780.0])
            .with_min_inner_size([820.0, 480.0])
            .with_title(format!("{APP_NAME} {VERSION}")),
        ..Default::default()
    };

    eframe::run_native(
        APP_NAME,
        options,
        Box::new(move |cc| {
            cc.egui_ctx.set_theme(egui::ThemePreference::Dark);
            let mut app = GuiApp::new(session);
            app.persist = !no_config;
            Ok(Box::new(app))
        }),
    )
}

#[derive(Debug, Default)]
struct Args {
    left: Option<PathBuf>,
    right: Option<PathBuf>,
    path: Option<PathBuf>,
    no_config: bool,
}

fn parse_args() -> std::result::Result<Option<Args>, String> {
    let mut args = Args::default();
    let mut raw = std::env::args().skip(1);
    while let Some(arg) = raw.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print!(
                    "{APP_NAME} {VERSION} — two-pane file manager (desktop)\n\n\
                     USAGE: bifold-gui [PATH] [--left DIR] [--right DIR] [--path DIR]\n"
                );
                return Ok(None);
            }
            "-V" | "--version" => {
                println!("{APP_NAME} {VERSION}");
                return Ok(None);
            }
            "--no-config" => args.no_config = true,
            "-l" | "--left" => args.left = Some(take_path(&mut raw, "--left")?),
            "-r" | "--right" => args.right = Some(take_path(&mut raw, "--right")?),
            "-p" | "--path" => args.path = Some(take_path(&mut raw, "--path")?),
            other if other.starts_with('-') => {
                return Err(format!("unknown option: {other} (try --help)"));
            }
            positional => args.path = Some(PathBuf::from(positional)),
        }
    }
    Ok(Some(args))
}

fn take_path(
    raw: &mut impl Iterator<Item = String>,
    flag: &str,
) -> std::result::Result<PathBuf, String> {
    raw.next()
        .map(PathBuf::from)
        .ok_or_else(|| format!("{flag} needs a directory"))
}

// ---------------------------------------------------------------------------
// Dialogs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum DialogKind {
    MkDir,
    NewFile,
    Rename,
    CopyTo,
    MoveTo,
    GoTo,
    Filter,
    Delete {
        targets: Vec<PathBuf>,
        mode: DeleteMode,
    },
}

impl DialogKind {
    fn title(&self) -> String {
        match self {
            Self::MkDir => "New folder".into(),
            Self::NewFile => "New file".into(),
            Self::Rename => "Rename".into(),
            Self::CopyTo => "Copy to".into(),
            Self::MoveTo => "Move to".into(),
            Self::GoTo => "Go to directory".into(),
            Self::Filter => "Filter".into(),
            Self::Delete { targets, mode } => format!(
                "{} {} item(s)",
                if *mode == DeleteMode::Permanent {
                    "Delete"
                } else {
                    "Trash"
                },
                targets.len()
            ),
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Self::MkDir | Self::NewFile | Self::Rename => "name",
            Self::CopyTo | Self::MoveTo => "destination",
            Self::GoTo => "path",
            Self::Filter => "pattern",
            Self::Delete { .. } => "",
        }
    }

    fn needs_text(&self) -> bool {
        !matches!(self, Self::Delete { .. })
    }
}

#[derive(Debug, Clone)]
struct Dialog {
    kind: DialogKind,
    text: String,
}

#[derive(Debug, Clone, Copy)]
enum PaneAction {
    Activate,
    Focus(usize),
    Toggle(usize),
    Open(usize),
    Up,
    Reload,
    CycleSort,
    ToggleSortDir,
}

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

struct GuiApp {
    session: Session,
    dialog: Option<Dialog>,
    viewer: Option<Preview>,
    reveal: [bool; 2],
    /// False when started with `--no-config`.
    persist: bool,
}

impl GuiApp {
    fn new(session: Session) -> Self {
        Self {
            session,
            dialog: None,
            viewer: None,
            reveal: [false; 2],
            persist: true,
        }
    }

    fn ui(&mut self, ctx: &egui::Context) {
        if self.session.jobs.is_busy() {
            ctx.request_repaint_after(Duration::from_millis(80));
        }
        if self.session.poll_jobs().is_some() {
            self.reveal = [true; 2];
        }

        self.shortcuts(ctx);

        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| self.toolbar(ui));
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| self.status_bar(ui));
        egui::SidePanel::left("pane_left")
            .resizable(true)
            .default_width((ctx.content_rect().width() * 0.5).max(260.0))
            .min_width(200.0)
            .show(ctx, |ui| self.pane(ui, LEFT));
        egui::CentralPanel::default().show(ctx, |ui| self.pane(ui, RIGHT));

        self.show_dialog(ctx);
        self.show_viewer(ctx);
    }

    // -- toolbar ------------------------------------------------------------

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        let busy = self.session.jobs.is_busy();
        ui.spacing_mut().item_spacing = egui::vec2(6.0, 4.0);
        ui.horizontal_wrapped(|ui| {
            let targets = self.session.pane().targets().len();
            ui.label(
                egui::RichText::new(if targets > 1 {
                    format!("{targets} items")
                } else {
                    "1 item".to_string()
                })
                .size(11.0)
                .color(DIM),
            );
            ui.separator();

            if ui
                .add_enabled(!busy, egui::Button::new("Copy →").small())
                .on_hover_text("F5 — copy the selection to the other pane")
                .clicked()
            {
                self.begin_transfer(DialogKind::CopyTo);
            }
            if ui
                .add_enabled(!busy, egui::Button::new("Move →").small())
                .on_hover_text("F6 — move the selection to the other pane")
                .clicked()
            {
                self.begin_transfer(DialogKind::MoveTo);
            }
            if ui
                .add_enabled(!busy, egui::Button::new("Rename").small())
                .on_hover_text("F2")
                .clicked()
            {
                self.begin_rename();
            }
            if ui
                .add_enabled(!busy, egui::Button::new("New folder").small())
                .on_hover_text("F7")
                .clicked()
            {
                self.dialog = Some(Dialog {
                    kind: DialogKind::MkDir,
                    text: String::new(),
                });
            }
            if ui
                .add_enabled(!busy, egui::Button::new("New file").small())
                .clicked()
            {
                self.dialog = Some(Dialog {
                    kind: DialogKind::NewFile,
                    text: String::new(),
                });
            }
            if ui
                .add_enabled(!busy, egui::Button::new("Delete").small())
                .on_hover_text("F8")
                .clicked()
            {
                self.begin_delete();
            }
            if ui
                .add_enabled(!busy, egui::Button::new("View").small())
                .on_hover_text("F3")
                .clicked()
            {
                self.view_cursor();
            }
            ui.separator();

            let side = self.session.active;
            let mut show_hidden = self.session.panes[side].show_hidden;
            if ui.checkbox(&mut show_hidden, "hidden").changed() {
                self.session.panes[side].toggle_hidden();
            }
            let sort_label = format!(
                "sort: {} {}",
                self.session.panes[side].sort_key.label(),
                self.session.panes[side].sort_dir.arrow()
            );
            let sort_resp = ui
                .small_button(sort_label)
                .on_hover_text("click: sort key · right-click: order");
            if sort_resp.clicked() {
                self.apply_action(side, PaneAction::CycleSort);
            }
            if sort_resp.secondary_clicked() {
                self.apply_action(side, PaneAction::ToggleSortDir);
            }

            let mut filter = self.session.panes[side].filter.clone();
            let width = (ui.available_width() - 70.0).clamp(90.0, 220.0);
            if ui
                .add(
                    egui::TextEdit::singleline(&mut filter)
                        .hint_text("filter")
                        .desired_width(width),
                )
                .changed()
            {
                self.session.panes[side].set_filter(filter);
            }
            if ui.small_button("✕").on_hover_text("clear filter").clicked() {
                self.session.panes[side].set_filter("");
            }
        });
    }

    // -- panes --------------------------------------------------------------

    fn pane(&mut self, ui: &mut egui::Ui, side: usize) {
        let active = self.session.active == side;
        let mut action: Option<PaneAction> = None;

        ui.horizontal(|ui| {
            let tag_text = if side == LEFT { " LEFT " } else { " RIGHT " };
            let tag = egui::Button::new(
                egui::RichText::new(tag_text)
                    .monospace()
                    .size(11.0)
                    .strong()
                    .color(if active { egui::Color32::BLACK } else { DIM }),
            )
            .fill(if active {
                ACCENT
            } else {
                egui::Color32::TRANSPARENT
            })
            .small();
            if ui.add(tag).clicked() {
                action = Some(PaneAction::Activate);
            }

            let path = ellipsis_left(&self.session.panes[side].path.display().to_string(), 52);
            ui.label(
                egui::RichText::new(path)
                    .monospace()
                    .size(12.0)
                    .color(if active { TEXT } else { DIM }),
            );

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("⟳").on_hover_text("reload").clicked() {
                    action = Some(PaneAction::Reload);
                }
                if ui
                    .small_button("↑")
                    .on_hover_text("parent directory")
                    .clicked()
                {
                    action = Some(PaneAction::Up);
                }
            });
        });

        let header_rect = ui
            .allocate_exact_size(egui::vec2(ui.available_width(), 15.0), egui::Sense::hover())
            .0;
        let painter = ui.painter();
        let mono_small = egui::FontId::monospace(10.0);
        painter.text(
            header_rect.left_center() + egui::vec2(20.0, 0.0),
            egui::Align2::LEFT_CENTER,
            "name",
            mono_small.clone(),
            FAINT,
        );
        painter.text(
            header_rect.right_center() - egui::vec2(126.0, 0.0),
            egui::Align2::RIGHT_CENTER,
            "size",
            mono_small.clone(),
            FAINT,
        );
        painter.text(
            header_rect.right_center() - egui::vec2(6.0, 0.0),
            egui::Align2::RIGHT_CENTER,
            "modified",
            mono_small,
            FAINT,
        );

        let reveal = std::mem::take(&mut self.reveal[side]);
        let pane = &self.session.panes[side];

        let mut scroll = egui::ScrollArea::vertical()
            .id_salt(("rows", side))
            .auto_shrink([false, false]);
        if reveal {
            let offset = (pane.cursor as f32 * ROW_HEIGHT - 80.0).max(0.0);
            scroll = scroll.vertical_scroll_offset(offset);
        }

        scroll.show_rows(ui, ROW_HEIGHT, pane.entries.len(), |ui, range| {
            for index in range {
                let Some(entry) = pane.entries.get(index) else {
                    continue;
                };
                let (rect, response) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), ROW_HEIGHT),
                    egui::Sense::click(),
                );
                let is_cursor = index == pane.cursor;
                let marked = pane.selected.contains(&entry.path);

                if is_cursor {
                    ui.painter().rect_filled(
                        rect,
                        3.0,
                        if active {
                            ACCENT_BG
                        } else {
                            egui::Color32::from_rgb(48, 50, 54)
                        },
                    );
                } else if marked {
                    ui.painter().rect_filled(rect, 3.0, MARK_BG);
                }

                if marked {
                    ui.painter().circle_filled(
                        rect.left_center() + egui::vec2(8.0, 0.0),
                        2.5,
                        MARK,
                    );
                }

                let name_color = if entry.is_dir() {
                    DIR
                } else if entry.symlink {
                    LINK
                } else if entry.hidden() {
                    FAINT
                } else {
                    TEXT
                };
                let name = ellipsis_right(&entry.name, 96);
                ui.painter().text(
                    rect.left_center() + egui::vec2(20.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    name,
                    egui::FontId::monospace(13.0),
                    name_color,
                );

                let size = if entry.is_dir() {
                    "<dir>".to_string()
                } else {
                    human_size(entry.size)
                };
                ui.painter().text(
                    rect.right_center() - egui::vec2(126.0, 0.0),
                    egui::Align2::RIGHT_CENTER,
                    size,
                    egui::FontId::monospace(11.0),
                    DIM,
                );
                let time = entry.modified.map(short_time).unwrap_or_default();
                ui.painter().text(
                    rect.right_center() - egui::vec2(6.0, 0.0),
                    egui::Align2::RIGHT_CENTER,
                    time,
                    egui::FontId::monospace(11.0),
                    DIM,
                );

                if response.double_clicked() {
                    action = Some(PaneAction::Open(index));
                } else if response.clicked() {
                    let toggle = ui.input(|i| i.modifiers.command || i.modifiers.shift);
                    action = Some(if toggle {
                        PaneAction::Toggle(index)
                    } else {
                        PaneAction::Focus(index)
                    });
                }
            }
        });

        if let Some(action) = action {
            self.apply_action(side, action);
        }
    }

    fn apply_action(&mut self, side: usize, action: PaneAction) {
        self.session.active = side;
        match action {
            PaneAction::Activate => {}
            PaneAction::Focus(index) => {
                self.session.panes[side].cursor = index;
            }
            PaneAction::Toggle(index) => {
                self.session.panes[side].cursor = index;
                self.session.panes[side].toggle_select();
            }
            PaneAction::Open(index) => {
                self.session.panes[side].cursor = index;
                self.open_cursor();
            }
            PaneAction::Up => {
                if self.session.panes[side].up() {
                    self.reveal[side] = true;
                }
            }
            PaneAction::Reload => self.session.panes[side].reload(),
            PaneAction::CycleSort => self.session.panes[side].cycle_sort(),
            PaneAction::ToggleSortDir => self.session.panes[side].toggle_sort_dir(),
        }
    }

    // -- status bar ---------------------------------------------------------

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let pane = self.session.pane();
            ui.label(
                egui::RichText::new(ellipsis_left(&pane.path.display().to_string(), 46))
                    .monospace()
                    .size(11.0)
                    .color(DIM),
            );
            ui.separator();
            let status = self.session.status();
            let color = if status.starts_with("error") {
                ERROR
            } else {
                TEXT
            };
            ui.label(
                egui::RichText::new(ellipsis_right(&status, 110))
                    .size(11.0)
                    .color(color),
            );
        });

        if let Some(progress) = self.session.job_progress() {
            ui.horizontal(|ui| {
                if progress.phase.is_finished() {
                    let color = if progress.phase == Phase::Failed {
                        ERROR
                    } else {
                        DIM
                    };
                    ui.label(
                        egui::RichText::new(ellipsis_right(&progress.headline(), 120))
                            .size(11.0)
                            .color(color),
                    );
                } else {
                    ui.add(
                        egui::ProgressBar::new(progress.ratio())
                            .desired_width(240.0)
                            .show_percentage(),
                    );
                    ui.label(
                        egui::RichText::new(ellipsis_right(&progress.headline(), 96))
                            .size(11.0)
                            .color(TEXT),
                    );
                    if ui.small_button("Cancel").clicked() {
                        self.session.cancel_job();
                    }
                }
            });
        }
    }

    // -- dialogs ------------------------------------------------------------

    fn show_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.dialog.take() else {
            return;
        };
        let mut submit = false;
        let mut cancel = false;
        let title = dialog.kind.title();

        egui::Window::new(title)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.set_min_width(420.0);
                if dialog.kind.needs_text() {
                    ui.label(
                        egui::RichText::new(dialog.kind.label())
                            .size(11.0)
                            .color(DIM),
                    );
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut dialog.text).desired_width(f32::INFINITY),
                    );
                    response.request_focus();
                    if ui.input(|i| i.key_pressed(egui::Key::Enter))
                        && (response.has_focus() || response.lost_focus())
                    {
                        submit = true;
                    }
                } else if let DialogKind::Delete { targets, mode } = &dialog.kind {
                    let where_to = if *mode == DeleteMode::Permanent {
                        "Permanently delete"
                    } else {
                        "Move to the trash"
                    };
                    ui.label(format!("{where_to} {} item(s)?", targets.len()));
                    ui.label(
                        egui::RichText::new("Reversible only when trashing.")
                            .size(11.0)
                            .color(FAINT),
                    );
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("OK").clicked() {
                        submit = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    cancel = true;
                }
            });

        if submit {
            self.submit_dialog(dialog);
        } else if !cancel {
            self.dialog = Some(dialog);
        }
    }

    fn submit_dialog(&mut self, dialog: Dialog) {
        let side = self.session.active;
        let value = dialog.text.trim().to_string();
        let kind = dialog.kind.clone();

        let result: Result<()> = match kind.clone() {
            DialogKind::MkDir => self.session.make_dir(&value).map(|_| ()),
            DialogKind::NewFile => self.session.make_file(&value).map(|_| ()),
            DialogKind::Rename => self.session.rename_cursor(&value).map(|_| ()),
            DialogKind::Filter => {
                self.session.panes[side].set_filter(value);
                Ok(())
            }
            DialogKind::GoTo => match fsops::resolve_dir(&value) {
                Ok(dir) => self.session.panes[side].navigate(&dir),
                Err(err) => Err(err),
            },
            DialogKind::CopyTo | DialogKind::MoveTo => match fsops::resolve_dir(&value) {
                Ok(dest) => {
                    let targets = self.session.panes[side].targets();
                    if matches!(kind, DialogKind::CopyTo) {
                        self.session.start_copy(&targets, &dest)
                    } else {
                        self.session.start_move(&targets, &dest)
                    }
                }
                Err(err) => Err(err),
            },
            DialogKind::Delete { targets, mode } => self.session.start_delete(&targets, mode),
        };

        match result {
            Ok(()) => self.reveal = [true; 2],
            Err(err) => self.session.message = Some(format!("error: {err}")),
        }
    }

    fn show_viewer(&mut self, ctx: &egui::Context) {
        let Some(preview) = self.viewer.clone() else {
            return;
        };
        let mut open = true;
        egui::Window::new(preview.title())
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .default_size(egui::vec2(780.0, 560.0))
            .open(&mut open)
            .show(ctx, |ui| {
                if preview.binary {
                    ui.label(egui::RichText::new("binary file — no preview").color(ERROR));
                    return;
                }
                egui::ScrollArea::both()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.label(
                            egui::RichText::new(preview.lines.join("\n"))
                                .monospace()
                                .size(12.0),
                        );
                    });
            });
        if !open {
            self.viewer = None;
        }
    }

    // -- actions ------------------------------------------------------------

    fn begin_transfer(&mut self, kind: DialogKind) {
        self.dialog = Some(Dialog {
            kind,
            text: self.session.target_dir().display().to_string(),
        });
    }

    fn begin_rename(&mut self) {
        let name = self
            .session
            .pane()
            .cursor_entry()
            .map(|entry| entry.name.clone())
            .unwrap_or_default();
        if name.is_empty() {
            return;
        }
        self.dialog = Some(Dialog {
            kind: DialogKind::Rename,
            text: name,
        });
    }

    fn begin_delete(&mut self) {
        let targets = self.session.pane().targets();
        if targets.is_empty() {
            return;
        }
        let mode = self.session.config.delete_mode();
        if self.session.config.confirm_delete {
            self.dialog = Some(Dialog {
                kind: DialogKind::Delete { targets, mode },
                text: String::new(),
            });
        } else if let Err(err) = self.session.start_delete(&targets, mode) {
            self.session.message = Some(format!("error: {err}"));
        }
    }

    fn open_cursor(&mut self) {
        let side = self.session.active;
        let Some(entry) = self.session.panes[side].cursor_entry().cloned() else {
            return;
        };
        if entry.is_dir() {
            match self.session.panes[side].descend() {
                Ok(_) => self.reveal[side] = true,
                Err(err) => self.session.message = Some(format!("error: {err}")),
            }
        } else {
            match preview::load_default(&entry.path) {
                Ok(preview) => self.viewer = Some(preview),
                Err(err) => self.session.message = Some(format!("error: {err}")),
            }
        }
    }

    fn view_cursor(&mut self) {
        let side = self.session.active;
        let Some(entry) = self.session.panes[side].cursor_entry().cloned() else {
            return;
        };
        if entry.is_dir() {
            self.session.message = Some("View works on files; Enter opens a folder".into());
            return;
        }
        match preview::load_default(&entry.path) {
            Ok(preview) => self.viewer = Some(preview),
            Err(err) => self.session.message = Some(format!("error: {err}")),
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if self.dialog.is_some() || self.viewer.is_some() || ctx.wants_keyboard_input() {
            return;
        }
        let pressed = |key: egui::Key| ctx.input(|i| i.key_pressed(key));
        let cmd = ctx.input(|i| i.modifiers.command);
        let side = self.session.active;

        if pressed(egui::Key::Tab) {
            self.session.switch();
        }
        if pressed(egui::Key::ArrowUp) {
            self.session.panes[side].move_cursor(-1);
            self.reveal[side] = true;
        }
        if pressed(egui::Key::ArrowDown) {
            self.session.panes[side].move_cursor(1);
            self.reveal[side] = true;
        }
        if pressed(egui::Key::PageUp) {
            self.session.panes[side].page(-1, 24);
            self.reveal[side] = true;
        }
        if pressed(egui::Key::PageDown) {
            self.session.panes[side].page(1, 24);
            self.reveal[side] = true;
        }
        if pressed(egui::Key::Home) {
            self.session.panes[side].home();
            self.reveal[side] = true;
        }
        if pressed(egui::Key::End) {
            self.session.panes[side].end();
            self.reveal[side] = true;
        }
        if pressed(egui::Key::Enter) {
            self.open_cursor();
        }
        if (pressed(egui::Key::Backspace) || pressed(egui::Key::ArrowLeft))
            && self.session.panes[side].up()
        {
            self.reveal[side] = true;
        }
        if pressed(egui::Key::Space) {
            self.session.panes[side].toggle_select();
            self.session.panes[side].move_cursor(1);
            self.reveal[side] = true;
        }
        if pressed(egui::Key::F2) {
            self.begin_rename();
        }
        if pressed(egui::Key::F3) {
            self.view_cursor();
        }
        if pressed(egui::Key::F5) {
            self.begin_transfer(DialogKind::CopyTo);
        }
        if pressed(egui::Key::F6) {
            self.begin_transfer(DialogKind::MoveTo);
        }
        if pressed(egui::Key::F7) {
            self.dialog = Some(Dialog {
                kind: DialogKind::MkDir,
                text: String::new(),
            });
        }
        if pressed(egui::Key::F8) || pressed(egui::Key::Delete) {
            self.begin_delete();
        }
        if cmd && pressed(egui::Key::A) {
            self.session.panes[side].select_all();
        }
        if cmd && pressed(egui::Key::D) {
            self.session.panes[side].clear_selection();
        }
        if cmd && pressed(egui::Key::R) {
            self.session.reload_all();
            self.session.message = Some("refreshed".into());
        }
        if cmd && pressed(egui::Key::N) {
            self.dialog = Some(Dialog {
                kind: DialogKind::NewFile,
                text: String::new(),
            });
        }
        if cmd && pressed(egui::Key::G) {
            self.dialog = Some(Dialog {
                kind: DialogKind::GoTo,
                text: self.session.panes[side].path.display().to_string(),
            });
        }
        if pressed(egui::Key::Slash) {
            self.dialog = Some(Dialog {
                kind: DialogKind::Filter,
                text: self.session.panes[side].filter.clone(),
            });
        }
        if pressed(egui::Key::Period) {
            self.session.panes[side].toggle_hidden();
        }
        if pressed(egui::Key::Escape) {
            self.session.panes[side].clear_selection();
            if !self.session.panes[side].filter.is_empty() {
                self.session.panes[side].set_filter("");
            }
        }
    }

    fn save_state(&mut self) {
        if !self.persist {
            return;
        }
        let (show_hidden, sort, sort_desc) = {
            let pane = self.session.pane();
            (
                pane.show_hidden,
                pane.sort_key.label().to_string(),
                pane.sort_dir == bifold_core::sort::SortDir::Desc,
            )
        };
        self.session.config.show_hidden = show_hidden;
        self.session.config.sort = sort;
        self.session.config.sort_desc = sort_desc;
        let left = self.session.panes[LEFT].path.clone();
        let right = self.session.panes[RIGHT].path.clone();
        self.session.config.remember_dirs(&left, &right);
        let _ = self.session.config.save();
    }
}

impl eframe::App for GuiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.ui(ctx);
    }
}

impl Drop for GuiApp {
    fn drop(&mut self) {
        self.save_state();
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use bifold_core::session::Session;
    use bifold_core::sort::SortKey;
    use eframe::egui;

    use super::*;

    fn fixture() -> (tempfile::TempDir, GuiApp) {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("left")).unwrap();
        fs::create_dir(dir.path().join("right")).unwrap();
        fs::write(dir.path().join("left/a.txt"), b"hello").unwrap();
        fs::write(dir.path().join("left/b.md"), b"# b").unwrap();
        let session = Session::new_at(&dir.path().join("left"), &dir.path().join("right"));
        (dir, GuiApp::new(session))
    }

    fn run_frames(app: &mut GuiApp, frames: usize) -> egui::FullOutput {
        let ctx = egui::Context::default();
        ctx.set_theme(egui::ThemePreference::Dark);
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1240.0, 780.0),
            )),
            ..Default::default()
        };
        let mut output = ctx.run(raw.clone(), |ctx| app.ui(ctx));
        for _ in 1..frames {
            output = ctx.run(raw.clone(), |ctx| app.ui(ctx));
        }
        output
    }

    #[test]
    fn renders_without_a_window() {
        let (_dir, mut app) = fixture();
        let output = run_frames(&mut app, 3);
        assert!(!output.shapes.is_empty(), "the UI must produce draw shapes");
    }

    #[test]
    fn copy_dialog_prefills_the_other_pane() {
        let (_dir, mut app) = fixture();
        app.begin_transfer(DialogKind::CopyTo);
        let dialog = app.dialog.as_ref().expect("dialog");
        assert_eq!(
            dialog.text,
            fs::canonicalize(app.session.other().path.clone())
                .unwrap()
                .display()
                .to_string()
        );
    }

    #[test]
    fn rename_dialog_prefills_the_cursor_name() {
        let (_dir, mut app) = fixture();
        app.begin_rename();
        let dialog = app.dialog.as_ref().expect("dialog");
        assert_eq!(dialog.text, "a.txt");
    }

    #[test]
    fn submitting_mkdir_creates_the_folder() {
        let (dir, mut app) = fixture();
        app.submit_dialog(Dialog {
            kind: DialogKind::MkDir,
            text: "created-from-gui".into(),
        });
        assert!(dir.path().join("left/created-from-gui").is_dir());
        assert_eq!(
            app.session.pane().cursor_entry().unwrap().name,
            "created-from-gui"
        );
    }

    #[test]
    fn filter_dialog_narrows_listing() {
        let (_dir, mut app) = fixture();
        app.submit_dialog(Dialog {
            kind: DialogKind::Filter,
            text: "*.md".into(),
        });
        assert_eq!(app.session.pane().entries.len(), 1);
        assert_eq!(app.session.pane().entries[0].name, "b.md");
    }

    #[test]
    fn delete_dialog_carries_the_selection() {
        let (_dir, mut app) = fixture();
        app.session.pane_mut().select_all();
        app.begin_delete();
        match app.dialog.as_ref().map(|d| &d.kind) {
            Some(DialogKind::Delete { targets, .. }) => assert_eq!(targets.len(), 2),
            other => panic!("expected delete dialog, got {other:?}"),
        }
    }

    #[test]
    fn failed_operation_is_reported_not_panicked() {
        let (_dir, mut app) = fixture();
        app.submit_dialog(Dialog {
            kind: DialogKind::GoTo,
            text: "/definitely/not/a/real/directory".into(),
        });
        let message = app.session.status();
        assert!(message.starts_with("error"), "{message}");
    }

    #[test]
    fn sort_keys_are_cycleable_through_the_toolbar_path() {
        let (_dir, mut app) = fixture();
        assert_eq!(app.session.pane().sort_key, SortKey::Name);
        app.apply_action(LEFT, PaneAction::CycleSort);
        assert_eq!(app.session.pane().sort_key, SortKey::Size);
        app.apply_action(LEFT, PaneAction::ToggleSortDir);
        assert_eq!(
            app.session.pane().sort_dir,
            bifold_core::sort::SortDir::Desc
        );
    }
}
