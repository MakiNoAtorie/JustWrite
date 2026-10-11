//! Cloud saving in the window: the commands, and the workers that talk to the service. All
//! network work runs on a worker thread and reports back through `UserEvent::Cloud`.
use super::*;
use crate::cloud::{self, Ids, Provider, Synced};
use forms::{Field, FormKind};

pub enum CloudDone {
    /// Connected or disconnected: a line for the status bar.
    Note(Result<String, String>),
    Listed(Result<Vec<String>, String>),
    Fetched(Result<PathBuf, String>),
    Synced(PathBuf, Result<Synced, String>),
}

/// The status of a linked project that is the same here and in the cloud.
pub const SYNCED: &str = "Cloud: synced";
/// Seconds between looks at the cloud file while a linked project is open.
pub const PULL: Duration = Duration::from_secs(60);
/// A linked project is packed (and so uploaded) this long after the last chapter save.
// Tune from real use: each upload sends the whole `.jw`.
pub const QUIET: Duration = Duration::from_secs(20);

impl App {
    fn cloud_ids(&self) -> Ids {
        Ids {
            onedrive: self.settings.onedrive_client_id.trim().to_owned(),
            google: self.settings.google_client_id.trim().to_owned(),
            google_secret: self.settings.google_client_secret.trim().to_owned(),
        }
    }
    /// Runs cloud work on a worker thread, one at a time. Returns whether it started.
    fn cloud_run(&mut self, work: impl FnOnce(&Path, &Ids) -> CloudDone + Send + 'static) -> bool {
        if self.cloud_busy {
            return false;
        }
        self.cloud_busy = true;
        let (proxy, data, ids) = (self.proxy.clone(), platform::data_directory(), self.cloud_ids());
        std::thread::spawn(move || {
            let _ = proxy.send_event(UserEvent::Cloud(work(&data, &ids)));
        });
        true
    }
    /// A command the author gave: says so when the earlier cloud work is still under way.
    fn cloud_command(&mut self, note: &str, work: impl FnOnce(&Path, &Ids) -> CloudDone + Send + 'static) {
        self.cloud_note.clear();
        self.status = if self.cloud_run(work) { note } else { "Cloud: still busy; try again in a moment." }.into();
        self.redraw();
    }

    pub(super) fn cloud_connect_form(&mut self) {
        let fields = vec![
            Field::choice("service", "Service", Provider::ALL.iter().map(|p| p.name().to_owned()).collect(), 0),
            Field::text("server", "Server address", "").hint("Such as https://cloud.example.org").when("service", &[0]),
            Field::info("Your browser opens for the sign-in. JustWrite keeps a JustWrite folder in the service and stores the sign-in in this computer's secret store. Connecting replaces an earlier connection."),
        ];
        self.open_form(Form::new(FormKind::Cloud, "Connect to cloud", fields).submit("Connect").width(560.));
    }
    pub(super) fn cloud_connect(&mut self, service: usize, server: String) {
        let provider = Provider::ALL[service.min(2)];
        self.cloud_command("Cloud: finish the sign-in in your browser within 3 minutes…", move |data, ids| CloudDone::Note(cloud::connect(data, provider, &server, ids)));
    }
    pub(super) fn cloud_disconnect(&mut self) {
        self.cloud_command("Cloud: disconnecting…", |data, _| CloudDone::Note(Ok(cloud::disconnect(data))));
    }
    /// Links the open project to the cloud folder and uploads it.
    pub(super) fn cloud_save(&mut self) {
        let Some(state) = &mut self.project else { return };
        let jw = state.core.path.clone();
        // Two computers are compared from the newest snapshot both have: make sure there is one.
        let _ = state.history.capture_auto(&state.core, "Saved to cloud", jw::time::now());
        self.save_all();
        self.cloud_command("Cloud: saving…", move |data, ids| match cloud::link(data, &jw) {
            Ok(()) => {
                let result = cloud::sync(data, ids, &jw);
                CloudDone::Synced(jw, result)
            }
            Err(error) => CloudDone::Note(Err(error)),
        });
    }
    pub(super) fn cloud_open(&mut self) {
        self.cloud_command("Cloud: reading the folder…", |data, ids| CloudDone::Listed(cloud::list(data, ids)));
    }
    /// A project was picked from the list; the dialog result comes to `cloud_fetch`.
    pub(super) fn cloud_pick(&mut self, name: String) {
        self.cloud_picked = Some(name);
        self.choose(Choose::CloudSave);
    }
    pub(super) fn cloud_fetch(&mut self, dest: PathBuf) {
        let Some(name) = self.cloud_picked.take() else { return };
        if self.project.as_ref().is_some_and(|p| p.core.path == dest) {
            self.status = "That file is the open project. Choose another name.".into();
            return;
        }
        self.cloud_command("Cloud: downloading…", move |data, ids| CloudDone::Fetched(cloud::fetch(data, ids, &name, &dest).map(|()| dest)));
    }

    /// Compares the open project with its cloud file. Does nothing for a project that is not linked.
    pub(super) fn cloud_sync(&mut self) {
        let Some(state) = &self.project else { return };
        // A download reloads the project, so not under a dialog. Only a fully saved project is
        // compared; a pack that is still due runs first and calls this.
        if self.modal() || self.screen.is_some() || state.packing || state.blocked || self.tabs.values().any(|t| t.document.dirty()) || state.core.is_dirty() {
            return;
        }
        let jw = state.core.path.clone();
        self.cloud_pull = Instant::now();
        // Busy: look again when the work under way is done, so a fresh pack is not left waiting.
        self.cloud_again = !self.cloud_run(move |data, ids| {
            let result = cloud::sync(data, ids, &jw);
            CloudDone::Synced(jw, result)
        });
    }
    /// Uploads the last pack of a project that is closing. Waits a few seconds at most; what does
    /// not go now goes when the project is opened again.
    pub(super) fn cloud_flush(&self, jw: PathBuf) {
        let (data, ids) = (platform::data_directory(), self.cloud_ids());
        let (done, wait) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = done.send(cloud::sync(&data, &ids, &jw));
        });
        let _ = wait.recv_timeout(Duration::from_secs(5));
    }

    /// Shows a cloud state in the status bar once, not again on every look at the cloud.
    fn cloud_say(&mut self, note: String) {
        if self.cloud_note != note {
            // A failure that must not be missed stays in the status bar.
            if !self.failing() {
                self.status = note.clone();
            }
            self.cloud_note = note;
        }
    }
    pub(super) fn cloud_done(&mut self, done: CloudDone) {
        self.cloud_busy = false;
        self.cloud_report(done);
        if std::mem::take(&mut self.cloud_again) {
            self.cloud_sync();
        }
        self.redraw();
    }
    fn cloud_report(&mut self, done: CloudDone) {
        match done {
            CloudDone::Note(result) => {
                // Connecting and disconnecting drop every link.
                if let (Ok(_), Some(state)) = (&result, &mut self.project) {
                    state.cloud_linked = false;
                }
                self.status = result.unwrap_or_else(|error| error);
            }
            CloudDone::Listed(Ok(files)) if files.is_empty() => self.status = "Cloud: the folder has no projects yet.".into(),
            CloudDone::Listed(Ok(files)) => {
                self.status.clear();
                self.cloud_files = files;
                self.show(Kind::Cloud);
            }
            CloudDone::Fetched(Ok(path)) => self.open_project_checked(path),
            CloudDone::Listed(Err(error)) | CloudDone::Fetched(Err(error)) => self.status = error,
            CloudDone::Synced(jw, result) => {
                let Some(state) = self.project.as_mut().filter(|p| p.core.path == jw) else { return };
                state.cloud_linked = !matches!(result, Ok(Synced::NotLinked));
                match result {
                    Ok(Synced::NotLinked) => self.cloud_note.clear(),
                    Ok(Synced::Nothing | Synced::Uploaded) => self.cloud_say(SYNCED.into()),
                    Ok(Synced::Downloaded) => self.cloud_downloaded(),
                    Ok(Synced::Incoming(file, tag)) => self.cloud_merge(file, tag),
                    // Two files now need the author; the message stays until it is replaced.
                    Ok(Synced::Conflict(name)) => {
                        self.cloud_note.clear();
                        self.fail(format!("Cloud: another project has this name in the cloud. This one is now “{name}” there."));
                    }
                    Err(error) => self.cloud_say(error),
                }
            }
        }
    }
    /// Text was typed, or is being saved, since the look at the cloud began.
    fn cloud_pending(&mut self) -> bool {
        self.flush_tabs();
        self.tabs.values().any(|t| t.document.dirty()) || self.project.as_ref().is_none_or(|p| p.packing || p.core.is_dirty())
    }
    /// The project changed here and on another computer. Its version is merged in through the
    /// history: it becomes a line, and what only one side changed is taken from that side.
    fn cloud_merge(&mut self, file: PathBuf, tag: String) {
        // Under a dialog, or typing went on: the next look at the cloud comes back here.
        if self.modal() || self.screen.is_some() || self.cloud_pending() {
            let _ = std::fs::remove_file(&file);
            return;
        }
        let Some(state) = &mut self.project else { return };
        let jw = state.core.path.clone();
        let result = crate::history::merge::merge_file(&mut state.history, &mut state.core, &file, jw::time::now())
            .and_then(|report| cloud::accept(&platform::data_directory(), &jw, tag).map(|()| report));
        let _ = std::fs::remove_file(&file);
        self.cloud_note.clear();
        match result {
            // The cloud file is a copy of this project: look again, and this one goes up over it.
            Ok(report) if report.line.is_empty() => self.cloud_again = true,
            Ok(report) => {
                self.after_restore();
                self.pack_project(false);
                let line = format!("Its version is on the line “{}” in the history.", report.line);
                // The text changed under the author: the message stays until it is replaced.
                if report.conflicts.is_empty() && !report.lists_differ {
                    self.cloud_note = SYNCED.into();
                    self.fail(format!("Cloud: merged the other computer's changes ({} taken, {} put together). {line}", report.taken, report.combined));
                } else {
                    let mut note = String::from("Cloud: merged the other computer's changes.");
                    if !report.conflicts.is_empty() {
                        note.push_str(&format!(" Changed in the same place on both, your text kept: {}.", report.conflicts.join(", ")));
                    }
                    if report.lists_differ {
                        note.push_str(" The chapter list changed on both; yours was kept.");
                    }
                    self.fail(format!("{note} {line}"));
                }
            }
            Err(error) => self.fail(format!("Cloud: the other computer's version could not be merged ({error})")),
        }
    }
    /// The `.jw` of the open project was replaced by a newer cloud version.
    fn cloud_downloaded(&mut self) {
        if self.cloud_pending() {
            // Text was typed during the download; the next save shows the "changed outside" form.
            self.cloud_note.clear();
            self.fail("Cloud: a newer version came down while you were typing. Saving will ask what to keep.".into());
            return;
        }
        // Nothing here is newer: show the downloaded version, in the same tabs.
        let Some(state) = self.project.take() else { return };
        let path = state.core.path.clone();
        self.pending_session = Some(self.session_split(&self.split));
        self.root = None;
        self.storage = None;
        // A dialog opened during the download belongs to the version that is going away.
        (self.form, self.overlay, self.popup, self.screen) = (None, None, None, None);
        state.core.abandon();
        self.clear_tabs();
        self.cloud_note = SYNCED.into();
        self.open_project(path, false);
    }
}
