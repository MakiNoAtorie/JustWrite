use crate::{
    APP_NAME,
    core::{
        commands::{self, COMMANDS, Command},
        config::{Session, SessionSplit, SessionTab, Settings, WindowState},
        prose::{self, BlockIndex},
        storage::{Entry, LocalStorage, ReadEvent, ReadSink, Storage, read_complete},
        workspace,
    },
    core::comments::Status as CommentStatus,
    jw::{self, manifest},
    platform::{self, Clipboard},
    render::{
        CpuRenderer, Rect, Renderer,
        theme::{Palette, color},
    },
    ui::{
        layout::{Divider, Pane, Split},
        menu::Popup,
        overlay::{Action, Kind, Overlay, Row},
        tab::Tab,
    },
};
#[path = "gui.rs"]
mod gui;
#[path = "project.rs"]
pub mod project;
#[path = "forms.rs"]
pub mod forms;
#[path = "screens.rs"]
pub mod screens;
#[path = "spelling.rs"]
pub mod spelling;
use forms::{ExportRequest, Form};
use project::{Panel, ProjectState};
use screens::{Screen, ScreenAct};
use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Motion, Shaping, fontdb};
use ropey::Rope;
use std::{
    collections::{BTreeMap, HashMap},
    num::NonZeroU32,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use winit::{
    application::ApplicationHandler,
    dpi::{LogicalSize, PhysicalPosition},
    event::{ElementState, Ime, MouseButton, MouseScrollDelta, StartCause, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoopProxy},
    keyboard::{Key, ModifiersState, NamedKey},
    window::{CursorIcon, Fullscreen, Theme, Window, WindowId},
};

pub enum UserEvent {
    Chosen(Option<PathBuf>, Choose),
    Read(u64, ReadEvent),
    Saved(u64, u64, Rope, Result<(), String>),
    Indexed(u64, u64, u64, BlockIndex),
    Fonts(fontdb::Database),
    Rows(u64, Vec<Row>),
    Config(Result<(Settings, Option<Session>), String>),
    Error(String),
    SessionSaved(Result<(), String>),
    Replaced(u64, u64, u64, Rope),
    ConfigWatch(notify::RecommendedWatcher),
    ConfigChanged,
    SettingsUpdated(Result<Settings, String>),
    /// A project finished opening, or could not be opened (`Err(Err(conflict))` means it is in use).
    ProjectOpened(PathBuf, Result<Box<crate::jw::project::Opened>, Result<String, crate::jw::lock::Conflict>>),
    /// A chapter file was written, with its word count.
    ChapterSaved(String, usize),
    /// The `.jw` was rewritten; `Err(None)` means it changed outside the program meanwhile.
    Packed(Result<(), Option<String>>),
    Exported(Result<PathBuf, String>),
    Preview(u64, Result<Arc<crate::export::typeset::Typeset>, String>),
    Imported(Result<Box<crate::import::split::Imported>, String>),
    Spelling(u64, u64, Vec<(usize, usize)>),
    SpellerLoaded(Result<Arc<crate::core::spell::Speller>, String>),
    ImportRead(PathBuf, Result<Box<crate::import::Raw>, String>),
    PreviewPage(u64, usize, u32, Arc<crate::export::typeset::Bitmap>),
}
#[derive(Clone, Copy)]
pub enum Choose {
    OpenProject,
    /// The title of the new project.
    NewProject,
    SaveProjectAs,
    ImportFile,
    Cover,
    /// Where to write an export.
    Export,
}
struct Scan {
    tab: u64,
    epoch: u64,
    revision: u64,
    rope: Option<Rope>,
    dirty: std::ops::Range<usize>,
    marker: crate::core::notes::Marker,
}
#[derive(Clone)]
enum Hit {
    Command(Command),
    Tab(u64, usize),
    Close(u64),
    /// A chapter, part or page in the project tree.
    Node(String),
    /// The arrow of a part.
    Fold(String),
    Scene(usize),
    Comment(String),
    Row(usize),
    TreeResize,
    OutlineResize,
    Scrollbar(u64),
    Divider(usize),
    Do(Act),
}
/// Clicks on custom-drawn controls; handled in `App::act`.
#[derive(Clone)]
enum Act {
    Menu(usize),
    Run(Command),
    Panel(Panel),
    Dismiss,
    Block,
    Press(&'static str),
    Field(bool),
    Case,
    Replace(bool),
    ClearShortcut,
    ResetShortcut,
    MatchStep(i32),
    ToggleReplace,
    Screen(ScreenAct),
    NewMenu,
    CommentDo(&'static str, String),
    CommentFilter(&'static str),
    OpenRecent(usize),
    FormFocus(usize),
    FormToggle(usize),
    FormSegment(usize, usize),
    FormDrop(usize),
    FormPick(usize, usize),
    FormStep(usize, i32),
    FormButton(usize),
    FormExtra(&'static str),
    FormSubmit,
    FormCloseDrop,
}
#[derive(Clone)]
enum Drag {
    Selection(u64),
    Tab(u64, u64, PhysicalPosition<f64>),
    Node(String, PhysicalPosition<f64>),
    TreeResize,
    OutlineResize,
    Scrollbar(u64),
    Divider(usize),
}
fn contains(rect: Rect, x: f64, y: f64) -> bool {
    x >= rect.x as f64
        && y >= rect.y as f64
        && x < rect.x as f64 + rect.width as f64
        && y < rect.y as f64 + rect.height as f64
}

/// The window size to open at, in logical pixels, for a screen of the given logical size: the
/// usual 1280 x 800, but never more than most of the screen, so that a taskbar or dock and the
/// window's own title bar still fit on a small laptop display.
fn default_window_size(screen_width: f64, screen_height: f64) -> (f64, f64) {
    ((screen_width * 0.9).clamp(640., 1280.), (screen_height * 0.85).clamp(420., 800.))
}

/// How long a status message stays in the footer before it fades.
const STATUS_LIFETIME: Duration = Duration::from_secs(10);

pub struct App {
    proxy: EventLoopProxy<UserEvent>,
    window: Option<Arc<Window>>,
    surface: Option<softbuffer::Surface<Arc<Window>, Arc<Window>>>,
    renderer: CpuRenderer,
    fonts: FontSystem,
    labels: HashMap<String, Buffer>,
    font_names: Vec<String>,
    settings: Settings,
    tabs: BTreeMap<u64, Tab>,
    split: Split,
    active_pane: u64,
    next_id: u64,
    scanner: mpsc::Sender<Scan>,
    /// The working copy of the open project; chapters are files below it.
    root: Option<PathBuf>,
    storage: Option<Arc<dyn Storage>>,
    project: Option<ProjectState>,
    tree_top: usize,
    outline_top: usize,
    show_tree: bool,
    show_outline: bool,
    tree_width: f32,
    outline_width: f32,
    closed: Vec<SessionTab>,
    overlay: Option<Overlay>,
    hits: Vec<(Rect, Hit)>,
    pane_rects: Vec<(u64, Rect)>,
    drag: Option<Drag>,
    mouse: PhysicalPosition<f64>,
    modifiers: ModifiersState,
    clipboard: Clipboard,
    preedit: String,
    status: String,
    /// The status text last seen and when it first appeared, so it can fade.
    status_seen: String,
    status_since: Instant,
    search_version: Arc<AtomicU64>,
    config_storage: Arc<dyn Storage>,
    config_watcher: Option<notify::RecommendedWatcher>,
    session_closing: bool,
    after_save: Option<Kind>,
    scale: f32,
    started: Instant,
    first_frame: bool,
    measuring: bool,
    popup: Option<Popup>,
    hover: Option<usize>,
    cursor_icon: CursorIcon,
    dividers: Vec<Divider>,
    widths: HashMap<String, f32>,
    in_modal: bool,
    preview_font: Option<String>,
    shortcut_key: String,
    tree_rect: Option<Rect>,
    outline_rect: Option<Rect>,
    clicks: (Instant, PhysicalPosition<f64>, u8),
    wheel: f32,
    session_due: Option<Instant>,
    system_light: bool,
    scale_override: Option<f32>,
    hover_since: Option<Instant>,
    recent: Vec<PathBuf>,
    saved_window: Option<WindowState>,
    file_type_registered: bool,
    spell: spelling::SpellState,
    pending_session: Option<SessionSplit>,
    /// Dialogs with several fields, and the large screens (history, compare, preview).
    form: Option<Form>,
    /// The export dialog, kept while its preview is on screen.
    parked_form: Option<Form>,
    screen: Option<Screen>,
    preview_version: u64,
    pending_export: Option<(ExportRequest, String)>,
    import_raw: Option<Arc<crate::import::Raw>>,
    import_into_new: bool,
    pending_import: Option<crate::import::split::Imported>,
    pending_jump: Option<String>,
}
/// Word around `pos` for double-click selection; scans at most 256 chars each way.
fn word_bounds(text: &Rope, pos: usize) -> (usize, usize) {
    let len = text.len_chars();
    let pos = pos.min(len);
    let word = |c: char| c.is_alphanumeric() || c == '_';
    let probe = if pos < len && text.char(pos) != '\n' {
        text.char(pos)
    } else if pos > 0 {
        text.char(pos - 1)
    } else {
        return (0, 0);
    };
    let class = word(probe);
    let same = |c: char| word(c) == class && !matches!(c, '\n' | '\r');
    let (mut start, mut end) = (pos, pos);
    while start > 0 && pos - start < 256 && same(text.char(start - 1)) {
        start -= 1;
    }
    while end < len && end - pos < 256 && same(text.char(end)) {
        end += 1;
    }
    (start, end)
}
/// The paragraph (run of non-blank lines) around `pos`, as a char range without its line ending.
fn paragraph_around(text: &Rope, pos: usize) -> std::ops::Range<usize> {
    let blank = |l: usize| text.line(l).chars().all(char::is_whitespace);
    let line = text.char_to_line(pos.min(text.len_chars()));
    if blank(line) {
        return pos..pos;
    }
    let (mut first, mut last) = (line, line);
    while first > 0 && !blank(first - 1) {
        first -= 1;
    }
    while last + 1 < text.len_lines() && !blank(last + 1) {
        last += 1;
    }
    let start = text.line_to_char(first);
    let mut end = if last + 1 < text.len_lines() { text.line_to_char(last + 1) } else { text.len_chars() };
    while end > start && matches!(text.char(end - 1), '\n' | '\r') {
        end -= 1;
    }
    start..end
}
impl App {
    pub fn new(proxy: EventLoopProxy<UserEvent>, started: Instant) -> Self {
        let (scanner, receiver) = mpsc::channel::<Scan>();
        let scan_proxy = proxy.clone();
        std::thread::spawn(move || {
            let mut indexes: HashMap<u64, BlockIndex> = HashMap::new();
            while let Ok(mut request) = receiver.recv() {
                let Some(rope) = request.rope else {
                    indexes.remove(&request.tab);
                    continue;
                };
                // Multiple documents share this worker, preserving each request's order.
                let blocks = indexes.entry(request.tab).or_default();
                if blocks.lines.is_empty() {
                    request.dirty = 0..rope.len_lines();
                }
                blocks.update_range(&rope, request.dirty, &request.marker);
                let _ = scan_proxy.send_event(UserEvent::Indexed(
                    request.tab,
                    request.epoch,
                    request.revision,
                    blocks.clone(),
                ));
            }
        });
        let config_storage: Arc<dyn Storage> =
            Arc::new(LocalStorage::new(platform::data_directory()));
        let proxy_for_spell = proxy.clone();
        let mut tabs = BTreeMap::new();
        tabs.insert(1, Tab::new(1));
        Self {
            proxy,
            window: None,
            surface: None,
            renderer: CpuRenderer::default(),
            fonts: platform::font_system(),
            labels: HashMap::new(),
            font_names: Vec::new(),
            settings: Settings::default(),
            tabs,
            split: Split::Pane(Pane {
                id: 2,
                tabs: vec![1],
                active: 0,
            }),
            active_pane: 2,
            next_id: 3,
            scanner,
            root: None,
            storage: None,
            project: None,
            tree_top: 0,
            outline_top: 0,
            show_tree: true,
            show_outline: true,
            tree_width: 232.,
            outline_width: 260.,
            closed: Vec::new(),
            overlay: None,
            hits: Vec::new(),
            pane_rects: Vec::new(),
            drag: None,
            mouse: PhysicalPosition::new(0., 0.),
            modifiers: ModifiersState::empty(),
            clipboard: Clipboard::default(),
            preedit: String::new(),
            status: String::new(),
            status_seen: String::new(),
            status_since: Instant::now(),
            search_version: Arc::new(AtomicU64::new(0)),
            config_storage,
            config_watcher: None,
            session_closing: false,
            after_save: None,
            scale: 1.,
            started,
            first_frame: false,
            measuring: std::env::args().any(|a| a == "--measure-startup"),
            popup: None,
            hover: None,
            cursor_icon: CursorIcon::Default,
            dividers: Vec::new(),
            widths: HashMap::new(),
            in_modal: false,
            preview_font: None,
            shortcut_key: String::new(),
            tree_rect: None,
            outline_rect: None,
            clicks: (Instant::now(), PhysicalPosition::new(0., 0.), 0),
            wheel: 0.,
            session_due: None,
            system_light: false,
            scale_override: std::env::var("JAME_SCALE").ok().and_then(|v| v.parse().ok()),
            hover_since: None,
            recent: Vec::new(),
            saved_window: None,
            file_type_registered: false,
            spell: spelling::SpellState::new(proxy_for_spell),
            pending_session: None,
            form: None,
            parked_form: None,
            screen: None,
            preview_version: 0,
            pending_export: None,
            import_raw: None,
            import_into_new: true,
            pending_import: None,
            pending_jump: None,
        }
    }
    fn id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
    fn active_id(&self) -> Option<u64> {
        self.split.pane(self.active_pane).and_then(Pane::tab)
    }
    fn redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
    /// Schedules a session save a few seconds from now, so a crash loses at most that much.
    fn dirty_session(&mut self) {
        if self.session_due.is_none() {
            self.session_due = Some(Instant::now() + Duration::from_secs(3));
        }
    }
    fn apply_window_theme(&self) {
        if let Some(window) = &self.window {
            window.set_theme(match self.settings.theme.as_str() {
                "light" => Some(Theme::Light),
                "dark" => Some(Theme::Dark),
                _ => None,
            });
        }
    }
    /// Takes settings loaded from disk, keeping the live Windows light/dark state.
    fn adopt(&mut self, mut settings: Settings) {
        settings.system_light = self.system_light;
        self.settings = settings;
    }
    fn title(&self) {
        if let Some(window) = &self.window {
            let project = self.project.as_ref().map(|p| p.core.title().to_owned());
            let title = match (self.active_id().and_then(|id| self.tabs.get(&id)).filter(|t| t.path.is_some()), project) {
                (Some(t), Some(project)) => format!("{}{} — {project} — {APP_NAME}", if t.document.dirty() { "● " } else { "" }, t.name()),
                (None, Some(project)) => format!("{project} — {APP_NAME}"),
                _ => APP_NAME.into(),
            };
            window.set_title(&title);
        }
    }
    fn scan(&self, id: u64, all: bool) {
        if let Some(tab) = self.tabs.get(&id) {
            let dirty = if all {
                0..tab.document.text.len_lines()
            } else {
                tab.document.changed.clone()
            };
            let _ = self.scanner.send(Scan {
                tab: id,
                epoch: tab.disk_epoch,
                revision: tab.document.revision,
                rope: Some(tab.document.text.clone()),
                dirty,
                marker: self.marker(),
            });
        }
    }
    fn edited(&mut self, id: u64) {
        self.dirty_session();
        self.follow_edits(id);
        if let Some(tab) = self.tabs.get_mut(&id) {
            tab.edited = Some(Instant::now());
            tab.view.ensure_cursor(&tab.document);
        }
        self.scan(id, false);
        self.status.clear();
        self.title();
        self.redraw();
    }
    fn new_tab(&mut self, pane: u64) -> u64 {
        let id = self.id();
        self.tabs.insert(id, Tab::new(id));
        if let Some(pane) = self.split.pane_mut(pane) {
            pane.tabs.push(id);
            pane.active = pane.tabs.len() - 1;
        }
        self.title();
        self.redraw();
        id
    }
    fn choose(&self, choice: Choose) {
        let proxy = self.proxy.clone();
        let parent = self.window.clone();
        let name = self.project.as_ref().map(|p| p.core.title().to_owned());
        let export_name = self.pending_export.as_ref().map(|(_, n)| n.clone()).unwrap_or_else(|| "manuscript".into());
        std::thread::spawn(move || {
            let jw: &[platform::Filter] = &[("JustWrite project", &["jw"])];
            let path = match choice {
                Choose::OpenProject => platform::choose_file(parent, "Open a project", jw),
                Choose::NewProject => platform::choose_save(parent, "Create a project", "My novel.jw", jw).map(|p| p.with_extension("jw")),
                Choose::SaveProjectAs => platform::choose_save(parent, "Save a copy as", &format!("{}.jw", name.unwrap_or_else(|| "My novel".into())), jw).map(|p| p.with_extension("jw")),
                Choose::ImportFile => platform::choose_file(parent, "Import a manuscript", &[("Manuscripts", &["docx", "md", "markdown", "txt"])]),
                Choose::Cover => platform::choose_file(parent, "Choose a cover image", &[("Images", &["jpg", "jpeg", "png"])]),
                Choose::Export => {
                    let extension = Path::new(&export_name).extension().and_then(|e| e.to_str()).unwrap_or("docx").to_owned();
                    platform::choose_save(parent, "Export to", &export_name, &[("Export", &[extension.as_str()])])
                }
            };
            let _ = proxy.send_event(UserEvent::Chosen(path, choice));
        });
    }
    fn open_relative(&mut self, path: PathBuf, position: usize) -> Option<u64> {
        if !workspace::valid_relative(&path) {
            self.status = "Invalid workspace-relative path".into();
            return None;
        }
        let path: PathBuf = path
            .components()
            .filter_map(|c| match c {
                std::path::Component::Normal(name) => Some(name),
                _ => None,
            })
            .collect();
        if let Some((&id, _)) = self.tabs.iter().find(|(_, tab)| {
            tab.path.as_ref() == Some(&path)
                && tab
                    .storage
                    .as_ref()
                    .zip(self.storage.as_ref())
                    .is_some_and(|(a, b)| Arc::ptr_eq(a, b))
        }) {
            let mut leaves = Vec::new();
            self.split.leaves(
                Rect {
                    x: 0,
                    y: 0,
                    width: 1,
                    height: 1,
                },
                &mut leaves,
            );
            for (pane, _) in leaves {
                if let Some(p) = self.split.pane_mut(pane) {
                    if let Some(index) = p.tabs.iter().position(|t| *t == id) {
                        p.active = index;
                        self.active_pane = pane;
                    }
                }
            }
            if let Some(tab) = self.tabs.get_mut(&id) {
                tab.document.select(position, false);
                tab.view.ensure_cursor(&tab.document);
            }
            self.title();
            self.redraw();
            return Some(id);
        }
        let storage = self.storage.clone()?;
        let reuse = self.active_id().filter(|id| {
            self.tabs
                .get(id)
                .is_some_and(|t| t.path.is_none() && t.document.text.len_chars() == 0)
        });
        let id = reuse.unwrap_or_else(|| self.new_tab(self.active_pane));
        let mut tab = Tab::new(id);
        tab.path = Some(path.clone());
        tab.root = self.root.clone();
        tab.storage = Some(storage.clone());
        tab.loading = true;
        tab.restore_cursor = Some(position);
        self.tabs.insert(id, tab);
        self.refresh_titles();
        let proxy = self.proxy.clone();
        storage.read(
            &path,
            Arc::new(move |event| {
                let _ = proxy.send_event(UserEvent::Read(id, event));
            }),
        );
        self.title();
        self.redraw();
        Some(id)
    }
    fn clear_tabs(&mut self) {
        for id in self.tabs.keys() {
            let _ = self.scanner.send(Scan {
                tab: *id,
                epoch: 0,
                revision: 0,
                rope: None,
                dirty: 0..0,
                marker: Default::default(),
            });
        }
        let old = std::mem::take(&mut self.tabs);
        std::thread::spawn(move || drop(old));
        let pane = self.id();
        self.split = Split::Pane(Pane {
            id: pane,
            tabs: Vec::new(),
            active: 0,
        });
        self.active_pane = pane;
        self.new_tab(pane);
    }
    fn save(&mut self, id: u64, _save_as: bool) {
        self.sync_marks_to_project(id);
        self.write_tab(id);
    }
    /// Starts writing a tab's text in the background. The marks are not written back to the
    /// project here: that converts the whole chapter and rewrites comments.json, with an fsync,
    /// on the interface thread. Autosave leaves them stale for the next flush, close or repack.
    fn write_tab(&mut self, id: u64) {
        let Some(tab) = self.tabs.get_mut(&id) else {
            return;
        };
        if tab.saving {
            return;
        }
        if tab.loading || tab.failed {
            self.status = "Saving is disabled until the whole chapter has loaded.".into();
            return;
        }
        let (Some(storage), Some(path)) = (&tab.storage, &tab.path) else {
            tab.edited = None;
            return;
        };
        let storage = storage.clone();
        let path = path.clone();
        let snapshot = tab.document.text.clone();
        let revision = tab.document.revision;
        let proxy = self.proxy.clone();
        tab.saving = true;
        tab.edited = None;
        std::thread::spawn(move || {
            let result = storage.write(&path, &snapshot).map_err(|e| e.to_string());
            let _ = proxy.send_event(UserEvent::Saved(id, revision, snapshot, result));
        });
    }
    fn close_tab(&mut self, id: u64, discard: bool) {
        if !discard && self.tabs.get(&id).is_some_and(|t| t.document.dirty()) {
            self.overlay = Some(Overlay::new(Kind::Close(id)));
            self.redraw();
            return;
        }
        if self.tabs.get(&id).is_some_and(|t| t.marks_stale) {
            self.sync_marks_to_project(id);
        }
        if let Some(tab) = self.tabs.remove(&id) {
            let _ = self.scanner.send(Scan {
                tab: id,
                epoch: 0,
                revision: 0,
                rope: None,
                dirty: 0..0,
                marker: Default::default(),
            });
            if let Some(path) = &tab.path {
                self.closed.push(SessionTab {
                    path: path.clone(),
                    cursor: tab.document.cursor,
                    top: tab.view.top,
                });
            }
            std::thread::spawn(move || drop(tab));
        }
        let mut leaves = Vec::new();
        self.split.leaves(
            Rect {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            },
            &mut leaves,
        );
        for (pane, _) in leaves {
            if let Some(p) = self.split.pane_mut(pane) {
                p.tabs.retain(|t| *t != id);
                p.active = p.active.min(p.tabs.len().saturating_sub(1));
            }
        }
        self.split.remove_empty();
        if self.split.pane(self.active_pane).is_none() {
            self.active_pane = self.split.first_pane();
        }
        if self.tabs.is_empty() {
            self.new_tab(self.active_pane);
        }
        self.title();
        self.redraw();
    }
    fn split_pane(&mut self, down: bool, move_tab: Option<(u64, u64)>) {
        self.split_pane_at(down, false, move_tab);
    }
    /// Where dropping a dragged tab over `rect` would split: (stacked vertically, new pane first).
    fn drop_zone(&self, rect: Rect, x: f64, y: f64) -> Option<(bool, bool)> {
        let edge = self.s(65.) as f64;
        let (left, top) = (rect.x as f64, rect.y as f64);
        let below_tabs = top + self.s(36.) as f64;
        let below_toolbar = top + self.s(74.) as f64;
        if y < below_tabs {
            None
        } else if x > left + rect.width as f64 - edge {
            Some((false, false))
        } else if x < left + edge {
            Some((false, true))
        } else if y > top + rect.height as f64 - edge {
            Some((true, false))
        } else if y > below_toolbar && y < below_toolbar + edge {
            Some((true, true))
        } else {
            None
        }
    }
    /// Slot a dragged tab would take: the gap between tabs nearest to `x`.
    fn tab_drop_index(&self, pane: &Pane, rect: Rect, x: f64) -> usize {
        let (first, _, width) = self.tab_geometry(pane, rect);
        let slot = ((x - rect.x as f64).max(0.) / width.max(1) as f64 + 0.5) as usize;
        (first + slot).min(pane.tabs.len())
    }
    fn split_pane_at(&mut self, down: bool, before: bool, move_tab: Option<(u64, u64)>) {
        let new_pane = self.id();
        let tab = if let Some((tab, source)) = move_tab {
            if let Some(p) = self.split.pane_mut(source) {
                p.tabs.retain(|t| *t != tab);
                p.active = p.active.min(p.tabs.len().saturating_sub(1));
            }
            tab
        } else {
            let tab = self.id();
            self.tabs.insert(tab, Tab::new(tab));
            tab
        };
        self.split.split_at(
            self.active_pane,
            Pane {
                id: new_pane,
                tabs: vec![tab],
                active: 0,
            },
            down,
            before,
        );
        self.active_pane = new_pane;
        self.split.remove_empty();
        self.title();
        self.redraw();
    }
    fn move_tab(&mut self, tab: u64, source: u64, target: u64, index: usize) {
        let index = if source == target
            && self
                .split
                .pane(source)
                .and_then(|p| p.tabs.iter().position(|id| *id == tab))
                .is_some_and(|old| old < index)
        {
            index.saturating_sub(1)
        } else {
            index
        };
        if let Some(p) = self.split.pane_mut(source) {
            p.tabs.retain(|id| *id != tab);
            p.active = p.active.min(p.tabs.len().saturating_sub(1));
        }
        if let Some(p) = self.split.pane_mut(target) {
            let index = index.min(p.tabs.len());
            p.tabs.insert(index, tab);
            p.active = index;
        }
        self.active_pane = target;
        self.split.remove_empty();
        self.title();
        self.redraw();
    }
    /// A menu or dialog that blocks the rest of the window (the docked find bar does not).
    fn modal(&self) -> bool {
        self.popup.is_some() || self.form.is_some() || self.overlay.as_ref().is_some_and(|o| !o.docked())
    }
    fn show(&mut self, kind: Kind) {
        let searching = matches!(kind, Kind::Find | Kind::Replace);
        let previous = self.overlay.take().filter(|o| o.docked() && searching);
        let mut overlay = Overlay::new(kind);
        if let Some(old) = previous {
            overlay.query = old.query;
            overlay.replacement = old.replacement;
            overlay.case = old.case;
        } else if searching {
            // Start from the selected text, like most editors.
            if let Some(tab) = self.active_id().and_then(|id| self.tabs.get(&id)) {
                let range = tab.document.selection();
                if !range.is_empty() && range.len() < 120 {
                    let text = tab.document.text.slice(range).to_string();
                    if !text.contains(['\n', '\r']) {
                        overlay.query = text;
                    }
                }
            }
        }
        overlay.select_all = !overlay.query.is_empty();
        self.overlay = Some(overlay);
        self.refresh_overlay();
        self.redraw();
    }
    fn refresh_overlay(&mut self) {
        let Some(overlay) = &mut self.overlay else {
            return;
        };
        let query = overlay.query.clone();
        let kind = overlay.kind.clone();
        let case = overlay.case;
        overlay.selected = 0;
        overlay.scroll = 0;
        overlay.rows.clear();
        match &kind {
            Kind::Palette | Kind::Keymap => {
                let mut rows: Vec<_> = COMMANDS
                    .iter()
                    .filter_map(|command| {
                        let score = workspace::fuzzy(&query, command.name)?;
                        let shortcut = self
                            .settings
                            .keymap
                            .get(command.key)
                            .map(String::as_str)
                            .unwrap_or(command.shortcut);
                        Some((
                            score,
                            Row {
                                label: command.name.into(),
                                hint: commands::display_shortcut(shortcut),
                                action: if kind == Kind::Keymap {
                                    Action::Shortcut(command.key.into())
                                } else {
                                    Action::Command(command.id)
                                },
                            },
                        ))
                    })
                    .collect();
                if !query.is_empty() {
                    rows.sort_by_key(|(score, _)| -score);
                }
                overlay.rows = rows.into_iter().map(|(_, row)| row).collect();
            }
            Kind::Recent => {
                let current = self.project.as_ref().map(|p| p.core.path.clone());
                overlay.rows = self
                    .recent
                    .iter()
                    .filter(|path| Some(*path) != current.as_ref() && path.is_file())
                    .filter(|path| workspace::fuzzy(&query, &path.to_string_lossy()).is_some())
                    .map(|path| Row {
                        label: path.file_stem().map_or_else(|| path.to_string_lossy().into_owned(), |n| n.to_string_lossy().into_owned()),
                        hint: path.to_string_lossy().into_owned(),
                        action: Action::Project(path.clone()),
                    })
                    .collect();
            }
            Kind::Setting(key) if key.ends_with("font") => {
                overlay.rows = self
                    .font_names
                    .iter()
                    .filter(|name| workspace::fuzzy(&query, name).is_some())
                    .take(100)
                    .map(|name| Row {
                        label: name.clone(),
                        hint: String::new(),
                        action: Action::Setting(format!("font:{key}:{name}")),
                    })
                    .collect();
            }
            Kind::Scenes => {
                let mut rows = Vec::new();
                let active = self.split.pane(self.active_pane).and_then(Pane::tab);
                if let Some(tab) = active.and_then(|id| self.tabs.get(&id)) {
                    let text = &tab.document.text;
                    let mut lines: Vec<usize> = vec![0];
                    lines.extend(tab.blocks.scenes.iter().copied());
                    for (n, line) in lines.into_iter().enumerate() {
                        if line >= text.len_lines() {
                            continue;
                        }
                        let snippet = (line..text.len_lines())
                            .skip(usize::from(n > 0))
                            .map(|l| text.line(l).to_string())
                            .map(|l| l.trim().to_owned())
                            .find(|l| !l.is_empty() && !prose::is_scene_break(l))
                            .unwrap_or_default();
                        let snippet: String = snippet.chars().take(90).collect();
                        if let Some(score) = workspace::fuzzy(&query, &snippet) {
                            rows.push((score, Row {
                                label: if snippet.is_empty() { "(empty scene)".into() } else { snippet },
                                hint: format!("Scene {} · line {}", n + 1, line + 1),
                                action: Action::Scene(line),
                            }));
                        }
                    }
                    if !query.is_empty() {
                        rows.sort_by_key(|(score, _)| -score);
                    }
                }
                overlay.rows = rows.into_iter().map(|(_, row)| row).collect();
            }
            Kind::QuickOpen => {
                let mut rows = Vec::new();
                if let Some(state) = &self.project {
                    let tree = &state.core.manifest.tree;
                    for (section, label) in [(manifest::Section::Manuscript, "Manuscript"), (manifest::Section::Notes, "Notes & research")] {
                        for (node, part) in tree.leaves(section) {
                            let Some(file) = node.file() else { continue };
                            let name = match part {
                                Some(part) => format!("{} · {}", part.title(), node.title()),
                                None => node.title().to_owned(),
                            };
                            if let Some(score) = workspace::fuzzy(&query, &name) {
                                rows.push((score, Row {
                                    label: node.title().to_owned(),
                                    hint: match part {
                                        Some(part) => format!("{label} · {}", part.title()),
                                        None => label.to_owned(),
                                    },
                                    action: Action::File(PathBuf::from(file), 0),
                                }));
                            }
                        }
                    }
                }
                if !query.is_empty() {
                    rows.sort_by_key(|(score, _)| -score);
                }
                overlay.rows = rows.into_iter().map(|(_, row)| row).collect();
            }
            Kind::Find | Kind::Replace | Kind::ProjectSearch => {
                let version = self.search_version.fetch_add(1, Ordering::Relaxed) + 1;
                let cancellation = self.search_version.clone();
                let proxy = self.proxy.clone();
                let storage = self.storage.clone();
                // Chapters and pages in reading order, with the titles shown beside matches.
                let files: Vec<(PathBuf, String)> = self
                    .project
                    .as_ref()
                    .map(|state| {
                        let tree = &state.core.manifest.tree;
                        [manifest::Section::Manuscript, manifest::Section::Notes]
                            .into_iter()
                            .flat_map(|s| tree.leaves(s))
                            .filter_map(|(n, _)| Some((PathBuf::from(n.file()?), n.title().to_owned())))
                            .collect()
                    })
                    .unwrap_or_default();
                let snapshot = self
                    .active_id()
                    .and_then(|id| self.tabs.get(&id))
                    .map(|tab| {
                        (
                            tab.document.text.clone(),
                            tab.path.clone().unwrap_or_default(),
                        )
                    });
                if let Some(overlay) = &mut self.overlay {
                    overlay.version = version;
                    overlay.pending = true;
                }
                std::thread::spawn(move || {
                    let mut rows = Vec::new();
                    match kind {
                        Kind::Find | Kind::Replace => {
                            if let Some((text, path)) = snapshot {
                                rows = workspace::find(&text, &query, case, 500)
                                    .into_iter()
                                    .map(|(start, end, line, preview)| Row {
                                        label: preview.clone(),
                                        hint: format!("Line {}", line + 1),
                                        action: Action::Match(workspace::Match {
                                            path: path.clone(),
                                            start,
                                            end,
                                            line,
                                            preview,
                                        }),
                                    })
                                    .collect();
                            }
                        }
                        _ => {
                            if let Some(storage) = storage {
                                if !query.is_empty() {
                                    for (path, title) in files.iter() {
                                        if cancellation.load(Ordering::Relaxed) != version {
                                            return;
                                        }
                                        if let Ok(text) = read_complete(&*storage, path) {
                                            for (start, end, line, preview) in
                                                workspace::find(&text, &query, case, 500 - rows.len())
                                            {
                                                rows.push(Row {
                                                    label: preview.clone(),
                                                    hint: format!("{title} · line {}", line + 1),
                                                    action: Action::Match(workspace::Match {
                                                        path: path.clone(),
                                                        start,
                                                        end,
                                                        line,
                                                        preview,
                                                    }),
                                                });
                                            }
                                        }
                                        if rows.len() >= 500 {
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if cancellation.load(Ordering::Relaxed) == version {
                        let _ = proxy.send_event(UserEvent::Rows(version, rows));
                    }
                });
            }
            _ => {}
        }
    }
    fn save_settings(&mut self) {
        let storage = self.config_storage.clone();
        let text = Rope::from_str(&self.settings.serialize());
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            if let Err(error) = storage.write(Path::new("settings.toml"), &text) {
                let _ =
                    proxy.send_event(UserEvent::Error(format!("Cannot save settings: {error}")));
            }
        });
        self.labels.clear();
        for tab in self.tabs.values_mut() {
            tab.view.clear_cache();
        }
        self.apply_window_theme();
        self.redraw();
    }
    fn activate_row(&mut self, row: Row) {
        match row.action {
            Action::Command(command) => {
                self.overlay = None;
                self.command(command);
            }
            Action::File(path, position) => {
                self.overlay = None;
                self.open_relative(path, position);
            }
            Action::Scene(line) => {
                if let Some(id) = self.active_id() {
                    if let Some(tab) = self.tabs.get_mut(&id) {
                        let position = tab
                            .document
                            .text
                            .line_to_char(line.min(tab.document.text.len_lines() - 1));
                        tab.document.select(position, false);
                        tab.view.top = position;
                    }
                }
                self.overlay = None;
            }
            Action::Match(found) => {
                let kind = self.overlay.as_ref().map(|o| o.kind.clone());
                if kind == Some(Kind::ProjectSearch) {
                    self.open_relative(found.path, found.start);
                    self.overlay = None;
                } else if let Some(id) = self.active_id() {
                    if let Some(tab) = self.tabs.get_mut(&id) {
                        tab.document.anchor = found.start.min(tab.document.text.len_chars());
                        tab.document.cursor = found.end.min(tab.document.text.len_chars());
                        tab.view.ensure_cursor(&tab.document);
                    }
                    if kind != Some(Kind::Replace) {
                        self.overlay = None;
                    }
                }
            }
            Action::Setting(key) => self.pick_font(&key),
            Action::Project(path) => {
                self.overlay = None;
                self.open_project(path, false);
            }
            Action::Shortcut(key) => {
                let query = self
                    .settings
                    .keymap
                    .get(&key)
                    .cloned()
                    .or_else(|| {
                        COMMANDS
                            .iter()
                            .find(|c| c.key == key)
                            .map(|c| c.shortcut.into())
                    })
                    .unwrap_or_default();
                let mut overlay = Overlay::new(Kind::Shortcut(key));
                overlay.query = query;
                self.overlay = Some(overlay);
            }
            Action::None => {}
        }
        self.title();
        self.redraw();
    }
    fn note(&mut self, text: &str) {
        if let Some(overlay) = &mut self.overlay {
            overlay.note = text.into();
        } else {
            self.status = text.into();
        }
    }
    fn close_overlay(&mut self) {
        self.overlay = None;
        self.after_save = None;
        self.status.clear();
        self.redraw();
    }
    fn step_match(&mut self, delta: i32) {
        let Some(overlay) = &mut self.overlay else {
            return;
        };
        let count = overlay.rows.len() as i32;
        if count > 0 {
            overlay.selected = (overlay.selected as i32 + delta).rem_euclid(count) as usize;
            overlay.reveal();
        }
        self.preview_match();
    }
    fn preview_match(&mut self) {
        let Some(overlay) = &self.overlay else {
            return;
        };
        if !matches!(overlay.kind, Kind::Find | Kind::Replace) {
            return;
        }
        let Some(Action::Match(found)) = overlay.rows.get(overlay.selected).map(|r| r.action.clone())
        else {
            return;
        };
        if let Some(tab) = self.active_id().and_then(|id| self.tabs.get_mut(&id)) {
            let len = tab.document.text.len_chars();
            tab.document.anchor = found.start.min(len);
            tab.document.cursor = found.end.min(len);
            tab.view.ensure_cursor(&tab.document);
        }
    }
    fn change(&mut self, edit: impl FnOnce(&mut Settings)) {
        let mut settings = self.settings.clone();
        edit(&mut settings);
        match Settings::parse(&settings.serialize()) {
            Ok(settings) => {
                self.adopt(settings);
                self.save_settings();
            }
            Err(error) => self.note(&error),
        }
    }
    fn open_font(&mut self, key: &str) {
        self.overlay = Some(Overlay::new(Kind::Setting(key.into())));
        self.refresh_overlay();
        self.redraw();
    }
    fn pick_font(&mut self, key: &str) {
        let parts: Vec<_> = key.splitn(3, ':').collect();
        if let [_, field, name] = parts[..] {
            let (field, name) = (field.to_owned(), name.to_owned());
            self.change(|s| match field.as_str() {
                "editor_font" => s.editor_font = name,
                "interface_font" => s.interface_font = name,
                _ => {}
            });
        }
        self.open_settings();
    }
    fn bind_shortcut(&mut self, command: &str, value: String) {
        let parsed = commands::parse_shortcut(&value).ok().flatten();
        let moved: Vec<_> = COMMANDS
            .iter()
            .filter(|c| c.key != command && parsed.is_some())
            .filter(|c| {
                let current = self.settings.keymap.get(c.key).map_or(c.shortcut, String::as_str);
                commands::parse_shortcut(current).ok().flatten() == parsed
            })
            .collect();
        for other in &moved {
            self.settings.keymap.insert(other.key.into(), String::new());
        }
        self.settings.keymap.insert(command.into(), value);
        self.save_settings();
        self.show(Kind::Keymap);
        if !moved.is_empty() {
            let names: Vec<_> = moved.iter().map(|c| c.name).collect();
            self.status = format!("Shortcut taken from: {}", names.join(", "));
        }
    }
    fn capture_shortcut(&mut self, command: String, key: &Key) -> bool {
        if matches!(key, Key::Named(NamedKey::Escape)) {
            self.show(Kind::Keymap);
            return true;
        }
        let modifier_only = matches!(
            key,
            Key::Named(
                NamedKey::Control
                    | NamedKey::Shift
                    | NamedKey::Alt
                    | NamedKey::AltGraph
                    | NamedKey::Super
                    | NamedKey::Meta
            )
        );
        let token = self.shortcut_key.trim_matches(' ').to_owned();
        if modifier_only || token.is_empty() {
            return true;
        }
        let (primary, shift, alt) = (
            platform::primary(self.modifiers),
            self.modifiers.shift_key(),
            self.modifiers.alt_key(),
        );
        let function_key = token.strip_prefix('f').is_some_and(|n| n.parse::<u8>().is_ok());
        if !(primary || alt || function_key) {
            self.note("Include Ctrl or Alt (or use a function key) so typing isn't affected.");
            self.redraw();
            return true;
        }
        self.bind_shortcut(&command, commands::format_shortcut(primary, shift, alt, &token));
        self.redraw();
        true
    }
    fn unsaved(&mut self, kind: Kind, discard: bool) {
        if discard {
            self.overlay = None;
            match kind {
                Kind::Close(id) => self.close_tab(id, true),
                Kind::Quit => self.persist_session(true),
                Kind::ChangeWorkspace(path) => {
                    self.shutdown_project();
                    self.clear_tabs();
                    self.open_project(path, false);
                }
                _ => {}
            }
        } else {
            self.after_save = Some(kind.clone());
            let ids: Vec<_> = match kind {
                Kind::Close(id) => vec![id],
                _ => self
                    .tabs
                    .iter()
                    .filter(|(_, t)| t.document.dirty())
                    .map(|(id, _)| *id)
                    .collect(),
            };
            for id in ids {
                self.save(id, false);
            }
        }
        self.redraw();
    }
    fn goto_line(&mut self, query: &str) {
        let Ok(line) = query.trim().parse::<usize>() else {
            self.note("Enter a line number.");
            return;
        };
        if let Some(tab) = self.active_id().and_then(|id| self.tabs.get_mut(&id)) {
            let text = &tab.document.text;
            let position = text.line_to_char(line.saturating_sub(1).min(text.len_lines() - 1));
            tab.document.select(position, false);
            tab.view.top = position;
        }
        self.overlay = None;
    }
    fn replace(&mut self, all: bool) {
        let Some(overlay) = &self.overlay else {
            return;
        };
        let (query, replacement, case, selected) = (
            overlay.query.clone(),
            overlay.replacement.clone(),
            overlay.case,
            overlay.selected,
        );
        let found = overlay.rows.get(selected).and_then(|r| match &r.action {
            Action::Match(m) => Some(m.clone()),
            _ => None,
        });
        if query.is_empty() {
            self.note("Enter text to find before replacing.");
            return;
        }
        let Some(id) = self.active_id() else {
            return;
        };
        let Some(tab) = self.tabs.get_mut(&id) else {
            return;
        };
        if all {
            let snapshot = tab.document.text.clone();
            let (revision, epoch) = (tab.document.revision, tab.disk_epoch);
            let proxy = self.proxy.clone();
            std::thread::spawn(move || {
                let text = workspace::replace_all(&snapshot.to_string(), &query, &replacement, case);
                let _ = proxy.send_event(UserEvent::Replaced(id, epoch, revision, Rope::from_str(&text)));
            });
            self.status = "Replacing all matches…".into();
            self.redraw();
            return;
        }
        let Some(found) = found else {
            self.note("Pick a match from the list first.");
            return;
        };
        let current = (found.end <= tab.document.text.len_chars() && found.start <= found.end)
            .then(|| tab.document.text.slice(found.start..found.end).to_string());
        let same = current.is_some_and(|text| {
            if case { text == query } else { text.eq_ignore_ascii_case(&query) }
        });
        if same {
            tab.document.replace(found.start..found.end, &replacement);
            self.edited(id);
            self.refresh_overlay();
            if let Some(overlay) = &mut self.overlay {
                overlay.selected = selected;
            }
        } else {
            self.note("The match changed. Search refreshed; try again.");
            self.refresh_overlay();
        }
    }
    fn press(&mut self, name: &str) {
        let kind = self.overlay.as_ref().map(|o| o.kind.clone());
        match (name, kind) {
            ("escape", _) => self.close_overlay(),
            ("enter", _) => {
                self.overlay_key(&Key::Named(NamedKey::Enter), None);
            }
            ("save" | "discard", Some(kind)) => self.unsaved(kind, name == "discard"),
            _ => {}
        }
    }
    fn act(&mut self, act: Act, rect: Rect) {
        match act {
            Act::Menu(index) => self.open_menu(index, rect),
            Act::Panel(panel) => {
                if let Some(state) = &mut self.project {
                    state.panel = panel;
                }
                self.show_outline = true;
            }
            Act::Run(command) => {
                self.popup = None;
                self.command(command);
            }
            Act::Dismiss => {
                if self.popup.take().is_none() {
                    if self.form.is_some() {
                        self.close_form();
                    } else {
                        self.close_overlay();
                    }
                }
            }
            Act::Block => {}
            Act::Press(name) => self.press(name),
            Act::Field(secondary) => {
                if let Some(o) = &mut self.overlay {
                    o.secondary = secondary;
                    o.caret = None;
                    o.focused = true;
                }
            }
            Act::MatchStep(delta) => self.step_match(delta),
            Act::ToggleReplace => {
                if let Some(o) = &mut self.overlay {
                    o.kind = if o.kind == Kind::Find { Kind::Replace } else { Kind::Find };
                    o.secondary = false;
                    o.focused = true;
                }
            }
            Act::Case => {
                if let Some(o) = &mut self.overlay {
                    o.case = !o.case;
                }
                self.refresh_overlay();
            }
            Act::Replace(all) => self.replace(all),
            Act::Screen(action) => self.screen_act(action),
            Act::NewMenu => self.open_popup(crate::ui::menu::NEW_MENU, rect.x, rect.y + rect.height as i32),
            Act::OpenRecent(index) => {
                if let Some(path) = self.recent.iter().filter(|r| r.is_file()).nth(index).cloned() {
                    self.open_project_checked(path);
                }
            }
            Act::CommentFilter(key) => self.comment_filter(key),
            Act::CommentDo(action, id) => self.comment_action(action, &id),
            Act::FormFocus(_)
            | Act::FormToggle(_)
            | Act::FormSegment(..)
            | Act::FormDrop(_)
            | Act::FormPick(..)
            | Act::FormStep(..)
            | Act::FormButton(_)
            | Act::FormExtra(_)
            | Act::FormSubmit
            | Act::FormCloseDrop => self.form_act(&act),
            Act::ClearShortcut | Act::ResetShortcut => {
                if let Some(Kind::Shortcut(key)) = self.overlay.as_ref().map(|o| o.kind.clone()) {
                    let value = if matches!(act, Act::ResetShortcut) {
                        COMMANDS.iter().find(|c| c.key == key).map_or("", |c| c.shortcut)
                    } else {
                        ""
                    };
                    self.bind_shortcut(&key, value.into());
                }
            }
        }
        self.redraw();
    }
    fn overlay_key(&mut self, key: &Key, text: Option<&str>) -> bool {
        let Some(overlay) = &self.overlay else {
            return false;
        };
        let kind = overlay.kind.clone();
        if overlay.docked() && !overlay.focused {
            if matches!(key, Key::Named(NamedKey::Escape)) {
                self.close_overlay();
                return true;
            }
            return false;
        }
        if let Kind::Shortcut(command) = &kind {
            return self.capture_shortcut(command.clone(), key);
        }
        let editable = overlay.has_input();
        let primary = platform::primary(self.modifiers);
        if primary && editable {
            if let Key::Character(character) = key {
                match character.to_lowercase().as_str() {
                    "a" => {
                        if let Some(overlay) = &mut self.overlay {
                            overlay.select_all = true;
                        }
                        self.redraw();
                        return true;
                    }
                    "v" => {
                        match self.clipboard.read() {
                            Ok(text) => {
                                if let Some(overlay) = &mut self.overlay {
                                    overlay.type_text(&text);
                                }
                                self.refresh_overlay();
                            }
                            Err(error) => self.status = error,
                        }
                        self.redraw();
                        return true;
                    }
                    "c" | "x" => {
                        let text = self
                            .overlay
                            .as_mut()
                            .map(|o| o.input().clone())
                            .unwrap_or_default();
                        if let Err(error) = self.clipboard.write(text) {
                            self.status = error;
                        } else if character.eq_ignore_ascii_case("x") {
                            if let Some(o) = &mut self.overlay {
                                o.input().clear();
                                o.select_all = false;
                            }
                            self.refresh_overlay();
                        }
                        self.redraw();
                        return true;
                    }
                    _ => {}
                }
            }
        }
        if matches!(key, Key::Named(NamedKey::Escape)) {
            self.close_overlay();
            return true;
        }
        if matches!(kind, Kind::Close(_) | Kind::Quit | Kind::ChangeWorkspace(_)) {
            match key {
                Key::Character(k) if k.eq_ignore_ascii_case("d") => self.unsaved(kind, true),
                Key::Character(k) if k.eq_ignore_ascii_case("s") => self.unsaved(kind, false),
                Key::Named(NamedKey::Enter) => self.unsaved(kind, false),
                _ => {}
            }
            self.redraw();
            return true;
        }
        if matches!(key, Key::Named(NamedKey::Enter)) {
            let query = overlay.query.clone();
            match kind {
                Kind::Delete(id) => {
                    self.overlay = None;
                    self.delete_node(&id);
                }
                Kind::GotoLine => self.goto_line(&query),
                Kind::Rename(id) => {
                    let title = query.trim().to_owned();
                    if title.is_empty() {
                        self.note("Enter a title.");
                    } else {
                        self.overlay = None;
                        self.rename_node(&id, &title);
                    }
                }
                Kind::Replace if primary || overlay.secondary => self.replace(primary),
                Kind::Find | Kind::Replace => {
                    let backwards = self.modifiers.shift_key();
                    self.step_match(if backwards { -1 } else { 1 });
                }
                _ => {
                    if let Some(row) = overlay.rows.get(overlay.selected).cloned() {
                        self.activate_row(row);
                    }
                }
            }
            self.redraw();
            return true;
        }
        let mut refresh = false;
        match key {
            Key::Named(NamedKey::ArrowDown | NamedKey::ArrowUp) => {
                let down = matches!(key, Key::Named(NamedKey::ArrowDown));
                if let Some(o) = &mut self.overlay {
                    o.selected = if down {
                        (o.selected + 1).min(o.rows.len().saturating_sub(1))
                    } else {
                        o.selected.saturating_sub(1)
                    };
                    o.reveal();
                }
                self.preview_match();
            }
            Key::Named(NamedKey::Tab) if kind == Kind::Replace => {
                if let Some(o) = &mut self.overlay {
                    o.secondary = !o.secondary;
                    o.caret = None;
                }
            }
            Key::Named(NamedKey::Backspace | NamedKey::Delete) if editable => {
                let back = matches!(key, Key::Named(NamedKey::Backspace));
                if let Some(o) = &mut self.overlay {
                    o.edit(back, true);
                }
                refresh = true;
            }
            Key::Named(NamedKey::ArrowLeft | NamedKey::ArrowRight) if editable => {
                let back = matches!(key, Key::Named(NamedKey::ArrowLeft));
                if let Some(o) = &mut self.overlay {
                    o.edit(back, false);
                }
            }
            Key::Named(NamedKey::Home) if editable => {
                if let Some(o) = &mut self.overlay {
                    o.select_all = false;
                    o.caret = Some(0);
                }
            }
            Key::Named(NamedKey::End) if editable => {
                if let Some(o) = &mut self.overlay {
                    o.select_all = false;
                    o.caret = None;
                }
            }
            Key::Character(_) | Key::Named(NamedKey::Space)
                if editable && !primary && !self.modifiers.alt_key() && self.preedit.is_empty() =>
            {
                let text = if matches!(key, Key::Named(NamedKey::Space)) { Some(text.unwrap_or(" ")) } else { text };
                if let Some(text) = text.filter(|s| !s.chars().any(char::is_control)) {
                    if let Some(o) = &mut self.overlay {
                        o.type_text(text);
                    }
                    refresh = true;
                }
            }
            _ => {}
        }
        if refresh {
            self.refresh_overlay();
        }
        self.redraw();
        true
    }
    fn command(&mut self, command: Command) {
        self.dirty_session();
        let active = self.active_id();
        match command {
            Command::NewProject => self.choose(Choose::NewProject),
            Command::OpenProject => self.choose(Choose::OpenProject),
            Command::Recent => self.show(Kind::Recent),
            Command::Import => self.choose(Choose::ImportFile),
            Command::Save => self.save_all(),
            Command::SaveAs => {
                if self.project.is_some() {
                    self.choose(Choose::SaveProjectAs);
                } else {
                    self.status = "Open or create a project first.".into();
                }
            }
            Command::CloseProject => {
                if self.tabs.values().any(|t| t.document.dirty()) {
                    self.save_all();
                    self.status = "Saving… close the project again once it is saved.".into();
                } else {
                    self.shutdown_project();
                    self.clear_tabs();
                    self.title();
                }
            }
            Command::NewChapter => self.new_node("chapter"),
            Command::NewPart => self.new_node("part"),
            Command::NewPage => self.new_node("page"),
            Command::Rename => match self.selected_item() {
                Some(id) => {
                    let mut overlay = Overlay::new(Kind::Rename(id.clone()));
                    overlay.query = self.node_title(&id).unwrap_or_default();
                    overlay.select_all = true;
                    self.overlay = Some(overlay);
                }
                None => self.status = "Click a chapter, part or page in the tree first.".into(),
            },
            Command::Delete => match self.selected_item() {
                Some(id) => self.overlay = Some(Overlay::new(Kind::Delete(id))),
                None => self.status = "Click a chapter, part or page in the tree first.".into(),
            },
            Command::MoveUp | Command::MoveDown => {
                let current = self.selected_item();
                if let Some(state) = &mut self.project {
                    if state.selected.is_none() {
                        state.selected = current;
                    }
                }
                self.move_selected(command == Command::MoveDown);
            }
            Command::CloseTab => {
                if let Some(id) = active {
                    self.close_tab(id, false);
                }
            }
            Command::Reopen => {
                if let Some(saved) = self.closed.pop() {
                    if let Split::Pane(restored) = self.restore_split(SessionSplit::Pane {
                        tabs: vec![saved],
                        active: 0,
                    }) {
                        if let Some(pane) = self.split.pane_mut(self.active_pane) {
                            pane.tabs.extend(restored.tabs);
                            pane.active = pane.tabs.len() - 1;
                        }
                    }
                }
            }
            Command::NextChapter | Command::PreviousChapter => {
                self.step_chapter(if command == Command::NextChapter { 1 } else { -1 });
            }
            Command::NextTab | Command::PreviousTab => {
                if let Some(pane) = self.split.pane_mut(self.active_pane) {
                    if !pane.tabs.is_empty() {
                        pane.active = if command == Command::NextTab {
                            (pane.active + 1) % pane.tabs.len()
                        } else {
                            (pane.active + pane.tabs.len() - 1) % pane.tabs.len()
                        };
                    }
                }
            }
            Command::Undo | Command::Redo => {
                if let Some(id) = active {
                    if let Some(tab) = self.tabs.get_mut(&id) {
                        if command == Command::Undo {
                            tab.document.undo();
                        } else {
                            tab.document.redo();
                        }
                    }
                    self.edited(id);
                    self.retry_orphans(id);
                }
            }
            Command::SelectAll => {
                if let Some(id) = active {
                    let tab = self.tabs.get_mut(&id).unwrap();
                    tab.document.anchor = 0;
                    tab.document.cursor = tab.document.text.len_chars();
                }
            }
            Command::Copy | Command::Cut => {
                if let Some(id) = active {
                    let tab = &self.tabs[&id];
                    let range = tab.document.selection();
                    if !range.is_empty() {
                        let text = tab.document.text.slice(range).to_string();
                        match self.clipboard.write(text) {
                            Ok(()) if command == Command::Cut => {
                                self.tabs.get_mut(&id).unwrap().document.insert("");
                                self.edited(id);
                            }
                            Ok(()) => {}
                            Err(error) => self.status = error,
                        }
                    }
                }
            }
            Command::Paste => {
                if let Some(id) = active {
                    match self.clipboard.read() {
                        Ok(text) => {
                            let tab = self.tabs.get_mut(&id).unwrap();
                            let text = text
                                .replace("\r\n", "\n")
                                .replace('\r', "\n")
                                .replace('\n', tab.document.line_ending);
                            tab.document.insert(&text);
                            self.edited(id);
                        }
                        Err(error) => self.status = error,
                    }
                }
            }
            Command::Bold | Command::Italic => {
                if let Some(id) = active {
                    let doc = &mut self.tabs.get_mut(&id).unwrap().document;
                    doc.toggle_emphasis(if command == Command::Bold { "**" } else { "*" });
                    self.edited(id);
                }
            }
            Command::ToggleNote | Command::NoteParagraph => {
                if let Some(id) = active {
                    let marker = self.marker();
                    let doc = &mut self.tabs.get_mut(&id).unwrap().document;
                    let selection = if command == Command::NoteParagraph { paragraph_around(&doc.text, doc.cursor) } else { doc.selection() };
                    match crate::core::notes::toggle(&doc.text, selection, &marker) {
                        Some(toggle) => {
                            doc.replace(toggle.range, &toggle.text);
                            doc.anchor = toggle.select.start;
                            doc.cursor = toggle.select.end;
                            self.edited(id);
                        }
                        None => self.status = "Nothing to turn into a note here.".into(),
                    }
                }
            }
            Command::AddComment => self.add_comment_form(),
            Command::AddWord => self.add_word(),
            Command::Palette => self.show(Kind::Palette),
            Command::QuickOpen => self.show(Kind::QuickOpen),
            Command::JumpScene => self.show(Kind::Scenes),
            Command::GotoLine => self.show(Kind::GotoLine),
            Command::WordCount => self.show_word_count(),
            Command::Find => self.show(Kind::Find),
            Command::Replace => self.show(Kind::Replace),
            Command::ProjectSearch => self.show(Kind::ProjectSearch),
            Command::SplitRight | Command::SplitDown => {
                self.split_pane(command == Command::SplitDown, None)
            }
            Command::CloseSplit => {
                let mut leaves = Vec::new();
                self.split.leaves(Rect { x: 0, y: 0, width: 1, height: 1 }, &mut leaves);
                let from = self.active_pane;
                if let Some(target) = leaves.iter().map(|(id, _)| *id).find(|id| *id != from) {
                    let moved = self.split.pane(from).map(|p| p.tabs.clone()).unwrap_or_default();
                    if let Some(pane) = self.split.pane_mut(from) {
                        pane.tabs.clear();
                    }
                    if let Some(pane) = self.split.pane_mut(target) {
                        pane.tabs.extend(moved);
                    }
                    self.split.remove_empty();
                    self.active_pane = target;
                } else {
                    self.status = "There is only one pane.".into();
                }
            }
            Command::ShowComments => {
                if let Some(state) = &mut self.project {
                    state.panel = Panel::Comments;
                }
                self.show_outline = true;
            }
            Command::ShowHistory => self.open_history(),
            Command::Snapshot => self.snapshot_form(),
            Command::Export => self.export_form(),
            Command::Preview => self.open_preview(),
            Command::BookDetails => self.book_form(),
            Command::ProjectSettings => self.project_form(),
            Command::ToggleTree => self.show_tree = !self.show_tree,
            Command::TogglePanel => self.show_outline = !self.show_outline,
            Command::ToggleMarkers => {
                self.settings.show_markers = !self.settings.show_markers;
                self.save_settings();
            }
            Command::HideNotes => {
                self.settings.hide_notes = !self.settings.hide_notes;
                self.save_settings();
                for id in self.tabs.keys().copied().collect::<Vec<_>>() {
                    self.scan(id, true);
                }
            }
            Command::Settings => self.open_settings(),
            Command::Keymap => self.show(Kind::Keymap),
            Command::Focus => {
                if let Some(window) = &self.window {
                    let focus = window.fullscreen().is_none();
                    window.set_decorations(!focus);
                    window.set_fullscreen(if focus {
                        Some(Fullscreen::Borderless(None))
                    } else {
                        None
                    });
                }
            }
            Command::Quit => {
                if self.tabs.values().any(|t| t.document.dirty()) {
                    self.show(Kind::Quit);
                } else {
                    self.persist_session(true);
                }
            }
        }
        self.title();
        self.redraw();
    }
    /// Keyboard control of an open menu; returns false when the key should fall through.
    fn popup_key(&mut self, key: &Key) -> bool {
        let Some(popup) = &self.popup else {
            return false;
        };
        let (items, menu, selected) = (popup.items, popup.menu, popup.selected);
        let usable: Vec<usize> = items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.is_some_and(|c| self.enabled(c)))
            .map(|(i, _)| i)
            .collect();
        let step = |forward: bool| {
            let at = selected.and_then(|s| usable.iter().position(|u| *u == s));
            let next = match (at, forward) {
                (None, true) => 0,
                (None, false) => usable.len().saturating_sub(1),
                (Some(i), true) => (i + 1) % usable.len().max(1),
                (Some(i), false) => (i + usable.len().max(1) - 1) % usable.len().max(1),
            };
            usable.get(next).copied()
        };
        match key {
            Key::Named(NamedKey::Escape) => self.popup = None,
            Key::Named(NamedKey::ArrowDown | NamedKey::ArrowUp) => {
                let forward = matches!(key, Key::Named(NamedKey::ArrowDown));
                let next = step(forward);
                if let Some(popup) = &mut self.popup {
                    popup.selected = next;
                }
            }
            Key::Named(NamedKey::ArrowLeft | NamedKey::ArrowRight) if menu.is_some() => {
                let count = crate::ui::menu::MENUS.len();
                let forward = matches!(key, Key::Named(NamedKey::ArrowRight));
                let target = (menu.unwrap() + if forward { 1 } else { count - 1 }) % count;
                let anchor = self
                    .hits
                    .iter()
                    .find(|(_, hit)| matches!(hit, Hit::Do(Act::Menu(m)) if *m == target))
                    .map(|(rect, _)| *rect);
                if let Some(rect) = anchor {
                    self.open_menu(target, rect);
                    if let Some(first) = self.popup.as_ref().and_then(|_| {
                        crate::ui::menu::MENUS[target]
                            .2
                            .iter()
                            .position(|item| item.is_some_and(|c| self.enabled(c)))
                    }) {
                        if let Some(popup) = &mut self.popup {
                            popup.selected = Some(first);
                        }
                    }
                }
            }
            Key::Named(NamedKey::Enter) => {
                let command = selected.and_then(|s| items.get(s).copied().flatten());
                self.popup = None;
                if let Some(command) = command {
                    self.command(command);
                }
            }
            Key::Named(NamedKey::Shift | NamedKey::Control | NamedKey::Alt | NamedKey::Super) => {}
            _ => {
                self.popup = None;
                self.redraw();
                return false;
            }
        }
        self.redraw();
        true
    }
    fn key(&mut self, key: Key, shortcut_key: String, text: Option<&str>) {
        if self.popup.is_some() && self.popup_key(&key) {
            return;
        }
        self.shortcut_key = shortcut_key.clone();
        if self.form.is_some() {
            self.form_key(&key, text);
            return;
        }
        if self.overlay_key(&key, text) {
            return;
        }
        if self.screen_key(&key) {
            return;
        }
        let primary = platform::primary(self.modifiers);
        let shift = self.modifiers.shift_key();
        if let Some(command) = commands::resolve(
            &shortcut_key,
            primary,
            shift,
            self.modifiers.alt_key(),
            &self.settings.keymap,
        ) {
            self.command(command);
            return;
        }
        if self.modifiers.alt_key() && !primary && !shift {
            let index = crate::ui::menu::MENUS
                .iter()
                .position(|(_, mnemonic, _)| shortcut_key == mnemonic.to_string());
            let anchor = index.and_then(|i| {
                self.hits
                    .iter()
                    .find(|(_, hit)| matches!(hit, Hit::Do(Act::Menu(m)) if *m == i))
                    .map(|(rect, _)| (i, *rect))
            });
            if let Some((i, rect)) = anchor {
                self.open_menu(i, rect);
                self.popup_key(&Key::Named(NamedKey::ArrowDown));
                self.redraw();
                return;
            }
        }
        if matches!(key, Key::Named(NamedKey::Escape)) {
            self.preedit.clear();
            if self
                .window
                .as_ref()
                .is_some_and(|w| w.fullscreen().is_some())
            {
                self.command(Command::Focus);
            }
            return;
        }
        if self.screen.is_some() || self.project.is_none() {
            return;
        }
        let Some(id) = self.active_id() else {
            return;
        };
        let typography = self
            .project
            .as_ref()
            .map(|p| (p.core.manifest.settings.typography.clone(), p.core.manifest.settings.language));
        let tab = self.tabs.get_mut(&id).unwrap();
        if tab.loading || tab.failed || self.project.is_none() {
            return;
        }
        let mut edited = false;
        match key {
            Key::Named(NamedKey::ArrowLeft) => {
                tab.document.horizontal(false, shift, primary);
                tab.view.ensure_cursor(&tab.document);
            }
            Key::Named(NamedKey::ArrowRight) => {
                tab.document.horizontal(true, shift, primary);
                tab.view.ensure_cursor(&tab.document);
            }
            Key::Named(NamedKey::ArrowUp | NamedKey::ArrowDown) => {
                let motion = if matches!(key, Key::Named(NamedKey::ArrowDown)) {
                    Motion::Down
                } else {
                    Motion::Up
                };
                let mut settings = self.settings.clone();
                settings.font_size *= self.scale;
                tab.view.motion(
                    &mut tab.document,
                    &tab.blocks,
                    &settings,
                    &mut self.fonts,
                    motion,
                    shift,
                );
                tab.view.ensure_cursor(&tab.document);
            }
            Key::Named(NamedKey::Home) => {
                let start = if primary {
                    0
                } else {
                    tab.document
                        .text
                        .line_to_char(tab.document.text.char_to_line(tab.document.cursor))
                };
                tab.document.select(start, shift);
                tab.view.ensure_cursor(&tab.document);
            }
            Key::Named(NamedKey::End) => {
                let line = tab.document.text.char_to_line(tab.document.cursor);
                let mut end = if primary || line + 1 == tab.document.text.len_lines() {
                    tab.document.text.len_chars()
                } else {
                    tab.document.text.line_to_char(line + 1)
                };
                if !primary {
                    while end > 0 && matches!(tab.document.text.char(end - 1), '\n' | '\r') {
                        end -= 1;
                    }
                }
                tab.document.select(end, shift);
                tab.view.ensure_cursor(&tab.document);
            }
            Key::Named(NamedKey::PageUp | NamedKey::PageDown) => {
                tab.view.scroll(
                    &tab.document,
                    if matches!(key, Key::Named(NamedKey::PageDown)) {
                        18
                    } else {
                        -18
                    },
                );
                tab.document.select(tab.view.top, shift);
            }
            Key::Named(NamedKey::Backspace) => {
                tab.document.delete(true);
                edited = true;
            }
            Key::Named(NamedKey::Delete) => {
                tab.document.delete(false);
                edited = true;
            }
            Key::Named(NamedKey::Enter) => {
                tab.document.enter();
                edited = true;
            }
            Key::Character(_) | Key::Named(NamedKey::Space)
                if !primary && !self.modifiers.alt_key() && self.preedit.is_empty() =>
            {
                let text = if matches!(key, Key::Named(NamedKey::Space)) { Some(text.unwrap_or(" ")) } else { text };
                if let Some(text) = text.filter(|t| !t.chars().any(char::is_control)) {
                    tab.document.type_text(text, typography.as_ref().map(|(t, l)| (t, *l)));
                    edited = true;
                }
            }
            _ => {}
        }
        if edited {
            self.edited(id);
        }
        self.redraw();
    }
    fn session_split(&self, split: &Split) -> SessionSplit {
        match split {
            Split::Pane(pane) => SessionSplit::Pane {
                active: pane.active,
                tabs: pane
                    .tabs
                    .iter()
                    .filter_map(|id| {
                        let tab = self.tabs.get(id)?;
                        Some(SessionTab {
                            path: tab.path.clone()?,
                            cursor: tab.document.cursor,
                            top: tab.view.top,
                        })
                    })
                    .collect(),
            },
            Split::Branch {
                down,
                ratio,
                first,
                second,
            } => SessionSplit::Split {
                down: *down,
                ratio: *ratio,
                first: Box::new(self.session_split(first)),
                second: Box::new(self.session_split(second)),
            },
        }
    }
    fn window_state(&self) -> Option<WindowState> {
        let window = self.window.as_ref()?;
        if window.fullscreen().is_some() || window.is_minimized().unwrap_or(false) {
            return self.saved_window;
        }
        let (position, size) = (window.outer_position().ok()?, window.inner_size());
        Some(WindowState {
            x: position.x,
            y: position.y,
            width: size.width,
            height: size.height,
            maximized: window.is_maximized(),
        })
    }
    fn persist_session(&mut self, close: bool) {
        if self.session_closing {
            return;
        }
        let session = Session {
            project: self.project.as_ref().map(|p| p.core.path.clone()),
            split: Some(self.session_split(&self.split)),
            tree_width: self.tree_width,
            outline_width: self.outline_width,
            show_tree: self.show_tree,
            show_outline: self.show_outline,
            recent: self.recent.clone(),
            window: self.window_state(),
            file_type: self.file_type_registered,
        };
        let text = match toml::to_string_pretty(&session) {
            Ok(text) => text,
            Err(error) => {
                self.status = error.to_string();
                return;
            }
        };
        if close {
            self.flush_tabs();
            self.shutdown_project();
        }
        let storage = self.config_storage.clone();
        let proxy = self.proxy.clone();
        self.session_closing = close;
        std::thread::spawn(move || {
            let result = storage
                .write(Path::new("session.toml"), &Rope::from_str(&text))
                .map_err(|e| e.to_string());
            let _ = proxy.send_event(UserEvent::SessionSaved(result));
        });
    }
    fn restore_split(&mut self, split: SessionSplit) -> Split {
        match split {
            SessionSplit::Pane { tabs, active } => {
                let pane = self.id();
                let mut ids = Vec::new();
                for saved in tabs {
                    let known = self
                        .project
                        .as_ref()
                        .is_some_and(|p| p.core.id_of_file(&saved.path.to_string_lossy().replace('\\', "/")).is_some());
                    if !known {
                        continue;
                    }
                    let id = self.id();
                    let mut tab = Tab::new(id);
                    tab.path = Some(saved.path.clone());
                    tab.root = self.root.clone();
                    tab.storage = self.storage.clone();
                    tab.loading = true;
                    tab.restore_cursor = Some(saved.cursor);
                    tab.view.top = saved.top;
                    if let Some(storage) = &tab.storage {
                        let proxy = self.proxy.clone();
                        storage.read(
                            &saved.path,
                            Arc::new(move |event| {
                                let _ = proxy.send_event(UserEvent::Read(id, event));
                            }),
                        );
                    }
                    self.tabs.insert(id, tab);
                    ids.push(id);
                }
                if ids.is_empty() {
                    let id = self.id();
                    self.tabs.insert(id, Tab::new(id));
                    ids.push(id);
                }
                self.refresh_titles();
                Split::Pane(Pane {
                    id: pane,
                    active: active.min(ids.len() - 1),
                    tabs: ids,
                })
            }
            SessionSplit::Split {
                down,
                ratio,
                first,
                second,
            } => Split::Branch {
                down,
                ratio: if ratio.is_finite() {
                    ratio.clamp(0.15, 0.85)
                } else {
                    0.5
                },
                first: Box::new(self.restore_split(*first)),
                second: Box::new(self.restore_split(*second)),
            },
        }
    }
    fn autosave(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        let mut next = None;
        let mut save = Vec::new();
        if self.settings.autosave {
            for (&id, tab) in &self.tabs {
                if !tab.loading
                    && !tab.failed
                    && !tab.saving
                    && tab.path.is_some()
                    && tab.document.dirty()
                {
                    if let Some(edited) = tab.edited {
                        let deadline = edited + Duration::from_millis(700);
                        if deadline <= now {
                            save.push(id);
                        } else {
                            next =
                                Some(next.map_or(deadline, |other: Instant| other.min(deadline)));
                        }
                    }
                }
            }
        }
        if self.status != self.status_seen {
            self.status_seen = self.status.clone();
            self.status_since = now;
        }
        if !self.status.is_empty() {
            let due = self.status_since + STATUS_LIFETIME;
            if due <= now {
                self.status.clear();
                self.status_seen.clear();
                self.redraw();
            } else {
                next = Some(next.map_or(due, |other: Instant| other.min(due)));
            }
        }
        if let Some(since) = self.hover_since {
            let due = since + Duration::from_millis(450);
            if due > now {
                next = Some(next.map_or(due, |other: Instant| other.min(due)));
            }
        }
        if let Some(due) = self.session_due {
            if due <= now {
                self.session_due = None;
                self.persist_session(false);
            } else {
                next = Some(next.map_or(due, |other: Instant| other.min(due)));
            }
        }
        if let Some(due) = self.project_tick(now) {
            next = Some(next.map_or(due, |other: Instant| other.min(due)));
        }
        for id in save {
            if let Some(tab) = self.tabs.get_mut(&id) {
                tab.marks_stale |= !tab.marks.is_empty();
            }
            self.write_tab(id);
        }
        event_loop.set_control_flow(next.map_or(ControlFlow::Wait, ControlFlow::WaitUntil));
    }
    fn frame(&mut self, event_loop: &ActiveEventLoop) {
        let frame_started = Instant::now();
        let Some(window) = self.window.clone() else {
            return;
        };
        let size = window.inner_size();
        if size.width == 0 || size.height == 0 {
            return;
        }
        self.scale = self.scale_override.unwrap_or(window.scale_factor() as f32);
        self.renderer.resize(size.width, size.height);
        self.hits.clear();
        self.pane_rects.clear();
        let palette = Palette::new(self.settings.light(), self.settings.accent_color());
        let full = Rect {
            x: 0,
            y: 0,
            width: size.width,
            height: size.height,
        };
        self.renderer.fill(full, palette.background);
        let focus = window.fullscreen().is_some();
        let menubar = if focus { 0 } else { self.su(30.) };
        let footer = if focus { 0 } else { self.su(28.) };
        let body = size.height.saturating_sub(footer + menubar);
        let logical_width = size.width as f32 / self.scale;
        let on_screen = self.screen.is_some();
        let has_project = self.project.is_some();
        let tree = if !focus && !on_screen && has_project && self.show_tree && logical_width > 640. {
            (self.tree_width * self.scale) as u32
        } else {
            0
        };
        let outline = if !focus && !on_screen && has_project && self.show_outline && logical_width > 820. {
            (self.outline_width * self.scale) as u32
        } else {
            0
        };
        self.tree_rect = None;
        self.outline_rect = None;
        if !focus {
            self.paint_menubar(Rect { height: menubar, ..full }, palette);
        }
        if tree > 0 {
            let rect = Rect { x: 0, y: menubar as i32, width: tree, height: body };
            self.tree_rect = Some(rect);
            self.paint_tree(rect, palette);
        }
        if outline > 0 {
            let rect = Rect {
                x: size.width as i32 - outline as i32,
                y: menubar as i32,
                width: outline,
                height: body,
            };
            self.outline_rect = Some(rect);
            self.paint_panel(rect, palette);
        }
        let center = Rect {
            x: tree as i32,
            y: menubar as i32,
            width: size.width.saturating_sub(tree + outline).max(1),
            height: body,
        };
        if on_screen {
            self.dividers.clear();
            self.paint_screen(center, palette);
        } else if self.project.is_none() {
            self.dividers.clear();
            self.paint_welcome(center, palette);
        } else {
            if focus {
                self.pane_rects.push((self.active_pane, center));
                self.dividers.clear();
            } else {
                self.split.leaves(center, &mut self.pane_rects);
                self.dividers_for(center);
            }
            for (pane, rect) in self.pane_rects.clone() {
                self.paint_pane(pane, rect, focus, palette);
            }
        }
        if focus {
            let hint = Rect {
                x: size.width as i32 - self.s(230.),
                y: size.height as i32 - self.s(26.),
                width: self.su(220.),
                height: self.su(22.),
            };
            self.put("Esc exits focus mode", hint, 11., palette.muted, gui::Face::Normal, gui::Align::Right);
        }
        if !focus {
            self.paint_dividers(palette);
            self.paint_footer(
                Rect { y: (size.height - footer) as i32, height: footer, ..full },
                palette,
            );
        }
        if !self.preedit.is_empty() {
            let preedit = self.preedit.clone();
            let position = self
                .active_id()
                .and_then(|id| self.tabs.get(&id))
                .and_then(|tab| {
                    tab.view.caret_at(tab.document.cursor).map(|(x, y)| (tab.view.rect.unwrap(), x, y))
                });
            if let Some((rect, x, y)) = position {
                self.label(
                    &preedit,
                    Rect {
                        x: rect.x + x as i32,
                        y: rect.y + y as i32,
                        width: 300,
                        height: 32,
                    },
                    self.settings.font_size * self.scale,
                    palette.accent,
                );
            }
        }
        self.paint_overlay(full, menubar as i32, palette);
        self.paint_form(full, palette);
        self.paint_popup(full, palette);
        self.paint_tooltip(full, palette);
        if let Some(Drag::Tab(tab, _, origin)) = self.drag.clone() {
            if (self.mouse.x - origin.x).abs() + (self.mouse.y - origin.y).abs() > 8. {
                self.paint_tab_ghost(tab, palette);
                if let Some((_, rect)) = self
                    .pane_rects
                    .iter()
                    .find(|(_, rect)| contains(*rect, self.mouse.x, self.mouse.y))
                {
                    if let Some((down, before)) = self.drop_zone(*rect, self.mouse.x, self.mouse.y) {
                        let thick = self.su(6.);
                        let top = rect.y + self.s(74.);
                        let strip = match (down, before) {
                            (false, false) => Rect { x: rect.x + rect.width as i32 - thick as i32, width: thick, ..*rect },
                            (false, true) => Rect { x: rect.x, width: thick, ..*rect },
                            (true, false) => Rect { y: rect.y + rect.height as i32 - thick as i32, height: thick, ..*rect },
                            (true, true) => Rect { y: top, height: thick, ..*rect },
                        };
                        self.renderer.fill(strip, palette.accent);
                    }
                }
            }
        }
        // ponytail: event-driven frames repaint the viewport; narrow invalidation if measured frame budgets require it.
        let damage: Vec<_> = self
            .renderer
            .damage()
            .into_iter()
            .filter_map(|rect| {
                Some(softbuffer::Rect {
                    x: rect.x as u32,
                    y: rect.y as u32,
                    width: NonZeroU32::new(rect.width)?,
                    height: NonZeroU32::new(rect.height)?,
                })
            })
            .collect();
        if let Some(surface) = &mut self.surface {
            let result = (|| {
                surface.resize(
                    NonZeroU32::new(size.width).unwrap(),
                    NonZeroU32::new(size.height).unwrap(),
                )?;
                let mut buffer = surface.buffer_mut()?;
                buffer.copy_from_slice(self.renderer.pixels());
                buffer.present_with_damage(&damage)
            })();
            if let Err(error) = result {
                self.status = format!("Render failed: {error}");
            }
        }
        if let Some(id) = self.active_id() {
            let tab = &self.tabs[&id];
            if let Some((x, y)) = tab.view.caret_at(tab.document.cursor) {
                if let Some(rect) = tab.view.rect {
                    window.set_ime_cursor_area(
                        PhysicalPosition::new(rect.x as f64 + x as f64, rect.y as f64 + y as f64),
                        LogicalSize::new(2., self.settings.font_size * 1.65),
                    );
                }
            }
        }
        if self.labels.len() > 2048 {
            self.labels.clear();
        }
        if !self.first_frame {
            self.first_frame = true;
            if self.measuring {
                println!(
                    "first_frame_ms={:.3},processing_ms={:.3}",
                    self.started.elapsed().as_secs_f64() * 1000.,
                    frame_started.elapsed().as_secs_f64() * 1000.
                );
                event_loop.exit();
            } else {
                let proxy = self.proxy.clone();
                std::thread::spawn(move || {
                    let _ = proxy.send_event(UserEvent::Fonts(platform::fallback_fonts()));
                });
                let storage = self.config_storage.clone();
                let proxy = self.proxy.clone();
                std::thread::spawn(move || {
                    let settings = match read_complete(&*storage, Path::new("settings.toml")) {
                        Ok(text) => Settings::parse(&text.to_string()),
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                            let mut settings = Settings::default();
                            settings.keymap = COMMANDS
                                .iter()
                                .map(|c| (c.key.into(), c.shortcut.into()))
                                .collect();
                            storage
                                .write(
                                    Path::new("settings.toml"),
                                    &Rope::from_str(&settings.serialize()),
                                )
                                .map(|_| settings)
                                .map_err(|error| error.to_string())
                        }
                        Err(error) => Err(error.to_string()),
                    };
                    let session = read_complete(&*storage, Path::new("session.toml"))
                        .ok()
                        .and_then(|text| toml::from_str::<Session>(&text.to_string()).ok());
                    let _ = proxy.send_event(UserEvent::Config(
                        settings.map(|settings| (settings, session)),
                    ));
                    let watch_proxy = proxy.clone();
                    if let Ok(watcher) = storage.watch(Arc::new(move |path| {
                        if path == Path::new("settings.toml") {
                            let _ = watch_proxy.send_event(UserEvent::ConfigChanged);
                        }
                    })) {
                        let _ = proxy.send_event(UserEvent::ConfigWatch(watcher));
                    }
                });
            }
        }
    }
    fn hit_at(&self, x: f64, y: f64) -> Option<(Rect, Hit)> {
        self.hits
            .iter()
            .rev()
            .find(|(rect, _)| contains(*rect, x, y))
            .map(|(rect, hit)| (*rect, hit.clone()))
    }
    fn over_editor(&self, x: f64, y: f64) -> bool {
        self.pane_rects.iter().any(|(pane, rect)| {
            contains(*rect, x, y)
                && self
                    .split
                    .pane(*pane)
                    .and_then(Pane::tab)
                    .and_then(|id| self.tabs.get(&id))
                    .and_then(|tab| tab.view.rect)
                    .is_some_and(|view| y >= view.y as f64 - self.s(27.) as f64)
        })
    }
    fn editor_click(&mut self, x: f64, y: f64) {
        if !self.over_editor(x, y) {
            return;
        }
        if let Some(overlay) = self.overlay.as_mut().filter(|o| o.docked()) {
            overlay.focused = false;
        }
        let Some(id) = self.active_id() else {
            return;
        };
        let now = Instant::now();
        let (last, at, count) = self.clicks;
        let near = (at.x - x).abs() + (at.y - y).abs() < 8.;
        let count = if near && now.duration_since(last) < Duration::from_millis(450) {
            count % 3 + 1
        } else {
            1
        };
        self.clicks = (now, self.mouse, count);
        let shift = self.modifiers.shift_key();
        let Some(tab) = self.tabs.get_mut(&id) else {
            return;
        };
        let Some(cursor) = tab.view.hit(x as f32, y as f32) else {
            return;
        };
        let clicked_comment = if count == 1 { tab.marks.iter().find(|m| !m.orphaned && cursor >= m.start && cursor < m.end).map(|m| m.id.clone()) } else { None };
        match count {
            1 => tab.document.select(cursor, shift),
            2 => {
                let (start, end) = word_bounds(&tab.document.text, cursor);
                tab.document.anchor = start;
                tab.document.cursor = end;
            }
            _ => {
                let text = &tab.document.text;
                let line = text.char_to_line(cursor.min(text.len_chars()));
                tab.document.anchor = text.line_to_char(line);
                tab.document.cursor = if line + 1 < text.len_lines() {
                    text.line_to_char(line + 1)
                } else {
                    text.len_chars()
                };
            }
        }
        self.drag = Some(Drag::Selection(id));
        if let (Some(comment), Some(state)) = (clicked_comment, &mut self.project) {
            state.active_comment = Some(comment);
            state.panel = Panel::Comments;
            self.sync_view_marks(id);
        }
    }
    fn mouse_down(&mut self, button: MouseButton) {
        self.dirty_session();
        self.hover_since = None;
        let (x, y) = (self.mouse.x, self.mouse.y);
        let blocked = self.modal();
        if !blocked {
            if let Some((pane, _)) = self.pane_rects.iter().find(|(_, rect)| contains(*rect, x, y)) {
                self.active_pane = *pane;
            }
        }
        let found = self.hit_at(x, y);
        let hit = found.as_ref().map(|(_, hit)| hit.clone());
        if button == MouseButton::Right {
            if blocked {
                self.popup = None;
            } else {
                match &hit {
                    Some(Hit::Tab(pane, index)) => {
                        if let Some(p) = self.split.pane_mut(*pane) {
                            p.active = *index;
                        }
                    }
                    Some(Hit::Node(id)) => {
                        if let Some(state) = &mut self.project {
                            state.selected = Some(id.clone());
                        }
                    }
                    None if self.over_editor(x, y) => {
                        let empty = self
                            .active_id()
                            .and_then(|id| self.tabs.get(&id))
                            .is_none_or(|t| t.document.selection().is_empty());
                        if empty {
                            self.editor_click(x, y);
                            self.drag = None;
                        }
                    }
                    _ => {}
                }
                self.popup_for(hit.as_ref());
            }
            self.title();
            self.redraw();
            return;
        }
        let rect = found.map_or(
            Rect { x: 0, y: 0, width: 0, height: 0 },
            |(rect, _)| rect,
        );
        match hit {
            Some(Hit::Do(act)) => self.act(act, rect),
            Some(Hit::Command(command)) => self.command(command),
            Some(Hit::Tab(pane, index)) => {
                if let Some(p) = self.split.pane_mut(pane) {
                    p.active = index;
                    self.active_pane = pane;
                    if let Some(tab) = p.tab() {
                        if button == MouseButton::Middle {
                            self.close_tab(tab, false);
                        } else {
                            self.drag = Some(Drag::Tab(tab, pane, self.mouse));
                        }
                    }
                }
            }
            Some(Hit::Close(id)) => self.close_tab(id, false),
            Some(Hit::Node(id)) => {
                if let Some(state) = &mut self.project {
                    state.selected = Some(id.clone());
                }
                let is_part = self.project.as_ref().and_then(|p| p.core.manifest.tree.find(&id)).is_some_and(manifest::Node::is_part);
                if is_part {
                    self.toggle_fold(&id);
                } else {
                    self.open_item(&id);
                }
                self.drag = Some(Drag::Node(id, self.mouse));
            }
            Some(Hit::Fold(id)) => self.toggle_fold(&id),
            Some(Hit::Scene(line)) => {
                self.activate_row(Row { label: String::new(), hint: String::new(), action: Action::Scene(line) });
            }
            Some(Hit::Comment(id)) => self.jump_to_comment(&id),
            Some(Hit::Row(index)) => {
                if let Some(o) = &mut self.overlay {
                    o.selected = index;
                }
                if let Some(row) = self
                    .overlay
                    .as_ref()
                    .and_then(|o| o.rows.get(index))
                    .cloned()
                {
                    self.activate_row(row);
                }
            }
            Some(Hit::TreeResize) => self.drag = Some(Drag::TreeResize),
            Some(Hit::OutlineResize) => self.drag = Some(Drag::OutlineResize),
            Some(Hit::Divider(index)) => self.drag = Some(Drag::Divider(index)),
            Some(Hit::Scrollbar(id)) => {
                self.drag = Some(Drag::Scrollbar(id));
                self.mouse_move(self.mouse);
            }
            None => self.editor_click(x, y),
        }
        self.title();
        self.redraw();
    }
    fn update_cursor(&mut self) {
        let icon = match self.hover.and_then(|i| self.hits.get(i)).map(|(_, hit)| hit) {
            Some(Hit::TreeResize | Hit::OutlineResize) => CursorIcon::ColResize,
            Some(Hit::Divider(i)) if self.dividers.get(*i).is_some_and(|d| d.down) => {
                CursorIcon::RowResize
            }
            Some(Hit::Divider(_)) => CursorIcon::ColResize,
            Some(Hit::Do(Act::Field(_))) => CursorIcon::Text,
            Some(Hit::Do(Act::Block | Act::Dismiss)) => CursorIcon::Default,
            Some(_) => CursorIcon::Pointer,
            None if !self.modal() && self.over_editor(self.mouse.x, self.mouse.y) => {
                CursorIcon::Text
            }
            None => CursorIcon::Default,
        };
        if icon != self.cursor_icon {
            self.cursor_icon = icon;
            if let Some(window) = &self.window {
                window.set_cursor(icon);
            }
        }
    }
    fn mouse_move(&mut self, position: PhysicalPosition<f64>) {
        self.mouse = position;
        match self.drag.clone() {
            Some(Drag::Selection(id)) => {
                if let Some(tab) = self.tabs.get_mut(&id) {
                    if let Some(rect) = tab.view.rect {
                        if position.y < rect.y as f64 {
                            tab.view.scroll(&tab.document, -2);
                        }
                        if position.y > (rect.y + rect.height as i32) as f64 {
                            tab.view.scroll(&tab.document, 2);
                        }
                    }
                    if let Some(cursor) = tab.view.hit(position.x as f32, position.y as f32) {
                        tab.document.select(cursor, true);
                    }
                }
            }
            Some(Drag::TreeResize) => {
                let left = self.tree_rect.map_or(0, |r| r.x) as f32;
                self.tree_width = ((position.x as f32 - left) / self.scale).clamp(130., 420.)
            }
            Some(Drag::OutlineResize) => {
                if let Some(window) = &self.window {
                    self.outline_width = ((window.inner_size().width as f32 - position.x as f32)
                        / self.scale)
                        .clamp(130., 420.);
                }
            }
            Some(Drag::Divider(index)) => {
                if let Some(divider) = self.dividers.get(index).cloned() {
                    let ratio = if divider.down {
                        (position.y - divider.parent.y as f64) / divider.parent.height.max(1) as f64
                    } else {
                        (position.x - divider.parent.x as f64) / divider.parent.width.max(1) as f64
                    };
                    if let Some(target) = self.split.ratio_mut(&divider.path) {
                        *target = ratio.clamp(0.15, 0.85) as f32;
                    }
                }
            }
            Some(Drag::Scrollbar(id)) => {
                if let Some(tab) = self.tabs.get_mut(&id) {
                    if let Some(rect) = tab.view.rect {
                        let fraction =
                            ((position.y - rect.y as f64) / rect.height as f64).clamp(0., 1.);
                        let position = (fraction * tab.document.text.len_chars() as f64) as usize;
                        let line = tab
                            .document
                            .text
                            .char_to_line(position.min(tab.document.text.len_chars()));
                        let start = tab.document.text.line_to_char(line);
                        tab.view.top = if position - start > 512 {
                            position
                        } else {
                            start
                        };
                    }
                }
            }
            _ => {}
        }
        if self.drag.is_some() {
            self.redraw();
            return;
        }
        // Slide between menu-bar menus while one is open.
        let over_menu = self
            .hits
            .iter()
            .find(|(rect, hit)| matches!(hit, Hit::Do(Act::Menu(_))) && contains(*rect, position.x, position.y))
            .map(|(rect, hit)| (*rect, hit.clone()));
        if let (Some((rect, Hit::Do(Act::Menu(index)))), Some(Popup { menu: Some(open), .. })) =
            (over_menu, &self.popup)
        {
            if index != *open {
                self.open_menu(index, rect);
                self.redraw();
            }
        }
        let top = self
            .hits
            .iter()
            .rposition(|(rect, _)| contains(*rect, position.x, position.y));
        if top != self.hover {
            self.hover = top;
            self.hover_since = Some(Instant::now());
            self.update_cursor();
            self.redraw();
        } else if top.is_none() {
            self.update_cursor();
        }
    }
    fn mouse_up(&mut self) {
        if let Some(Drag::Node(id, origin)) = self.drag.take() {
            if (self.mouse.x - origin.x).abs() + (self.mouse.y - origin.y).abs() >= 8. {
                self.finish_node_drag(&id);
            }
            self.redraw();
            return;
        }
        if let Some(Drag::Tab(tab, source, origin)) = self.drag.take() {
            if (self.mouse.x - origin.x).abs() + (self.mouse.y - origin.y).abs() < 8. {
                return;
            }
            if let Some((target, rect)) = self
                .pane_rects
                .iter()
                .find(|(_, rect)| contains(*rect, self.mouse.x, self.mouse.y))
                .copied()
            {
                if let Some((down, before)) = self.drop_zone(rect, self.mouse.x, self.mouse.y) {
                    if source == target
                        && self.split.pane(source).is_some_and(|p| p.tabs.len() == 1)
                    {
                        return;
                    }
                    self.active_pane = target;
                    self.split_pane_at(down, before, Some((tab, source)));
                } else if self.mouse.y < rect.y as f64 + self.s(38.) as f64 {
                    if let Some(pane) = self.split.pane(target) {
                        let index = self.tab_drop_index(pane, rect, self.mouse.x);
                        self.move_tab(tab, source, target, index);
                    }
                }
            }
        }
        self.drag = None;
        self.redraw();
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let saved = read_complete(&*self.config_storage, Path::new("session.toml"))
            .ok()
            .and_then(|text| toml::from_str::<Session>(&text.to_string()).ok())
            .and_then(|session| session.window)
            .filter(|w| w.width >= 300 && w.height >= 200);
        self.saved_window = saved;
        let monitor = event_loop.primary_monitor().or_else(|| event_loop.available_monitors().next());
        let (width, height) = monitor.as_ref().map_or((1280., 800.), |m| {
            let scale = m.scale_factor().max(0.5);
            default_window_size(m.size().width as f64 / scale, m.size().height as f64 / scale)
        });
        let mut attributes = Window::default_attributes()
            .with_visible(!cfg!(test))
            .with_title(APP_NAME)
            .with_theme(None)
            .with_inner_size(LogicalSize::new(width, height))
            .with_min_inner_size(LogicalSize::new(440., 300.));
        if let Some(state) = saved {
            // Only reuse the position if it is still on a connected monitor.
            let on_screen = event_loop.available_monitors().any(|monitor| {
                let (p, s) = (monitor.position(), monitor.size());
                state.x + 80 > p.x
                    && state.y >= p.y
                    && state.x + 80 < p.x + s.width as i32
                    && state.y + 40 < p.y + s.height as i32
            });
            // A remembered size from a bigger screen is trimmed to the one in use.
            let (limit_w, limit_h) = monitor.as_ref().map_or((u32::MAX, u32::MAX), |m| (m.size().width, m.size().height));
            attributes = attributes.with_inner_size(winit::dpi::PhysicalSize::new(state.width.min(limit_w), state.height.min(limit_h)));
            if on_screen {
                attributes = attributes.with_position(winit::dpi::PhysicalPosition::new(state.x, state.y));
            }
            attributes = attributes.with_maximized(state.maximized);
        }
        let window = Arc::new(
            event_loop
                .create_window(attributes)
                .expect("Cannot create editor window"),
        );
        window.set_ime_allowed(true);
        self.system_light = window.theme() == Some(Theme::Light);
        self.settings.system_light = self.system_light;
        let context =
            softbuffer::Context::new(window.clone()).expect("Cannot initialize CPU renderer");
        self.surface = Some(
            softbuffer::Surface::new(&context, window.clone())
                .expect("Cannot create render surface"),
        );
        self.window = Some(window);
        self.redraw();
    }
    fn new_events(&mut self, event_loop: &ActiveEventLoop, cause: StartCause) {
        if matches!(cause, StartCause::ResumeTimeReached { .. }) {
            self.autosave(event_loop);
            self.redraw();
        }
    }
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.autosave(event_loop);
    }
    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Chosen(Some(path), choice) => self.chosen(path, choice),
            UserEvent::Chosen(None, _) => {
                self.after_save = None;
                self.pending_import = None;
                self.pending_export = None;
            }
            UserEvent::Read(id, event) => {
                if let Some(tab) = self.tabs.get_mut(&id) {
                    match event {
                        ReadEvent::Chunk(chunk) => {
                            if tab.document.text.len_chars() == 0 {
                                tab.document.line_ending =
                                    if chunk.chars().take(8192).any(|c| c == '\r') {
                                        "\r\n"
                                    } else {
                                        "\n"
                                    };
                            }
                            tab.document.append_loaded(chunk);
                            if let Some(cursor) = tab.restore_cursor {
                                if cursor <= tab.document.text.len_chars() {
                                    tab.document.select(cursor, false);
                                    tab.restore_cursor = None;
                                }
                            }
                        }
                        ReadEvent::Complete => {
                            tab.loading = false;
                            if let Some(cursor) = tab.restore_cursor.take() {
                                tab.document.select(cursor, false);
                            }
                            self.scan(id, true);
                            self.rebuild_marks(id);
                            self.apply_jump();
                        }
                        ReadEvent::Error(error) => {
                            tab.loading = false;
                            tab.failed = true;
                            self.status = format!("Cannot load {}: {error}", tab.name());
                        }
                    }
                }
            }
            UserEvent::Saved(id, revision, snapshot, result) => {
                if let Some(tab) = self.tabs.get_mut(&id) {
                    tab.saving = false;
                    match result {
                        Ok(()) => {
                            tab.document.mark_saved(revision, snapshot);
                            if tab.document.dirty() {
                                tab.edited = Some(Instant::now());
                            }
                            self.status = "Saved".into();
                        }
                        Err(error) => {
                            self.status = format!("Save failed: {error}");
                            self.after_save = None;
                            tab.edited = None;
                        }
                    }
                    self.scan(id, false);
                    if let Some(after) = self.after_save.clone() {
                        let done = match after {
                            Kind::Close(id) => {
                                self.tabs.get(&id).is_none_or(|t| !t.document.dirty())
                            }
                            _ => self.tabs.values().all(|t| !t.document.dirty()),
                        };
                        if done {
                            self.after_save = None;
                            self.overlay = None;
                            match after {
                                Kind::Close(id) => self.close_tab(id, true),
                                Kind::Quit => self.persist_session(true),
                                Kind::ChangeWorkspace(path) => {
                                    self.shutdown_project();
                                    self.clear_tabs();
                                    self.open_project(path, false);
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
            UserEvent::Indexed(id, epoch, revision, blocks) => {
                if let Some(tab) = self.tabs.get_mut(&id) {
                    if tab.document.revision == revision && tab.disk_epoch == epoch {
                        let old = std::mem::replace(&mut tab.blocks, blocks);
                        std::thread::spawn(move || drop(old));
                    }
                }
            }
            UserEvent::Fonts(db) => {
                self.font_names = db
                    .faces()
                    .flat_map(|face| face.families.iter().map(|(name, _)| name.clone()))
                    .collect();
                self.font_names.sort();
                self.font_names.dedup();
                self.fonts = FontSystem::new_with_locale_and_db("en-US".into(), db);
                // Font IDs belong to a database; cached glyph IDs must not cross this swap.
                self.renderer = CpuRenderer::default();
                self.labels.clear();
                self.widths.clear();
                for tab in self.tabs.values_mut() {
                    tab.view.clear_cache();
                }
            }
            UserEvent::Rows(version, rows) => {
                if let Some(overlay) = &mut self.overlay {
                    if overlay.version == version {
                        overlay.rows = rows;
                        overlay.pending = false;
                        overlay.selected =
                            overlay.selected.min(overlay.rows.len().saturating_sub(1));
                    }
                }
                self.preview_match();
            }
            UserEvent::Config(result) => match result {
                Ok((settings, session)) => {
                    self.adopt(settings);
                    self.labels.clear();
                    self.apply_window_theme();
                    let argument = std::env::args_os()
                        .skip(1)
                        .find(|s| !s.to_string_lossy().starts_with("--"));
                    if let Some(session) = &session {
                        self.file_type_registered = session.file_type;
                        self.tree_width = if (130.0..=420.0).contains(&session.tree_width) { session.tree_width } else { 232. };
                        self.outline_width = if (130.0..=420.0).contains(&session.outline_width) { session.outline_width } else { 260. };
                        self.recent = session.recent.clone();
                        self.show_tree = session.show_tree;
                        self.show_outline = session.show_outline;
                    }
                    if cfg!(windows) && !self.file_type_registered {
                        // Once per user: associate .jw files with this program.
                        self.file_type_registered = true;
                        std::thread::spawn(|| {
                            let _ = platform::register_file_type();
                        });
                    }
                    if self.project.is_none() {
                        if let Some(path) = argument {
                            let path = PathBuf::from(path);
                            if path.extension().is_some_and(|e| ["docx", "md", "markdown", "txt"].iter().any(|x| e.eq_ignore_ascii_case(x))) {
                                // A manuscript on the command line is offered for import.
                                self.chosen(path, Choose::ImportFile);
                            } else if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("jw")) && !path.exists() {
                                // `justwrite novel.jw` starts the project when it is not there yet.
                                let title = path.file_stem().map_or_else(|| "My novel".to_owned(), |s| s.to_string_lossy().into_owned());
                                self.create_project(path, title);
                            } else {
                                self.open_project(path, false);
                            }
                        } else if let Some(path) = session.as_ref().and_then(|s| s.project.clone()).filter(|p| p.is_file()) {
                            self.pending_session = session.and_then(|s| s.split);
                            self.open_project(path, false);
                        }
                    }
                }
                Err(error) => self.status = format!("Settings error: {error}"),
            },
            UserEvent::ProjectOpened(path, result) => self.project_opened(path, result),
            UserEvent::ChapterSaved(rel, words) => self.chapter_saved(&rel, words),
            UserEvent::Packed(result) => self.packed(result),
            UserEvent::Exported(result) => self.exported(result),
            UserEvent::Preview(version, result) => self.preview_ready(version, result),
            UserEvent::Imported(result) => self.imported(result),
            UserEvent::Spelling(tab, signature, ranges) => self.spelling_ready(tab, signature, ranges),
            UserEvent::SpellerLoaded(result) => self.speller_loaded(result),
            UserEvent::ImportRead(path, result) => self.import_read(path, result),
            UserEvent::PreviewPage(version, page, q, bitmap) => self.preview_page(version, page, q, bitmap),
            UserEvent::SessionSaved(result) => match result {
                Ok(()) if self.session_closing => event_loop.exit(),
                Ok(()) => {}
                Err(error) => {
                    self.session_closing = false;
                    self.status = format!("Cannot save session: {error}");
                }
            },
            UserEvent::Error(error) => self.status = error,
            UserEvent::Replaced(id, epoch, revision, text) => {
                if let Some(tab) = self.tabs.get_mut(&id) {
                    if tab.document.revision == revision && tab.disk_epoch == epoch {
                        let end = tab.document.text.len_chars();
                        tab.document.replace_rope(0..end, text);
                        self.edited(id);
                        self.refresh_overlay();
                    } else {
                        self.status =
                            "The document changed during replace-all. Please run it again.".into();
                    }
                }
            }
            UserEvent::ConfigWatch(watcher) => self.config_watcher = Some(watcher),
            UserEvent::ConfigChanged => {
                let storage = self.config_storage.clone();
                let proxy = self.proxy.clone();
                std::thread::spawn(move || {
                    let result = read_complete(&*storage, Path::new("settings.toml"))
                        .map_err(|e| e.to_string())
                        .and_then(|text| Settings::parse(&text.to_string()));
                    let _ = proxy.send_event(UserEvent::SettingsUpdated(result));
                });
            }
            UserEvent::SettingsUpdated(result) => match result {
                Ok(settings) => {
                    self.adopt(settings);
                    self.labels.clear();
                    for tab in self.tabs.values_mut() {
                        tab.view.clear_cache();
                    }
                    self.apply_window_theme();
                }
                Err(error) => self.status = format!("Settings error: {error}"),
            },
        }
        self.title();
        self.redraw();
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => self.command(Command::Quit),
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                self.dirty_session();
                self.redraw()
            }
            WindowEvent::Moved(_) => self.dirty_session(),
            WindowEvent::ThemeChanged(theme) => {
                self.system_light = theme == Theme::Light;
                self.settings.system_light = self.system_light;
                self.labels.clear();
                for tab in self.tabs.values_mut() {
                    tab.view.clear_cache();
                }
                self.redraw();
            }
            WindowEvent::RedrawRequested => self.frame(event_loop),
            WindowEvent::ModifiersChanged(modifiers) => {
                let alt = self.modifiers.alt_key();
                self.modifiers = modifiers.state();
                if alt != self.modifiers.alt_key() {
                    self.redraw();
                }
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                let shortcut = match &event.logical_key {
                    Key::Character(c) => c.to_lowercase(),
                    Key::Named(key) => format!("{key:?}").to_lowercase(),
                    _ => String::new(),
                };
                let shortcut = match event.physical_key {
                    winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::Digit8) => {
                        "8".into()
                    }
                    winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::Digit7) => {
                        "7".into()
                    }
                    winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::Digit5) => {
                        "5".into()
                    }
                    winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::Backslash) => {
                        "\\".into()
                    }
                    _ => shortcut,
                };
                self.key(event.logical_key, shortcut, event.text.as_deref());
            }
            WindowEvent::Ime(Ime::Preedit(text, _)) => {
                self.preedit = text;
                self.redraw();
            }
            WindowEvent::Ime(Ime::Commit(text)) => {
                self.preedit.clear();
                if self.form.is_some() {
                    self.form_text(&text);
                } else if let Some(overlay) = self.overlay.as_mut().filter(|o| o.focused) {
                    overlay.type_text(&text);
                    self.refresh_overlay();
                } else if self.screen.is_some() || self.project.is_none() {
                } else if let Some(id) = self.active_id() {
                    self.tabs.get_mut(&id).unwrap().document.insert(&text);
                    self.edited(id);
                }
                self.redraw();
            }
            WindowEvent::Ime(Ime::Disabled) => {
                self.preedit.clear();
                self.redraw();
            }
            WindowEvent::CursorMoved { position, .. } => self.mouse_move(position),
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button,
                ..
            } if matches!(
                button,
                MouseButton::Left | MouseButton::Middle | MouseButton::Right
            ) =>
            {
                self.mouse_down(button)
            }
            WindowEvent::MouseInput {
                state: ElementState::Released,
                ..
            } => self.mouse_up(),
            WindowEvent::MouseWheel { delta, .. } => {
                self.wheel += match delta {
                    MouseScrollDelta::LineDelta(_, y) => -y * 3.,
                    MouseScrollDelta::PixelDelta(position) => {
                        -position.y as f32 / (self.settings.font_size * self.scale)
                    }
                };
                let rows = self.wheel.trunc() as i32;
                self.wheel -= rows as f32;
                if rows == 0 {
                    return;
                }
                let (x, y) = (self.mouse.x, self.mouse.y);
                let tab_strip = self.hit_at(x, y).and_then(|(_, hit)| match hit {
                    Hit::Tab(pane, _) => Some(pane),
                    Hit::Close(id) => self
                        .split
                        .pane(self.active_pane)
                        .filter(|p| p.tabs.contains(&id))
                        .map(|p| p.id),
                    _ => None,
                });
                if self.form.is_some() {
                    self.form_scroll(rows);
                } else if let Some(o) = self.overlay.as_mut().filter(|o| !o.docked()) {
                    let last = o.rows.len().saturating_sub(o.visible);
                    o.scroll = (o.scroll as i32 + rows).clamp(0, last as i32) as usize;
                } else if self.screen.is_some() {
                    self.screen_scroll(rows, false);
                } else if let Some(pane) = tab_strip {
                    self.active_pane = pane;
                    self.command(if rows > 0 { Command::NextTab } else { Command::PreviousTab });
                } else if self.tree_rect.is_some_and(|r| contains(r, x, y)) {
                    self.tree_top = (self.tree_top as i32 + rows).max(0) as usize;
                } else if self.outline_rect.is_some_and(|r| contains(r, x, y)) {
                    self.scroll_panel(rows);
                } else if let Some((pane, _)) = self
                    .pane_rects
                    .iter()
                    .find(|(_, rect)| contains(*rect, x, y))
                {
                    if let Some(id) = self.split.pane(*pane).and_then(Pane::tab) {
                        if let Some(tab) = self.tabs.get_mut(&id) {
                            tab.view.scroll(&tab.document, rows);
                        }
                    }
                }
                self.redraw();
            }
            WindowEvent::DroppedFile(path) => self.dropped(path),
            WindowEvent::Focused(false) => {
                self.drag = None;
                self.modifiers = ModifiersState::empty();
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod window_size_tests {
    use super::default_window_size;

    #[test]
    fn big_screens_get_the_usual_size_and_small_ones_leave_room() {
        assert_eq!(default_window_size(2560., 1440.), (1280., 800.));
        let (w, h) = default_window_size(1366., 768.);
        assert!(w < 1366. && h < 768. - 60., "{w} x {h} leaves no room for a taskbar");
        assert_eq!(default_window_size(300., 200.), (640., 420.), "never absurdly small");
    }
}
