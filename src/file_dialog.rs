use std::{future::Future, path::{Path, PathBuf}, pin::Pin, sync::Arc, task::{Context, Poll, Wake, Waker}};
use eframe::egui;
use usd_bevy::editor::{EditorCommand, EditorSnapshot, SaveMode};

const USD_EXTENSIONS: &[&str] = &["usd", "usda", "usdc", "usdz"];

fn save_labels(mode: SaveMode) -> (&'static str, &'static str) {
    match mode {
        SaveMode::RootLayer => ("Save root layer — extension selects USD format", "scene.usda"),
        SaveMode::EditLayer => ("Save edit layer — extension selects USD format", "edit-layer.usda"),
        SaveMode::Flattened => ("Save flattened scene — extension selects USD format", "flattened.usda"),
    }
}

#[derive(Clone)]
pub enum Request {
    Open { document_id: u64, revision: u64 },
    Save { mode: SaveMode, document_id: u64, edit_layer: String },
}

impl Request {
    pub fn open(snapshot: &EditorSnapshot) -> Self {
        Self::Open { document_id: snapshot.document_id, revision: snapshot.revision }
    }

    pub fn save(mode: SaveMode, snapshot: &EditorSnapshot) -> Self {
        Self::Save { mode, document_id: snapshot.document_id, edit_layer: snapshot.edit_layer.clone() }
    }

    fn command(self, path: PathBuf) -> Result<EditorCommand, String> {
        let filename = path.into_os_string().into_string().map_err(|_| "USD paths must be UTF-8".to_owned())?;
        Ok(match self {
            Self::Open { document_id, revision } => EditorCommand::OpenChecked { filename, document_id, revision },
            Self::Save { mode, document_id, edit_layer } => {
                let extension = Path::new(&filename).extension().and_then(|value| value.to_str()).unwrap_or("");
                if !USD_EXTENSIONS.iter().any(|supported| extension.eq_ignore_ascii_case(supported)) {
                    return Err("Save filename must end in .usda, .usdc, .usd or .usdz".into());
                }
                EditorCommand::SaveChecked { filename, mode, document_id, edit_layer }
            }
        })
    }
}

type Selection = Pin<Box<dyn Future<Output = Option<PathBuf>>>>;

struct Repaint(egui::Context);
impl Wake for Repaint {
    fn wake(self: Arc<Self>) { self.0.request_repaint(); }
    fn wake_by_ref(self: &Arc<Self>) { self.0.request_repaint(); }
}

pub struct FileDialogs {
    pending: Option<(Request, Selection)>,
    waker: Waker,
    status: Option<String>,
}

impl FileDialogs {
    pub fn new(ctx: &egui::Context) -> Self {
        Self { pending: None, waker: Waker::from(Arc::new(Repaint(ctx.clone()))), status: None }
    }

    pub fn start(&mut self, request: Request) {
        if self.pending.is_some() { return; }
        let dialog = rfd::AsyncFileDialog::new();
        let future: Selection = match &request {
            Request::Open { .. } => {
                let selection = dialog.add_filter("USD", USD_EXTENSIONS).pick_file();
                Box::pin(async move { selection.await.map(|file| file.path().to_owned()) })
            }
            Request::Save { mode, .. } => {
                let (title, filename) = save_labels(*mode);
                let selection = dialog.set_title(title).set_file_name(filename)
                    .add_filter("USD ASCII / binary / package (.usda, .usdc, .usd, .usdz)", USD_EXTENSIONS).save_file();
                Box::pin(async move { selection.await.map(|file| file.path().to_owned()) })
            }
        };
        self.begin(request, future);
    }

    fn begin(&mut self, request: Request, future: Selection) {
        if self.pending.is_some() { return; }
        self.pending = Some((request, future));
        self.status = Some("Waiting for file selection".into());
        self.waker.wake_by_ref();
    }

    pub fn poll(&mut self) -> Option<EditorCommand> {
        let (_, future) = self.pending.as_mut()?;
        let Poll::Ready(path) = future.as_mut().poll(&mut Context::from_waker(&self.waker)) else { return None };
        let (request, _) = self.pending.take().unwrap();
        self.status = None;
        match path.map(|path| request.command(path)) {
            Some(Ok(command)) => Some(command),
            Some(Err(error)) => { self.status = Some(error); None }
            None => None,
        }
    }

    pub fn status(&self) -> Option<&str> { self.status.as_deref() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_selection_retains_document_revision_context() {
        let snapshot = EditorSnapshot { document_id: 42, revision: 7, ..Default::default() };
        let command = Request::open(&snapshot).command("selected.usda".into()).unwrap();
        assert!(matches!(command, EditorCommand::OpenChecked { document_id: 42, revision: 7, filename } if filename == "selected.usda"));
    }

    #[test]
    fn pending_dialog_does_not_block_or_allow_duplicate_requests() {
        let mut dialogs = FileDialogs::new(&egui::Context::default());
        dialogs.begin(Request::open(&EditorSnapshot::default()), Box::pin(std::future::pending()));
        dialogs.begin(Request::open(&EditorSnapshot::default()), Box::pin(async { Some(PathBuf::from("wrong.usda")) }));
        for _ in 0..3 { assert!(dialogs.poll().is_none()); }
        assert_eq!(dialogs.status(), Some("Waiting for file selection"));
    }

    #[test]
    fn cancellation_and_completion_are_consumed_once() {
        let mut dialogs = FileDialogs::new(&egui::Context::default());
        dialogs.begin(Request::open(&EditorSnapshot::default()), Box::pin(async { None }));
        assert!(dialogs.poll().is_none());
        assert!(dialogs.status().is_none());
        dialogs.begin(Request::open(&EditorSnapshot::default()), Box::pin(async { Some(PathBuf::from("path with spaces.usda")) }));
        assert!(matches!(dialogs.poll(), Some(EditorCommand::OpenChecked { filename, document_id: 0, revision: 0 }) if filename == "path with spaces.usda"));
        assert!(dialogs.poll().is_none());
        assert!(dialogs.status().is_none());
    }

    #[test]
    fn delayed_completion_registers_and_wakes_before_delivering_selection() {
        use std::sync::{Mutex, atomic::{AtomicBool, AtomicUsize, Ordering}};
        struct Counter(AtomicUsize);
        impl Wake for Counter {
            fn wake(self: Arc<Self>) { self.0.fetch_add(1, Ordering::SeqCst); }
        }
        let ready = Arc::new(AtomicBool::new(false));
        let registered = Arc::new(Mutex::new(None::<Waker>));
        let future = {
            let ready = ready.clone();
            let registered = registered.clone();
            std::future::poll_fn(move |context| {
                if ready.load(Ordering::SeqCst) { Poll::Ready(Some(PathBuf::from("delayed.usda"))) }
                else { *registered.lock().unwrap() = Some(context.waker().clone()); Poll::Pending }
            })
        };
        let mut dialogs = FileDialogs::new(&egui::Context::default());
        let counter = Arc::new(Counter(AtomicUsize::new(0)));
        dialogs.waker = Waker::from(counter.clone());
        dialogs.begin(Request::open(&EditorSnapshot::default()), Box::pin(future));
        assert!(dialogs.poll().is_none());
        assert_eq!(counter.0.load(Ordering::SeqCst), 1);
        ready.store(true, Ordering::SeqCst);
        registered.lock().unwrap().take().unwrap().wake();
        assert_eq!(counter.0.load(Ordering::SeqCst), 2);
        assert!(matches!(dialogs.poll(), Some(EditorCommand::OpenChecked { filename, document_id: 0, revision: 0 }) if filename == "delayed.usda"));
        assert!(dialogs.poll().is_none());
    }

    #[test]
    fn save_selection_retains_document_and_edit_layer_context() {
        let snapshot = EditorSnapshot { document_id: 42, edit_layer: "weak.usda".into(), ..Default::default() };
        let command = Request::save(SaveMode::EditLayer, &snapshot).command("output.usda".into()).unwrap();
        assert!(matches!(command, EditorCommand::SaveChecked { document_id: 42, edit_layer, mode: SaveMode::EditLayer, .. } if edit_layer == "weak.usda"));
    }

    #[test]
    fn save_formats_are_explicit_and_keep_the_selected_path() {
        let snapshot = EditorSnapshot { document_id: 42, edit_layer: "weak.usda".into(), ..Default::default() };
        for extension in ["usda", "usdc", "usd", "usdz", "USDZ"] {
            let path = format!("path with spaces/scene.{extension}");
            let command = Request::save(SaveMode::RootLayer, &snapshot).command(path.clone().into()).unwrap();
            assert!(matches!(command, EditorCommand::SaveChecked { filename, document_id: 42, .. } if filename == path));
        }
        for path in ["scene", "scene.", "scene.png"] {
            assert!(Request::save(SaveMode::RootLayer, &snapshot).command(path.into()).is_err());
        }
        for mode in [SaveMode::RootLayer, SaveMode::EditLayer, SaveMode::Flattened] {
            let (title, filename) = save_labels(mode);
            assert!(title.contains("extension selects"));
            assert!(filename.ends_with(".usda"));
        }
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_selection_reports_error_instead_of_changing_the_path() {
        use std::os::unix::ffi::OsStringExt;
        let mut dialogs = FileDialogs::new(&egui::Context::default());
        dialogs.begin(Request::open(&EditorSnapshot::default()), Box::pin(async { Some(std::ffi::OsString::from_vec(vec![0xff]).into()) }));
        assert!(dialogs.poll().is_none());
        assert_eq!(dialogs.status(), Some("USD paths must be UTF-8"));
    }
}
