use std::{future::Future, path::PathBuf, pin::Pin, sync::Arc, task::{Context, Poll, Wake, Waker}};
use eframe::egui;
use usd_bevy::editor::{EditorCommand, EditorSnapshot, SaveMode};

#[derive(Clone)]
pub enum Request {
    Open,
    Save { mode: SaveMode, document_id: u64, edit_layer: String },
}

impl Request {
    pub fn save(mode: SaveMode, snapshot: &EditorSnapshot) -> Self {
        Self::Save { mode, document_id: snapshot.document_id, edit_layer: snapshot.edit_layer.clone() }
    }

    fn command(self, path: PathBuf) -> Result<EditorCommand, String> {
        let filename = path.into_os_string().into_string().map_err(|_| "USD paths must be UTF-8".to_owned())?;
        Ok(match self {
            Self::Open => EditorCommand::Open(filename),
            Self::Save { mode, document_id, edit_layer } => EditorCommand::SaveChecked { filename, mode, document_id, edit_layer },
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
            Request::Open => {
                let selection = dialog.add_filter("USD", &["usd", "usda", "usdc", "usdz"]).pick_file();
                Box::pin(async move { selection.await.map(|file| file.path().to_owned()) })
            }
            Request::Save { .. } => {
                let selection = dialog.add_filter("USD ASCII", &["usda"]).save_file();
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
    fn pending_dialog_does_not_block_or_allow_duplicate_requests() {
        let mut dialogs = FileDialogs::new(&egui::Context::default());
        dialogs.begin(Request::Open, Box::pin(std::future::pending()));
        dialogs.begin(Request::Open, Box::pin(async { Some(PathBuf::from("wrong.usda")) }));
        for _ in 0..3 { assert!(dialogs.poll().is_none()); }
        assert_eq!(dialogs.status(), Some("Waiting for file selection"));
    }

    #[test]
    fn cancellation_and_completion_are_consumed_once() {
        let mut dialogs = FileDialogs::new(&egui::Context::default());
        dialogs.begin(Request::Open, Box::pin(async { None }));
        assert!(dialogs.poll().is_none());
        assert!(dialogs.status().is_none());
        dialogs.begin(Request::Open, Box::pin(async { Some(PathBuf::from("path with spaces.usda")) }));
        assert!(matches!(dialogs.poll(), Some(EditorCommand::Open(path)) if path == "path with spaces.usda"));
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
        dialogs.begin(Request::Open, Box::pin(future));
        assert!(dialogs.poll().is_none());
        assert_eq!(counter.0.load(Ordering::SeqCst), 1);
        ready.store(true, Ordering::SeqCst);
        registered.lock().unwrap().take().unwrap().wake();
        assert_eq!(counter.0.load(Ordering::SeqCst), 2);
        assert!(matches!(dialogs.poll(), Some(EditorCommand::Open(path)) if path == "delayed.usda"));
        assert!(dialogs.poll().is_none());
    }

    #[test]
    fn save_selection_retains_document_and_edit_layer_context() {
        let snapshot = EditorSnapshot { document_id: 42, edit_layer: "weak.usda".into(), ..Default::default() };
        let command = Request::save(SaveMode::EditLayer, &snapshot).command("output.usda".into()).unwrap();
        assert!(matches!(command, EditorCommand::SaveChecked { document_id: 42, edit_layer, mode: SaveMode::EditLayer, .. } if edit_layer == "weak.usda"));
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_selection_reports_error_instead_of_changing_the_path() {
        use std::os::unix::ffi::OsStringExt;
        let mut dialogs = FileDialogs::new(&egui::Context::default());
        dialogs.begin(Request::Open, Box::pin(async { Some(std::ffi::OsString::from_vec(vec![0xff]).into()) }));
        assert!(dialogs.poll().is_none());
        assert_eq!(dialogs.status(), Some("USD paths must be UTF-8"));
    }
}
