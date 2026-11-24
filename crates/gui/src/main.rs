//! Bifold desktop front end (egui/eframe).
//!
//! Same [`Session`] as the terminal build; only the rendering differs.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Duration;

use bifold_core::complete::{Completer, InputKind, Tab};
use bifold_core::config::Config;
use bifold_core::error::Result;
use bifold_core::format::{ellipsis_left, ellipsis_right, human_size, short_time};
use bifold_core::fsops::{self, DeleteMode};
use bifold_core::help::{self, HelpLine};
use bifold_core::history::Recall;
use bifold_core::job::Phase;
use bifold_core::keys::{Action, Key, KeySpec, ViewerAction};
use bifold_core::preview::{Preview, PreviewKind};
use bifold_core::session::{LEFT, RIGHT, Session};
use bifold_core::style::{Color as CoreColor, StyledLine};
use bifold_core::theme::Theme;
use bifold_core::{APP_NAME, VERSION};
use eframe::egui;

/// Desktop colours derived from the active theme. Tokens that are "terminal
/// default" in the theme (`reset`) get a neutral dark fallback here.
#[derive(Clone, Copy)]
struct Pal {
    accent: egui::Color32,
    on_accent: egui::Color32,
    accent_bg: egui::Color32,
    text: egui::Color32,
    dim: egui::Color32,
    faint: egui::Color32,
    dir: egui::Color32,
    link: egui::Color32,
    mark: egui::Color32,
    mark_bg: egui::Color32,
    cursor_off: egui::Color32,
    error: egui::Color32,
    warn: egui::Color32,
}

fn rgb(color: Option<CoreColor>, fallback: (u8, u8, u8)) -> egui::Color32 {
    let (r, g, b) = color.map(CoreColor::to_rgb).unwrap_or(fallback);
    egui::Color32::from_rgb(r, g, b)
}

fn luma(color: egui::Color32) -> f32 {
    0.299 * f32::from(color.r()) + 0.587 * f32::from(color.g()) + 0.114 * f32::from(color.b())
}

impl Pal {
    fn new(theme: &Theme) -> Self {
        let accent = rgb(theme.accent, (96, 208, 192));
        Self {
            accent,
            on_accent: if luma(accent) > 140.0 {
                egui::Color32::BLACK
            } else {
                egui::Color32::WHITE
            },
            accent_bg: rgb(theme.active_row_bg, (28, 58, 56)),
            text: rgb(theme.text, (222, 226, 230)),
            dim: rgb(theme.subtext0, (132, 138, 144)),
            faint: rgb(theme.overlay0, (96, 102, 108)),
            dir: rgb(theme.blue, (122, 172, 255)),
            link: rgb(theme.mauve, (186, 146, 232)),
            mark: rgb(theme.yellow, (232, 200, 122)),
            mark_bg: rgb(theme.selection_bg, (44, 40, 26)),
            cursor_off: rgb(theme.surface0, (48, 50, 54)),
            error: rgb(theme.red, (232, 112, 112)),
            warn: rgb(theme.peach, (240, 160, 110)),
        }
    }
}

/// egui visuals for a theme: panels, windows, buttons and text fields follow it.
fn visuals_for(theme: &Theme, pal: &Pal) -> egui::Visuals {
    let mut v = if theme.is_light() {
        egui::Visuals::light()
    } else {
        egui::Visuals::dark()
    };
    let surface0 = rgb(theme.surface0, (48, 50, 54));
    let surface1 = rgb(theme.surface1, (64, 68, 74));
    if let Some(bg) = theme.panel_bg {
        let bg = rgb(Some(bg), (0, 0, 0));
        v.panel_fill = bg;
        v.window_fill = bg;
        v.extreme_bg_color = surface0;
        v.faint_bg_color = surface0;
    }
    if theme.text.is_some() {
        v.override_text_color = Some(pal.text);
    }
    v.hyperlink_color = pal.accent;
    v.selection.bg_fill = pal.accent_bg;
    v.selection.stroke = egui::Stroke::new(1.0_f32, pal.accent);
    v.window_stroke = egui::Stroke::new(1.0_f32, surface1);
    v.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0_f32, surface1);
    v.widgets.inactive.weak_bg_fill = surface0;
    v.widgets.inactive.bg_fill = surface0;
    v.widgets.hovered.weak_bg_fill = surface1;
    v.widgets.hovered.bg_fill = surface1;
    v.widgets.active.weak_bg_fill = pal.accent;
    v.widgets.active.bg_fill = pal.accent;
    v.widgets.active.fg_stroke.color = pal.on_accent;
    v.widgets.open.weak_bg_fill = surface1;
    v
}

const ROW_HEIGHT: f32 = 21.0;
const VIEWER_ROW: f32 = 15.0;
const VIEWER_PAGE: usize = 30;
/// Size (in character cells) reported to the previewer; the window is not cell-based.
const VIEWER_COLS: u16 = 100;
const VIEWER_ROWS: u16 = 40;

fn main() -> eframe::Result {
    let args = match parse_args() {
        Ok(Some(args)) => args,
        Ok(None) => return Ok(()),
        Err(message) => {
            eprintln!("{APP_NAME}: {message}");
            std::process::exit(2);
        }
    };

    let (mut config, config_problem) = if args.no_config {
        (Config::default(), None)
    } else {
        Config::load_checked()
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
    if let Some(theme) = &args.theme {
        config.theme = theme.clone();
    }

    let no_config = args.no_config;
    let mut session = Session::new(config);
    session.report_config_problem(config_problem);
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
    theme: Option<String>,
}

fn parse_args() -> std::result::Result<Option<Args>, String> {
    let mut args = Args::default();
    let mut raw = std::env::args().skip(1);
    while let Some(arg) = raw.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print!(
                    "{APP_NAME} {VERSION} — two-pane file manager (desktop)\n\n\
                     USAGE: bifold-gui [PATH] [--left DIR] [--right DIR] [--path DIR]\n       [--theme NAME] [--list-themes] [--no-config]\n"
                );
                return Ok(None);
            }
            "-V" | "--version" => {
                println!("{APP_NAME} {VERSION}");
                return Ok(None);
            }
            "--no-config" => args.no_config = true,
            "--theme" => args.theme = Some(raw.next().ok_or("--theme needs a name")?),
            "--list-themes" => {
                print!(
                    "{}",
                    bifold_core::theme::list_text(Config::themes_dir().as_deref())
                );
                return Ok(None);
            }
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
    /// Command line run in the active folder (Enter on an executable).
    Run,
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
            Self::Run => "Run".into(),
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
            Self::Run => "command (runs in the active folder)",
            Self::Delete { .. } => "",
        }
    }

    /// What Tab completes in this dialog (`None`: no text input).
    fn input_kind(&self) -> Option<InputKind> {
        match self {
            Self::GoTo | Self::CopyTo | Self::MoveTo => Some(InputKind::Dir),
            Self::Run => Some(InputKind::Command),
            Self::Filter | Self::MkDir | Self::NewFile | Self::Rename => Some(InputKind::Path),
            Self::Delete { .. } => None,
        }
    }

    fn needs_text(&self) -> bool {
        !matches!(self, Self::Delete { .. })
    }
}

struct RunningCommand {
    result: Receiver<bifold_core::error::Result<bifold_core::runcmd::RunOutcome>>,
    /// Started by the edit action: a clean exit (or an editor that keeps running)
    /// is not worth a window full of output.
    edit: bool,
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

struct ViewerState {
    preview: Preview,
    /// Row to scroll to on the next frame (set by keyboard actions).
    jump: Option<usize>,
    /// First visible row, read back from the scroll area.
    row: usize,
}

impl ViewerState {
    fn new(preview: Preview) -> Self {
        Self {
            preview,
            jump: None,
            row: 0,
        }
    }
}

struct GuiApp {
    session: Session,
    pal: Pal,
    /// Name of the theme the egui visuals were last built for.
    applied_theme: Option<String>,
    dialog: Option<Dialog>,
    viewer: Option<ViewerState>,
    /// A command running in the background: its text and where the result arrives.
    running: Option<RunningCommand>,
    /// Tab completion in dialogs: the state, and the last result together with
    /// the text it produced (the list is shown only while the text is unchanged).
    completer: Completer,
    tab_view: Option<(String, Tab)>,
    /// Up/Down through earlier commands in the run dialog, and the text the last
    /// recall put in the field (any other text means the user edited it).
    recall: Recall,
    recall_text: String,
    help: bool,
    help_tab: usize,
    /// The toolbar filter box has keyboard focus: let it have the keys.
    typing: bool,
    reveal: [bool; 2],
    /// Whether quitting writes the config file. Off by default so tests never
    /// touch the real one; `run` turns it on (unless `--no-config`).
    persist: bool,
}

impl GuiApp {
    fn new(session: Session) -> Self {
        let pal = Pal::new(&session.theme);
        Self {
            pal,
            applied_theme: None,
            session,
            dialog: None,
            running: None,
            completer: Completer::default(),
            recall: Recall::default(),
            recall_text: String::new(),
            tab_view: None,
            viewer: None,
            help: false,
            help_tab: 0,
            typing: false,
            reveal: [false; 2],
            persist: false,
        }
    }

    /// Re-derive colours and egui visuals when the theme changed (or on the first frame).
    fn sync_theme(&mut self, ctx: &egui::Context) {
        if self.applied_theme.as_deref() == Some(self.session.theme.name.as_str()) {
            return;
        }
        self.pal = Pal::new(&self.session.theme);
        let theme = &self.session.theme;
        ctx.set_theme(if theme.is_light() {
            egui::ThemePreference::Light
        } else {
            egui::ThemePreference::Dark
        });
        let which = if theme.is_light() {
            egui::Theme::Light
        } else {
            egui::Theme::Dark
        };
        ctx.set_visuals_of(which, visuals_for(theme, &self.pal));
        self.applied_theme = Some(theme.name.clone());
    }

    fn ui(&mut self, ctx: &egui::Context) {
        if self.session.jobs.is_busy() {
            ctx.request_repaint_after(Duration::from_millis(80));
        }
        if self.session.poll_jobs().is_some() {
            self.reveal = [true; 2];
        }

        self.poll_running(ctx);
        self.shortcuts(ctx);
        self.sync_theme(ctx);

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
        self.show_help(ctx);
    }

    // -- toolbar ------------------------------------------------------------

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        let pal = self.pal;
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
                .color(pal.dim),
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
                .add_enabled(self.running.is_none(), egui::Button::new("Edit").small())
                .on_hover_text("e — open in your editor (gui_editor in the config)")
                .clicked()
            {
                self.begin_edit();
            }
            if ui
                .add_enabled(!busy, egui::Button::new("Delete").small())
                .on_hover_text("F8")
                .clicked()
            {
                self.begin_delete();
            }
            if ui
                .add_enabled(self.running.is_none(), egui::Button::new("Run…").small())
                .on_hover_text("! — run any command in this folder")
                .clicked()
            {
                self.run_action(ui.ctx(), Action::RunCommand);
            }
            if ui
                .add_enabled(!busy, egui::Button::new("View").small())
                .on_hover_text("F3 — text, or hex for binary files")
                .clicked()
            {
                self.view_cursor(false);
            }
            if ui
                .add_enabled(!busy, egui::Button::new("Hex").small())
                .on_hover_text("F4 — hex dump")
                .clicked()
            {
                self.view_cursor(true);
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

            let current_theme = self.session.theme.name.clone();
            let mut chosen_theme = None;
            egui::ComboBox::from_id_salt("theme-picker")
                .selected_text(current_theme.as_str())
                .width(120.0)
                .show_ui(ui, |ui| {
                    for name in self.session.theme_names() {
                        if ui
                            .selectable_label(name == current_theme, name.as_str())
                            .clicked()
                        {
                            chosen_theme = Some(name);
                        }
                    }
                })
                .response
                .on_hover_text("colour theme (t cycles)");
            if let Some(name) = chosen_theme {
                self.session.set_theme(&name);
            }

            let mut filter = self.session.panes[side].filter.clone();
            let width = (ui.available_width() - 70.0).clamp(90.0, 220.0);
            let filter_box = ui.add(
                egui::TextEdit::singleline(&mut filter)
                    .hint_text("filter")
                    .desired_width(width),
            );
            self.typing = filter_box.has_focus();
            if filter_box.changed() {
                self.session.panes[side].set_filter(filter);
            }
            if ui.small_button("✕").on_hover_text("clear filter").clicked() {
                self.session.panes[side].set_filter("");
            }
        });
    }

    // -- panes --------------------------------------------------------------

    fn pane(&mut self, ui: &mut egui::Ui, side: usize) {
        let pal = self.pal;
        let active = self.session.active == side;
        let mut action: Option<PaneAction> = None;

        ui.horizontal(|ui| {
            let tag_text = if side == LEFT { " LEFT " } else { " RIGHT " };
            let tag = egui::Button::new(
                egui::RichText::new(tag_text)
                    .monospace()
                    .size(11.0)
                    .strong()
                    .color(if active { pal.on_accent } else { pal.dim }),
            )
            .fill(if active {
                pal.accent
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
                    .color(if active { pal.text } else { pal.dim }),
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
            pal.faint,
        );
        painter.text(
            header_rect.right_center() - egui::vec2(126.0, 0.0),
            egui::Align2::RIGHT_CENTER,
            "size",
            mono_small.clone(),
            pal.faint,
        );
        painter.text(
            header_rect.right_center() - egui::vec2(6.0, 0.0),
            egui::Align2::RIGHT_CENTER,
            "modified",
            mono_small,
            pal.faint,
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
                            pal.accent_bg
                        } else {
                            pal.cursor_off
                        },
                    );
                } else if marked {
                    ui.painter().rect_filled(rect, 3.0, pal.mark_bg);
                }

                if marked {
                    ui.painter().circle_filled(
                        rect.left_center() + egui::vec2(8.0, 0.0),
                        2.5,
                        pal.mark,
                    );
                }

                let name_color = if entry.is_dir() {
                    pal.dir
                } else if entry.symlink {
                    pal.link
                } else if entry.hidden() {
                    pal.faint
                } else {
                    pal.text
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
                    pal.dim,
                );
                let time = entry.modified.map(short_time).unwrap_or_default();
                ui.painter().text(
                    rect.right_center() - egui::vec2(6.0, 0.0),
                    egui::Align2::RIGHT_CENTER,
                    time,
                    egui::FontId::monospace(11.0),
                    pal.dim,
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
        let pal = self.pal;
        ui.horizontal(|ui| {
            let pane = self.session.pane();
            ui.label(
                egui::RichText::new(ellipsis_left(&pane.path.display().to_string(), 46))
                    .monospace()
                    .size(11.0)
                    .color(pal.dim),
            );
            ui.separator();
            let status = self.session.status();
            let color = if status.starts_with("error") {
                pal.error
            } else if status.starts_with("config:") {
                pal.warn
            } else {
                pal.text
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
                        pal.error
                    } else {
                        pal.dim
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
                            .color(pal.text),
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
        let pal = self.pal;
        let Some(mut dialog) = self.dialog.take() else {
            // No dialog open: the next one starts a fresh walk through the history.
            self.recall.reset();
            self.recall_text.clear();
            return;
        };
        let mut submit = false;
        let mut cancel = false;
        let title = dialog.kind.title();
        let cwd = self.session.pane().path.clone();
        let completer = &mut self.completer;
        let tab_view = &mut self.tab_view;
        let history = &self.session.history;
        let recall = &mut self.recall;
        let recall_text = &mut self.recall_text;
        let recalls = matches!(dialog.kind, DialogKind::Run);

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
                            .color(pal.dim),
                    );
                    // Tab completes: a dialog key, fixed like Enter and Esc. egui would
                    // move focus, so keep Tab in the field and consume it here
                    // (Shift+Tab first: `consume_key` ignores an extra Shift).
                    let input_id = egui::Id::new("bifold-dialog-input");
                    ui.memory_mut(|m| {
                        m.set_focus_lock_filter(
                            input_id,
                            egui::EventFilter {
                                tab: true,
                                horizontal_arrows: false,
                                vertical_arrows: recalls,
                                escape: false,
                            },
                        )
                    });
                    let (back, forward) = ui.input_mut(|i| {
                        (
                            i.consume_key(egui::Modifiers::SHIFT, egui::Key::Tab),
                            i.consume_key(egui::Modifiers::NONE, egui::Key::Tab),
                        )
                    });
                    // Up/Down recall earlier commands in the run line: fixed dialog keys.
                    let (up, down) = if recalls {
                        ui.input_mut(|i| {
                            (
                                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                            )
                        })
                    } else {
                        (false, false)
                    };
                    let mut recalled = false;
                    if up || down {
                        if dialog.text != *recall_text {
                            recall.reset(); // typed since the last recall
                        }
                        let found = if up {
                            recall.older(history, &dialog.text)
                        } else {
                            recall.newer(history, &dialog.text)
                        };
                        if let Some(text) = found {
                            dialog.text = text.clone();
                            *recall_text = text;
                            recalled = true;
                        }
                    }
                    let tabbed = back || forward;
                    if tabbed && let Some(kind) = dialog.kind.input_kind() {
                        let result = completer.tab(kind, &dialog.text, &cwd, back);
                        if let Some(text) = &result.text {
                            dialog.text = text.clone();
                        }
                        *tab_view = Some((dialog.text.clone(), result));
                    }
                    let mut output = egui::TextEdit::singleline(&mut dialog.text)
                        .id(input_id)
                        .desired_width(f32::INFINITY)
                        .show(ui);
                    if tabbed || recalled {
                        // The cursor belongs at the end of what was just filled in.
                        let end = egui::text::CCursor::new(dialog.text.chars().count());
                        output
                            .state
                            .cursor
                            .set_char_range(Some(egui::text::CCursorRange::one(end)));
                        output.state.store(ui.ctx(), output.response.id);
                    }
                    let response = output.response;
                    response.request_focus();
                    show_completions(ui, &pal, tab_view.as_ref(), &dialog.text);
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
                            .color(pal.faint),
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
            DialogKind::Run => {
                if value.is_empty() {
                    Err(bifold_core::error::Error::invalid(
                        "command cannot be empty",
                    ))
                } else {
                    self.session.history.push(&value);
                    self.start_command(value)
                }
            }
            DialogKind::Delete { targets, mode } => self.session.start_delete(&targets, mode),
        };

        match result {
            Ok(()) => self.reveal = [true; 2],
            Err(err) => self.session.message = Some(format!("error: {err}")),
        }
    }

    /// Run `command` in the active folder on a worker thread, so the window
    /// stays responsive; `poll_running` picks the result up.
    fn start_command(&mut self, command: String) -> bifold_core::error::Result<()> {
        self.launch(command, false)
    }

    fn launch(&mut self, command: String, edit: bool) -> bifold_core::error::Result<()> {
        if self.running.is_some() {
            return Err(bifold_core::error::Error::invalid(
                "a command is already running",
            ));
        }
        let cwd = self.session.panes[self.session.active].path.clone();
        let timeout = self.session.run_timeout();
        let (tx, rx) = mpsc::channel();
        let (cmd, dir) = (command.clone(), cwd.clone());
        std::thread::spawn(move || {
            let _ = tx.send(bifold_core::runcmd::run_captured(&cmd, &dir, timeout));
        });
        self.session.message = Some(format!("running: {command}"));
        self.running = Some(RunningCommand { result: rx, edit });
        Ok(())
    }

    fn poll_running(&mut self, ctx: &egui::Context) {
        let Some(running) = &self.running else {
            return;
        };
        let outcome = match running.result.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => {
                ctx.request_repaint_after(Duration::from_millis(80));
                return;
            }
            Err(TryRecvError::Disconnected) => Err(bifold_core::error::Error::invalid(
                "the command runner stopped unexpectedly",
            )),
        };
        let edit = running.edit;
        self.running = None;
        match outcome {
            Ok(out) if edit && (out.succeeded() || out.detached) => {
                self.session.message = Some(format!("edit: {}", out.command));
                self.session.last_run = Some(out);
            }
            Ok(out) => {
                self.session.message = Some(out.summary());
                self.session.last_run = Some(out);
                self.show_last_output();
            }
            Err(err) => self.session.message = Some(format!("error: {err}")),
        }
        // The command probably changed the folder it ran in.
        self.session.reload_all();
        self.reveal = [true; 2];
    }

    fn show_viewer(&mut self, ctx: &egui::Context) {
        let pal = self.pal;
        let theme = self.session.theme.clone();
        let Some(mut viewer) = self.viewer.take() else {
            return;
        };
        let mut open = true;
        let mut toggle_hex = false;
        egui::Window::new(viewer.preview.title())
            .id(egui::Id::new("bifold-viewer"))
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .default_size(egui::vec2(820.0, 560.0))
            .open(&mut open)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let is_hex = viewer.preview.kind == PreviewKind::Hex;
                    if viewer.preview.kind != PreviewKind::Output
                        && ui
                            .small_button(if is_hex { "text" } else { "hex" })
                            .clicked()
                    {
                        toggle_hex = true;
                    }
                    ui.label(
                        egui::RichText::new(format!(
                            "{} · {} lines",
                            human_size(viewer.preview.size),
                            viewer.preview.line_count()
                        ))
                        .size(11.0)
                        .color(pal.dim),
                    );
                });
                ui.spacing_mut().item_spacing.y = 0.0;
                let mut area = egui::ScrollArea::both()
                    .id_salt("viewer-rows")
                    .auto_shrink([false, false]);
                if let Some(row) = viewer.jump.take() {
                    area = area.vertical_scroll_offset(row as f32 * VIEWER_ROW);
                }
                let lines = &viewer.preview.lines;
                let output = area.show_rows(ui, VIEWER_ROW, lines.len(), |ui, range| {
                    for line in &lines[range] {
                        let galley = ui.painter().layout_job(layout_line(line, &theme, &pal));
                        let width = galley.size().x.max(ui.available_width());
                        let (rect, _) = ui.allocate_exact_size(
                            egui::vec2(width, VIEWER_ROW),
                            egui::Sense::hover(),
                        );
                        ui.painter().galley(rect.min, galley, pal.text);
                    }
                });
                viewer.row = (output.state.offset.y / VIEWER_ROW).round().max(0.0) as usize;
            });
        if open {
            self.viewer = Some(viewer);
            if toggle_hex {
                self.run_viewer_action(ViewerAction::ToggleHex);
            }
        }
    }

    fn open_help(&mut self) {
        self.help = true;
        self.help_tab = 0;
    }

    /// Keys while the help window is open: tabs and close are fixed, not configurable.
    fn help_key(&mut self, key: &KeySpec) {
        let count = help::TITLES.len();
        let closes = help::is_help_key(key)
            || matches!(
                key.key,
                Key::Esc | Key::Char('q') | Key::Char('?') | Key::Enter
            )
            || self.session.keymap.action(key) == Some(Action::Help);
        if closes {
            self.help = false;
            return;
        }
        self.help_tab = match key.key {
            Key::Tab if key.shift => help::step(self.help_tab, -1, count),
            Key::Right | Key::Tab | Key::Char('l') => help::step(self.help_tab, 1, count),
            Key::Left => help::step(self.help_tab, -1, count),
            Key::Home => 0,
            Key::End => count - 1,
            Key::Char(c @ '1'..='9') if (c as usize - '1' as usize) < count => {
                c as usize - '1' as usize
            }
            _ => self.help_tab,
        };
    }

    fn show_help(&mut self, ctx: &egui::Context) {
        let pal = self.pal;
        if !self.help {
            return;
        }
        let pages = self.session.help_pages();
        let mut tab = self.help_tab.min(pages.len().saturating_sub(1));
        let mut open = true;
        egui::Window::new("Help")
            .id(egui::Id::new("bifold-help"))
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .collapsible(false)
            .default_width(640.0)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.set_max_width(640.0);
                ui.horizontal(|ui| {
                    for (index, page) in pages.iter().enumerate() {
                        if ui
                            .selectable_label(index == tab, format!("{} {}", index + 1, page.title))
                            .clicked()
                        {
                            tab = index;
                        }
                    }
                });
                ui.separator();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let Some(page) = pages.get(tab) else {
                        return;
                    };
                    // Consecutive key rows share one grid so the columns line up.
                    let mut rows: Vec<(&str, &str)> = Vec::new();
                    let mut block = 0;
                    let flush = |ui: &mut egui::Ui,
                                 rows: &mut Vec<(&str, &str)>,
                                 block: &mut usize| {
                        if rows.is_empty() {
                            return;
                        }
                        egui::Grid::new(("help-keys", tab, *block))
                            .num_columns(2)
                            .spacing([18.0, 3.0])
                            .show(ui, |ui| {
                                for (keys, text) in rows.drain(..) {
                                    ui.label(egui::RichText::new(keys).monospace().color(pal.mark));
                                    ui.label(text);
                                    ui.end_row();
                                }
                            });
                        *block += 1;
                    };
                    for line in &page.lines {
                        match line {
                            HelpLine::Key(keys, text) => rows.push((keys, text)),
                            HelpLine::Heading(text) => {
                                flush(ui, &mut rows, &mut block);
                                ui.add_space(4.0);
                                ui.label(egui::RichText::new(text).strong().color(pal.accent));
                            }
                            HelpLine::Note(text) => {
                                flush(ui, &mut rows, &mut block);
                                ui.add(egui::Label::new(text.as_str()).wrap());
                            }
                        }
                    }
                    flush(ui, &mut rows, &mut block);
                });
                ui.separator();
                ui.label(
                    egui::RichText::new("←/→ or 1-5 switch tab · h / Esc close")
                        .size(11.0)
                        .color(pal.faint),
                );
            });
        self.help_tab = tab;
        self.help = open;
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

    /// Open the file under the cursor in the desktop editor (`gui_editor`, else the
    /// system default). A terminal editor needs the terminal UI.
    fn begin_edit(&mut self) {
        let result = self
            .session
            .edit_command(true)
            .and_then(|command| self.launch(command, true));
        if let Err(err) = result {
            self.session.message = Some(format!("error: {err}"));
        }
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
        } else if let Some(command) = self.session.run_suggestion() {
            self.dialog = Some(Dialog {
                kind: DialogKind::Run,
                text: command,
            });
        } else {
            self.show_file(&entry.path, false);
        }
    }

    /// Ctrl+O: the output of the last command run from the file list.
    fn show_last_output(&mut self) {
        match self.session.last_run_preview() {
            Some(preview) => self.viewer = Some(ViewerState::new(preview)),
            None => {
                self.session.message =
                    Some("no command has been run yet (Enter on an executable runs one)".into());
            }
        }
    }

    fn view_cursor(&mut self, hex: bool) {
        let side = self.session.active;
        let Some(entry) = self.session.panes[side].cursor_entry().cloned() else {
            return;
        };
        if entry.is_dir() {
            self.session.message = Some("View works on files; Enter opens a folder".into());
            return;
        }
        self.show_file(&entry.path, hex);
    }

    fn show_file(&mut self, path: &std::path::Path, hex: bool) {
        match self.session.preview(path, VIEWER_COLS, VIEWER_ROWS, hex) {
            Ok(preview) => self.viewer = Some(ViewerState::new(preview)),
            Err(err) => self.session.message = Some(format!("error: {err}")),
        }
    }

    fn run_viewer_action(&mut self, action: ViewerAction) {
        let Some(viewer) = self.viewer.as_mut() else {
            return;
        };
        let last = viewer.preview.lines.len().saturating_sub(1);
        let base = viewer.jump.unwrap_or(viewer.row);
        match action {
            ViewerAction::Close => self.viewer = None,
            ViewerAction::Up => viewer.jump = Some(base.saturating_sub(1)),
            ViewerAction::Down => viewer.jump = Some((base + 1).min(last)),
            ViewerAction::PageUp => viewer.jump = Some(base.saturating_sub(VIEWER_PAGE)),
            ViewerAction::PageDown => viewer.jump = Some((base + VIEWER_PAGE).min(last)),
            ViewerAction::Top => viewer.jump = Some(0),
            ViewerAction::Bottom => viewer.jump = Some(last),
            // Lines scroll horizontally on the desktop; wrapping is a terminal-only option.
            ViewerAction::ToggleWrap => {}
            // Command output has no file behind it to re-read as hex.
            ViewerAction::ToggleHex if viewer.preview.kind == PreviewKind::Output => {}
            ViewerAction::ToggleHex => {
                let want_hex = viewer.preview.kind != PreviewKind::Hex;
                let path = viewer.preview.path.clone();
                self.show_file(&path, want_hex);
            }
        }
    }

    fn run_action(&mut self, ctx: &egui::Context, action: Action) {
        let side = self.session.active;
        if self.session.apply(action, 24) {
            self.reveal = [true; 2];
            return;
        }
        match action {
            Action::Quit => {
                if self.session.jobs.is_busy() {
                    self.session.message = Some("transfer running — cancel it first".into());
                } else {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            Action::Open => self.open_cursor(),
            Action::View => self.view_cursor(false),
            Action::ViewHex => self.view_cursor(true),
            Action::LastOutput => self.show_last_output(),
            Action::RunCommand => {
                self.dialog = Some(Dialog {
                    kind: DialogKind::Run,
                    text: String::new(),
                })
            }
            Action::Rename => self.begin_rename(),
            Action::Edit => self.begin_edit(),
            Action::Copy => self.begin_transfer(DialogKind::CopyTo),
            Action::Move => self.begin_transfer(DialogKind::MoveTo),
            Action::Delete => self.begin_delete(),
            Action::MkDir => {
                self.dialog = Some(Dialog {
                    kind: DialogKind::MkDir,
                    text: String::new(),
                })
            }
            Action::NewFile => {
                self.dialog = Some(Dialog {
                    kind: DialogKind::NewFile,
                    text: String::new(),
                })
            }
            Action::Filter => {
                self.dialog = Some(Dialog {
                    kind: DialogKind::Filter,
                    text: self.session.panes[side].filter.clone(),
                })
            }
            Action::GoTo => {
                self.dialog = Some(Dialog {
                    kind: DialogKind::GoTo,
                    text: self.session.panes[side].path.display().to_string(),
                })
            }
            Action::Help => self.open_help(),
            _ => {}
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        // `ctx.wants_keyboard_input()` is true for *any* focused widget, so one Tab
        // (egui's focus navigation) would otherwise kill every shortcut.
        if self.dialog.is_some() || self.typing {
            return;
        }
        ctx.memory_mut(|memory| {
            if let Some(id) = memory.focused() {
                memory.surrender_focus(id);
            }
        });
        let keys = key_events(ctx);
        if self.viewer.is_some() {
            for key in keys {
                if let Some(action) = self.session.keymap.viewer_action(&key) {
                    self.run_viewer_action(action);
                }
            }
            return;
        }
        if self.help {
            for key in keys {
                self.help_key(&key);
            }
            return;
        }
        for key in keys {
            // Fixed key: works whatever the keymap says.
            if help::is_help_key(&key) {
                self.open_help();
                continue;
            }
            if let Some(action) = self.session.keymap.action(&key) {
                self.run_action(ctx, action);
            }
        }
    }

    fn save_state(&mut self) {
        if !self.persist {
            return;
        }
        let _ = self.session.save_config();
    }
}

/// Under a dialog's text field: the candidates of the last Tab (the cycled one
/// highlighted), "no match", or a quiet reminder. The list only counts while
/// the text is the one Tab produced; any edit drops it.
fn show_completions(ui: &mut egui::Ui, pal: &Pal, view: Option<&(String, Tab)>, text: &str) {
    let live = view
        .filter(|(shown_for, _)| shown_for == text)
        .map(|(_, tab)| tab);
    let Some(tab) = live.filter(|t| !t.listing.is_empty() || t.message.is_some()) else {
        ui.label(
            egui::RichText::new("Tab completes")
                .size(11.0)
                .color(pal.faint),
        );
        return;
    };
    if let Some(note) = &tab.message {
        ui.label(egui::RichText::new(note).size(11.0).color(pal.warn));
        return;
    }
    ui.horizontal_wrapped(|ui| {
        for (i, label) in tab.listing.iter().enumerate() {
            let text = egui::RichText::new(label).monospace().size(11.0);
            if tab.current == Some(i) {
                ui.label(text.strong().color(pal.accent));
            } else {
                ui.label(text.color(pal.dim));
            }
        }
        if tab.more > 0 {
            ui.label(
                egui::RichText::new(format!("+{} more", tab.more))
                    .size(11.0)
                    .color(pal.faint),
            );
        }
    });
}

/// Key presses of this frame, in the shared key vocabulary.
fn key_events(ctx: &egui::Context) -> Vec<KeySpec> {
    ctx.input(|input| {
        input
            .events
            .iter()
            .filter_map(|event| match event {
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => spec_from_egui(*key, *modifiers),
                _ => None,
            })
            .collect()
    })
}

fn spec_from_egui(key: egui::Key, modifiers: egui::Modifiers) -> Option<KeySpec> {
    use egui::Key as K;
    let mapped = match key {
        K::Enter => Key::Enter,
        K::Escape => Key::Esc,
        K::Tab => Key::Tab,
        K::Space => Key::Space,
        K::Backspace => Key::Backspace,
        K::Delete => Key::Delete,
        K::Insert => Key::Insert,
        K::ArrowUp => Key::Up,
        K::ArrowDown => Key::Down,
        K::ArrowLeft => Key::Left,
        K::ArrowRight => Key::Right,
        K::Home => Key::Home,
        K::End => Key::End,
        K::PageUp => Key::PageUp,
        K::PageDown => Key::PageDown,
        K::F1 => Key::F(1),
        K::F2 => Key::F(2),
        K::F3 => Key::F(3),
        K::F4 => Key::F(4),
        K::F5 => Key::F(5),
        K::F6 => Key::F(6),
        K::F7 => Key::F(7),
        K::F8 => Key::F(8),
        K::F9 => Key::F(9),
        K::F10 => Key::F(10),
        K::F11 => Key::F(11),
        K::F12 => Key::F(12),
        other => {
            let symbol = other.symbol_or_name();
            let mut chars = symbol.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) if c.is_ascii_alphabetic() => Key::Char(if modifiers.shift {
                    c.to_ascii_uppercase()
                } else {
                    c.to_ascii_lowercase()
                }),
                (Some(c), None) => Key::Char(c),
                _ => return None,
            }
        }
    };
    // `command` is Cmd on macOS: treat it like Ctrl so one keymap serves both front ends.
    Some(KeySpec::new(
        mapped,
        modifiers.ctrl || modifiers.command,
        modifiers.alt,
        modifiers.shift,
    ))
}

fn layout_line(line: &StyledLine, theme: &Theme, pal: &Pal) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    for span in &line.spans {
        let color = span
            .fg
            .and_then(|c| theme.resolve(c))
            .map(|c| rgb(Some(c), (222, 226, 230)))
            .unwrap_or(pal.text);
        job.append(
            &span.text,
            0.0,
            egui::TextFormat {
                font_id: egui::FontId::monospace(12.0),
                color,
                ..Default::default()
            },
        );
    }
    job
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
    fn a_new_app_never_writes_the_config_file() {
        let (_dir, app) = fixture();
        assert!(!app.persist);
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

    fn frame(ctx: &egui::Context, app: &mut GuiApp) {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1240.0, 780.0),
            )),
            ..Default::default()
        };
        let _ = ctx.run(raw, |ctx| app.ui(ctx));
    }

    fn press(ctx: &egui::Context, app: &mut GuiApp, key: egui::Key, modifiers: egui::Modifiers) {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1240.0, 780.0),
            )),
            events: vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            }],
            ..Default::default()
        };
        let _ = ctx.run(raw, |ctx| app.ui(ctx));
    }

    #[test]
    fn keymap_drives_the_desktop_front_end() {
        let (_dir, mut app) = fixture();
        let ctx = egui::Context::default();
        let none = egui::Modifiers::NONE;

        press(&ctx, &mut app, egui::Key::F7, none);
        assert!(matches!(
            app.dialog.as_ref().map(|d| &d.kind),
            Some(DialogKind::MkDir)
        ));
        app.dialog = None;
        frame(&ctx, &mut app); // lets egui drop focus from the closed dialog's text field

        press(&ctx, &mut app, egui::Key::Tab, none);
        assert_eq!(app.session.active, RIGHT);
        press(&ctx, &mut app, egui::Key::Tab, egui::Modifiers::SHIFT);
        assert_eq!(app.session.active, LEFT);

        press(
            &ctx,
            &mut app,
            egui::Key::Questionmark,
            egui::Modifiers::SHIFT,
        );
        assert!(app.help);
        press(&ctx, &mut app, egui::Key::Num3, none);
        assert!(app.help && app.help_tab == 2, "digits switch tabs");
        press(&ctx, &mut app, egui::Key::Escape, none);
        assert!(!app.help, "Esc closes help");
        press(&ctx, &mut app, egui::Key::H, none);
        assert!(
            app.help && app.help_tab == 0,
            "h opens help at the first tab"
        );
        press(&ctx, &mut app, egui::Key::H, none);
        assert!(!app.help, "h closes it again");

        // Rebound in config: unbinding F7 and putting mkdir on `m`.
        app.session.keymap = {
            let mut config = bifold_core::config::Config::default();
            config.keys.insert("m".into(), "mkdir".into());
            config.keys.insert("f7".into(), "none".into());
            config.keymap().0
        };
        press(&ctx, &mut app, egui::Key::F7, none);
        assert!(app.dialog.is_none());
        frame(&ctx, &mut app);
        press(&ctx, &mut app, egui::Key::M, none);
        assert!(app.dialog.is_some());
    }

    #[test]
    fn binary_file_opens_as_hex_and_scrolls_by_key() {
        let (dir, mut app) = fixture();
        let blob: Vec<u8> = (0..=255u8).cycle().take(4096).collect();
        fs::write(dir.path().join("left/blob.bin"), &blob).unwrap();
        app.session.pane_mut().reload();
        let at = app
            .session
            .pane()
            .entries
            .iter()
            .position(|e| e.name == "blob.bin")
            .unwrap();
        app.session.pane_mut().cursor = at;

        let ctx = egui::Context::default();
        let none = egui::Modifiers::NONE;
        press(&ctx, &mut app, egui::Key::F3, none);
        let viewer = app.viewer.as_ref().expect("viewer open");
        assert_eq!(viewer.preview.kind, PreviewKind::Hex);
        assert_eq!(viewer.preview.line_count(), 4096 / 16);
        frame(&ctx, &mut app);
        frame(&ctx, &mut app);
        assert_eq!(app.viewer.as_ref().unwrap().row, 0);

        press(&ctx, &mut app, egui::Key::End, none);
        let row = app.viewer.as_ref().unwrap().row;
        assert!(row > 200, "End scrolled to the bottom, row={row}");
        press(&ctx, &mut app, egui::Key::Home, none);
        assert_eq!(app.viewer.as_ref().unwrap().row, 0);

        press(&ctx, &mut app, egui::Key::X, none);
        assert_eq!(
            app.viewer.as_ref().unwrap().preview.kind,
            PreviewKind::Hex,
            "binary stays hex when toggled"
        );
        press(&ctx, &mut app, egui::Key::Escape, none);
        assert!(app.viewer.is_none());
    }

    #[test]
    fn egui_keys_map_to_shared_specs() {
        let spec = |key, mods| spec_from_egui(key, mods).unwrap().to_string();
        assert_eq!(spec(egui::Key::F5, egui::Modifiers::NONE), "f5");
        assert_eq!(spec(egui::Key::G, egui::Modifiers::SHIFT), "G");
        assert_eq!(spec(egui::Key::A, egui::Modifiers::CTRL), "ctrl+a");
        assert_eq!(spec(egui::Key::A, egui::Modifiers::COMMAND), "ctrl+a");
        assert_eq!(spec(egui::Key::Period, egui::Modifiers::NONE), ".");
        assert_eq!(spec(egui::Key::Slash, egui::Modifiers::NONE), "/");
    }

    #[test]
    fn theme_drives_egui_visuals() {
        let (_dir, mut app) = fixture();
        let ctx = egui::Context::default();

        assert!(app.session.set_theme("gruvbox-light"));
        frame(&ctx, &mut app);
        let visuals = ctx.style().visuals.clone();
        assert!(
            !visuals.dark_mode,
            "light theme switches egui to its light base"
        );
        assert_eq!(visuals.panel_fill, egui::Color32::from_rgb(251, 241, 199));
        assert_eq!(
            visuals.override_text_color,
            Some(egui::Color32::from_rgb(60, 56, 54))
        );
        assert_eq!(app.pal.accent, egui::Color32::from_rgb(7, 102, 120));
        assert_eq!(
            app.pal.on_accent,
            egui::Color32::WHITE,
            "text on the dark teal accent"
        );

        assert!(app.session.set_theme("nord"));
        frame(&ctx, &mut app);
        assert!(ctx.style().visuals.dark_mode);
        assert_eq!(
            ctx.style().visuals.panel_fill,
            egui::Color32::from_rgb(46, 52, 64)
        );
        assert_eq!(
            app.pal.on_accent,
            egui::Color32::BLACK,
            "text on the light frost accent"
        );
    }

    #[test]
    fn terminal_theme_falls_back_to_neutral_colours() {
        let (_dir, mut app) = fixture();
        let ctx = egui::Context::default();
        assert!(app.session.set_theme("terminal"));
        frame(&ctx, &mut app);
        assert_eq!(
            app.pal.text,
            egui::Color32::from_rgb(222, 226, 230),
            "reset text gets a fallback"
        );
        assert!(ctx.style().visuals.override_text_color.is_none());
    }

    #[test]
    fn t_key_cycles_themes_on_the_desktop() {
        let (_dir, mut app) = fixture();
        let ctx = egui::Context::default();
        let before = app.session.theme.name.clone();
        press(&ctx, &mut app, egui::Key::T, egui::Modifiers::NONE);
        assert_ne!(app.session.theme.name, before);
        assert_eq!(
            app.applied_theme.as_deref(),
            Some(app.session.theme.name.as_str())
        );
    }

    #[test]
    fn hex_viewer_renders_under_every_builtin_theme() {
        let (dir, mut app) = fixture();
        std::fs::write(dir.path().join("left/z.bin"), [0u8, 1, 2, 3, 255]).unwrap();
        app.session.pane_mut().reload();
        let at = app
            .session
            .pane()
            .entries
            .iter()
            .position(|e| e.name == "z.bin")
            .unwrap();
        app.session.pane_mut().cursor = at;
        let ctx = egui::Context::default();
        press(&ctx, &mut app, egui::Key::F3, egui::Modifiers::NONE);
        for name in app.session.theme_names() {
            assert!(app.session.set_theme(&name));
            frame(&ctx, &mut app);
            let output = ctx.run(egui::RawInput::default(), |ctx| app.ui(ctx));
            assert!(!output.shapes.is_empty(), "{name}");
        }
    }

    #[test]
    fn h_opens_help_even_when_the_help_action_is_unbound() {
        let (_dir, mut app) = fixture();
        app.session.keymap = {
            let mut config = bifold_core::config::Config::default();
            config.keys.insert("?".into(), "none".into());
            config.keymap().0
        };
        let ctx = egui::Context::default();
        let none = egui::Modifiers::NONE;
        press(
            &ctx,
            &mut app,
            egui::Key::Questionmark,
            egui::Modifiers::SHIFT,
        );
        assert!(!app.help, "the configurable key is gone");
        press(&ctx, &mut app, egui::Key::H, none);
        assert!(app.help);
    }

    #[test]
    fn help_tabs_navigate_with_the_keyboard() {
        let (_dir, mut app) = fixture();
        let ctx = egui::Context::default();
        let none = egui::Modifiers::NONE;
        press(&ctx, &mut app, egui::Key::H, none);
        press(&ctx, &mut app, egui::Key::ArrowRight, none);
        assert_eq!(app.help_tab, 1);
        press(&ctx, &mut app, egui::Key::Tab, none);
        assert_eq!(app.help_tab, 2);
        press(&ctx, &mut app, egui::Key::Tab, egui::Modifiers::SHIFT);
        assert_eq!(app.help_tab, 1);
        press(&ctx, &mut app, egui::Key::End, none);
        assert_eq!(app.help_tab, 4);
        press(&ctx, &mut app, egui::Key::ArrowRight, none);
        assert_eq!(app.help_tab, 0, "wraps");
        press(&ctx, &mut app, egui::Key::Num4, none);
        assert_eq!(app.help_tab, 3);
        press(&ctx, &mut app, egui::Key::X, none);
        assert!(
            app.help && app.help_tab == 3,
            "other keys are ignored, not close"
        );
        press(&ctx, &mut app, egui::Key::Q, none);
        assert!(!app.help);
    }

    #[test]
    fn every_help_tab_renders_in_every_theme() {
        let (_dir, mut app) = fixture();
        let ctx = egui::Context::default();
        press(&ctx, &mut app, egui::Key::H, egui::Modifiers::NONE);
        for name in ["bifold", "nord", "solarized-light"] {
            assert!(app.session.set_theme(name));
            for tab in 0..help::TITLES.len() {
                app.help_tab = tab;
                let output = ctx.run(egui::RawInput::default(), |ctx| app.ui(ctx));
                assert!(!output.shapes.is_empty(), "{name} tab {tab}");
                assert!(app.help, "help stays open while rendering");
            }
        }
    }

    #[test]
    fn h_does_not_trigger_while_typing_or_in_a_dialog() {
        let (_dir, mut app) = fixture();
        let ctx = egui::Context::default();
        app.typing = true;
        press(&ctx, &mut app, egui::Key::H, egui::Modifiers::NONE);
        assert!(!app.help, "filter box has the keyboard");
        app.typing = false;
        app.dialog = Some(Dialog {
            kind: DialogKind::MkDir,
            text: String::new(),
        });
        press(&ctx, &mut app, egui::Key::H, egui::Modifiers::NONE);
        assert!(!app.help, "a dialog has the keyboard");
    }

    #[cfg(unix)]
    fn script_fixture() -> (tempfile::TempDir, GuiApp) {
        use std::os::unix::fs::PermissionsExt;
        let (dir, mut app) = fixture();
        let script = dir.path().join("left/hello.sh");
        fs::write(
            &script,
            "#!/bin/sh\necho from-script \"$1\"\necho oops 1>&2\nexit 2\n",
        )
        .unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        app.session.reload_all();
        let at = app.session.panes[LEFT]
            .entries
            .iter()
            .position(|e| e.name == "hello.sh")
            .unwrap();
        app.session.panes[LEFT].cursor = at;
        (dir, app)
    }

    #[cfg(unix)]
    fn wait_for_command(app: &mut GuiApp) {
        let ctx = egui::Context::default();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while app.running.is_some() {
            assert!(
                std::time::Instant::now() < deadline,
                "command never finished"
            );
            app.poll_running(&ctx);
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[cfg(unix)]
    fn edit_fixture(gui_editor: &str) -> (tempfile::TempDir, GuiApp) {
        let (dir, mut app) = script_fixture();
        app.session.config.gui_editor = gui_editor.into();
        (dir, app)
    }

    #[cfg(unix)]
    #[test]
    fn edit_runs_the_gui_editor_quietly_when_it_succeeds() {
        // `echo` stands in for an editor that opens its own window and exits 0.
        let (_dir, mut app) = edit_fixture("echo opened");
        let ctx = egui::Context::default();
        app.run_action(&ctx, Action::Edit);
        assert!(app.running.is_some(), "runs in the background");
        wait_for_command(&mut app);
        assert!(app.viewer.is_none(), "no output window for a clean exit");
        let last = app.session.last_run.as_ref().unwrap();
        assert_eq!(last.command, "echo opened hello.sh");
        assert!(app.session.status().starts_with("edit:"));
    }

    #[cfg(unix)]
    #[test]
    fn edit_shows_the_output_when_the_editor_fails() {
        let (_dir, mut app) = edit_fixture("sh -c 'echo no display >&2; exit 3' sh");
        let ctx = egui::Context::default();
        app.run_action(&ctx, Action::Edit);
        wait_for_command(&mut app);
        let viewer = app.viewer.as_ref().expect("the failure is shown");
        assert!(
            viewer
                .preview
                .plain_lines()
                .join("\n")
                .contains("no display")
        );
    }

    #[test]
    fn edit_on_a_folder_is_an_error_not_a_launch() {
        let (dir, mut app) = fixture();
        fs::create_dir(dir.path().join("left/zz")).unwrap();
        app.session.reload_all();
        app.session.panes[LEFT].cursor = app.session.panes[LEFT]
            .entries
            .iter()
            .position(|e| e.name == "zz")
            .unwrap();
        let ctx = egui::Context::default();
        app.run_action(&ctx, Action::Edit);
        assert!(app.running.is_none());
        assert!(app.session.status().contains("folder"));
    }

    #[cfg(unix)]
    #[test]
    fn the_run_key_runs_any_program_in_the_current_folder() {
        let (dir, mut app) = script_fixture();
        let ctx = egui::Context::default();
        app.run_action(&ctx, Action::RunCommand);
        let dialog = app.dialog.clone().expect("run dialog");
        assert!(matches!(dialog.kind, DialogKind::Run));
        assert_eq!(dialog.text, "", "an empty line, not tied to the cursor");

        app.submit_dialog(Dialog {
            kind: DialogKind::Run,
            text: "pwd; echo arbitrary".into(),
        });
        wait_for_command(&mut app);
        let lines = app.viewer.as_ref().expect("output").preview.plain_lines();
        let here = fs::canonicalize(dir.path().join("left")).unwrap();
        assert_eq!(lines[0], here.display().to_string());
        assert_eq!(lines[1], "arbitrary");
    }

    #[cfg(unix)]
    #[test]
    fn enter_on_an_executable_opens_a_run_dialog_and_shows_the_output() {
        let (_dir, mut app) = script_fixture();
        app.open_cursor();
        let dialog = app.dialog.clone().expect("run dialog");
        assert!(matches!(dialog.kind, DialogKind::Run));
        assert_eq!(dialog.text, "./hello.sh");
        assert!(
            app.viewer.is_none() && app.running.is_none(),
            "nothing runs yet"
        );

        app.submit_dialog(Dialog {
            kind: DialogKind::Run,
            text: "./hello.sh edited".into(),
        });
        assert!(app.running.is_some(), "runs in the background");
        wait_for_command(&mut app);

        let viewer = app.viewer.as_ref().expect("output viewer");
        assert_eq!(viewer.preview.kind, PreviewKind::Output);
        assert_eq!(viewer.preview.plain_lines(), ["from-script edited", "oops"]);
        assert_eq!(
            viewer.preview.title(),
            "$ ./hello.sh edited  [output]  (ran: ./hello.sh edited — exit 2)"
        );
        assert!(app.session.message.as_deref().unwrap().contains("exit 2"));

        // Hex toggling does not apply: there is no file to re-read.
        app.run_viewer_action(ViewerAction::ToggleHex);
        assert_eq!(
            app.viewer.as_ref().unwrap().preview.kind,
            PreviewKind::Output
        );
        let output = run_frames(&mut app, 2);
        assert!(!output.shapes.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn a_second_command_is_refused_while_one_runs() {
        let (_dir, mut app) = script_fixture();
        app.submit_dialog(Dialog {
            kind: DialogKind::Run,
            text: "sleep 1".into(),
        });
        assert!(app.running.is_some());
        app.submit_dialog(Dialog {
            kind: DialogKind::Run,
            text: "echo second".into(),
        });
        assert!(
            app.session
                .message
                .as_deref()
                .unwrap()
                .contains("already running"),
            "{:?}",
            app.session.message
        );
        wait_for_command(&mut app);
        assert_eq!(
            app.viewer.as_ref().unwrap().preview.plain_lines(),
            ["(no output)"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn running_a_command_refreshes_the_panes() {
        let (dir, mut app) = script_fixture();
        app.submit_dialog(Dialog {
            kind: DialogKind::Run,
            text: "touch made-by-command".into(),
        });
        wait_for_command(&mut app);
        assert!(dir.path().join("left/made-by-command").exists());
        assert!(
            app.session.panes[LEFT]
                .entries
                .iter()
                .any(|e| e.name == "made-by-command")
        );
    }

    #[cfg(unix)]
    #[test]
    fn an_empty_command_is_refused_and_plain_files_still_open_the_viewer() {
        let (_dir, mut app) = script_fixture();
        app.submit_dialog(Dialog {
            kind: DialogKind::Run,
            text: "  ".into(),
        });
        assert!(app.running.is_none());
        assert!(app.session.message.as_deref().unwrap().contains("empty"));

        let at = app.session.panes[LEFT]
            .entries
            .iter()
            .position(|e| e.name == "a.txt")
            .unwrap();
        app.session.panes[LEFT].cursor = at;
        app.open_cursor();
        assert!(app.dialog.is_none());
        assert_eq!(
            app.viewer.as_ref().unwrap().preview.plain_lines(),
            ["hello"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_run_dialog_renders() {
        let (_dir, mut app) = script_fixture();
        app.open_cursor();
        let output = run_frames(&mut app, 2);
        assert!(!output.shapes.is_empty());
        assert!(app.dialog.is_some(), "stays open across frames");
    }

    #[cfg(unix)]
    #[test]
    fn ctrl_o_reopens_the_last_output_after_the_viewer_was_closed() {
        let (_dir, mut app) = script_fixture();
        let ctx = egui::Context::default();
        press(&ctx, &mut app, egui::Key::O, egui::Modifiers::CTRL);
        assert!(app.viewer.is_none());
        assert!(
            app.session
                .message
                .as_deref()
                .unwrap()
                .contains("no command has been run")
        );

        app.submit_dialog(Dialog {
            kind: DialogKind::Run,
            text: "./hello.sh again".into(),
        });
        wait_for_command(&mut app);
        assert!(
            app.viewer.is_some(),
            "output opens when the command finishes"
        );
        press(&ctx, &mut app, egui::Key::Q, egui::Modifiers::NONE);
        assert!(app.viewer.is_none());

        press(&ctx, &mut app, egui::Key::O, egui::Modifiers::CTRL);
        let viewer = app.viewer.as_ref().expect("Ctrl+O brings it back");
        assert_eq!(viewer.preview.plain_lines(), ["from-script again", "oops"]);
    }

    #[test]
    fn h_capital_toggles_the_hint_setting_without_breaking_the_desktop_ui() {
        let (_dir, mut app) = fixture();
        let ctx = egui::Context::default();
        press(&ctx, &mut app, egui::Key::H, egui::Modifiers::SHIFT);
        assert!(!app.session.config.show_hints);
        assert!(
            !run_frames(&mut app, 2).shapes.is_empty(),
            "the desktop has no hint line to hide"
        );
    }

    /// A dialog open and focused, with a few folders to complete.
    fn completion_dialog(
        kind: DialogKind,
        text: &str,
    ) -> (tempfile::TempDir, GuiApp, egui::Context) {
        let (dir, mut app) = fixture();
        for d in ["docs", "downloads", "src", "my dir"] {
            fs::create_dir(dir.path().join("left").join(d)).unwrap();
        }
        fs::write(dir.path().join("left/my file.txt"), "x").unwrap();
        app.session.reload_all();
        app.dialog = Some(Dialog {
            kind,
            text: text.into(),
        });
        let ctx = egui::Context::default();
        for _ in 0..3 {
            frame(&ctx, &mut app); // the text field gets focus and its Tab filter
        }
        (dir, app, ctx)
    }

    fn dialog_text(app: &GuiApp) -> String {
        app.dialog.as_ref().expect("dialog still open").text.clone()
    }

    #[test]
    fn up_and_down_recall_earlier_commands_in_the_run_dialog() {
        let (_dir, mut app, ctx) = completion_dialog(DialogKind::Run, "");
        for c in ["make test", "git status", "ls -l"] {
            app.session.history.push(c);
        }
        let (up, down) = (egui::Key::ArrowUp, egui::Key::ArrowDown);
        let none = egui::Modifiers::NONE;
        press(&ctx, &mut app, up, none);
        assert_eq!(dialog_text(&app), "ls -l");
        press(&ctx, &mut app, up, none);
        assert_eq!(dialog_text(&app), "git status");
        press(&ctx, &mut app, down, none);
        assert_eq!(dialog_text(&app), "ls -l");
        press(&ctx, &mut app, down, none);
        assert_eq!(dialog_text(&app), "", "back to what was typed");
        assert_eq!(
            app.session.active, LEFT,
            "arrows do not move the file cursor"
        );
        assert!(app.dialog.is_some());
    }

    #[test]
    fn recall_starts_over_after_the_text_is_edited_and_other_dialogs_ignore_arrows() {
        let (_dir, mut app, ctx) = completion_dialog(DialogKind::Run, "");
        app.session.history.push("one");
        app.session.history.push("two");
        let none = egui::Modifiers::NONE;
        press(&ctx, &mut app, egui::Key::ArrowUp, none);
        press(&ctx, &mut app, egui::Key::ArrowUp, none);
        assert_eq!(dialog_text(&app), "one");
        app.dialog.as_mut().unwrap().text = "one --flag".into(); // typed
        press(&ctx, &mut app, egui::Key::ArrowUp, none);
        assert_eq!(dialog_text(&app), "two", "restarts from the newest");

        let (_dir, mut app, ctx) = completion_dialog(DialogKind::GoTo, "src");
        app.session.history.push("one");
        press(&ctx, &mut app, egui::Key::ArrowUp, none);
        assert_eq!(dialog_text(&app), "src", "only the run line has history");
    }

    #[cfg(unix)]
    #[test]
    fn a_submitted_command_is_remembered() {
        let (_dir, mut app) = script_fixture();
        app.submit_dialog(Dialog {
            kind: DialogKind::Run,
            text: "  echo hi  ".into(),
        });
        wait_for_command(&mut app);
        assert_eq!(app.session.history.entries(), ["echo hi"]);
    }

    #[test]
    fn tab_completes_in_a_dialog_and_does_not_move_focus_or_switch_panes() {
        let (_dir, mut app, ctx) = completion_dialog(DialogKind::GoTo, "sr");
        press(&ctx, &mut app, egui::Key::Tab, egui::Modifiers::NONE);
        assert_eq!(dialog_text(&app), "src/");
        assert_eq!(
            app.session.active, LEFT,
            "Tab is not the switch-pane key inside a dialog"
        );
        // still focused: typing continues in the same field
        frame(&ctx, &mut app);
        assert!(app.dialog.is_some());
    }

    #[test]
    fn tab_and_shift_tab_cycle_through_candidates() {
        let (_dir, mut app, ctx) = completion_dialog(DialogKind::GoTo, "do");
        let none = egui::Modifiers::NONE;
        press(&ctx, &mut app, egui::Key::Tab, none);
        assert_eq!(dialog_text(&app), "docs/");
        press(&ctx, &mut app, egui::Key::Tab, none);
        assert_eq!(dialog_text(&app), "downloads/");
        press(&ctx, &mut app, egui::Key::Tab, none);
        assert_eq!(dialog_text(&app), "docs/", "wraps");
        press(&ctx, &mut app, egui::Key::Tab, egui::Modifiers::SHIFT);
        assert_eq!(dialog_text(&app), "downloads/", "Shift+Tab goes back");
        // the candidates are shown while the text is the one Tab produced
        let (shown_for, tab) = app.tab_view.clone().expect("a result to show");
        assert_eq!(shown_for, "downloads/");
        assert_eq!(tab.listing, ["docs/", "downloads/"]);
        assert_eq!(tab.current, Some(1));
    }

    #[test]
    fn the_run_dialog_completes_with_shell_escaping() {
        let (_dir, mut app, ctx) = completion_dialog(DialogKind::Run, "cat my");
        press(&ctx, &mut app, egui::Key::Tab, egui::Modifiers::NONE);
        assert_eq!(dialog_text(&app), "cat my\\ ");
        app.dialog.as_mut().unwrap().text.push('f');
        press(&ctx, &mut app, egui::Key::Tab, egui::Modifiers::NONE);
        assert_eq!(dialog_text(&app), "cat my\\ file.txt ");
    }

    #[test]
    fn filename_dialogs_complete_files_and_folders_but_go_to_only_folders() {
        let (_dir, mut app, ctx) = completion_dialog(DialogKind::Rename, "a");
        press(&ctx, &mut app, egui::Key::Tab, egui::Modifiers::NONE);
        assert_eq!(
            dialog_text(&app),
            "a.txt",
            "files complete in a name dialog"
        );

        let (_dir, mut app, ctx) = completion_dialog(DialogKind::GoTo, "a");
        press(&ctx, &mut app, egui::Key::Tab, egui::Modifiers::NONE);
        assert_eq!(dialog_text(&app), "a", "a.txt is not a folder");
        assert_eq!(
            app.tab_view.as_ref().unwrap().1.message.as_deref(),
            Some("no match")
        );
    }

    #[test]
    fn completion_lists_render_and_disappear_after_an_edit() {
        let (_dir, mut app, ctx) = completion_dialog(DialogKind::GoTo, "do");
        press(&ctx, &mut app, egui::Key::Tab, egui::Modifiers::NONE);
        let output = run_frames(&mut app, 2);
        assert!(!output.shapes.is_empty());
        // An edit makes the list stale: it is only shown for the text Tab produced.
        app.dialog.as_mut().unwrap().text.push('x');
        let live = app
            .tab_view
            .as_ref()
            .filter(|(shown_for, _)| *shown_for == app.dialog.as_ref().unwrap().text);
        assert!(live.is_none());
        assert!(!run_frames(&mut app, 2).shapes.is_empty());
    }

    #[test]
    fn tab_keeps_working_after_a_dialog_is_closed_and_reopened() {
        let (_dir, mut app, ctx) = completion_dialog(DialogKind::GoTo, "do");
        press(&ctx, &mut app, egui::Key::Tab, egui::Modifiers::NONE);
        press(&ctx, &mut app, egui::Key::Escape, egui::Modifiers::NONE);
        assert!(app.dialog.is_none());
        frame(&ctx, &mut app);
        // Back in the file list, Tab switches panes again.
        press(&ctx, &mut app, egui::Key::Tab, egui::Modifiers::NONE);
        assert_eq!(app.session.active, RIGHT);
    }
}
