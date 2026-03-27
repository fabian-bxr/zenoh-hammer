use flume::{Receiver, unbounded};
use std::path::PathBuf;

pub struct NativeFileDialog {
    receiver: Receiver<Option<PathBuf>>,
}

impl NativeFileDialog {
    pub fn open(initial_path: Option<PathBuf>) -> Self {
        let (tx, rx) = unbounded();
        std::thread::spawn(move || {
            let mut dialog = rfd::FileDialog::new();
            if let Some(dir) = initial_path.as_ref().and_then(|p| p.parent()) {
                dialog = dialog.set_directory(dir);
            }
            let result = dialog.pick_file();
            let _ = tx.send(result);
        });
        NativeFileDialog { receiver: rx }
    }

    pub fn save(initial_path: Option<PathBuf>) -> Self {
        let (tx, rx) = unbounded();
        std::thread::spawn(move || {
            let mut dialog = rfd::FileDialog::new();
            if let Some(p) = &initial_path {
                if let Some(dir) = p.parent() {
                    dialog = dialog.set_directory(dir);
                }
                if let Some(name) = p.file_name() {
                    dialog = dialog.set_file_name(name.to_string_lossy().to_string());
                }
            }
            let result = dialog.save_file();
            let _ = tx.send(result);
        });
        NativeFileDialog { receiver: rx }
    }

    /// Returns Some(Some(path)) if user picked, Some(None) if cancelled, None if still pending
    pub fn try_recv(&self) -> Option<Option<PathBuf>> {
        self.receiver.try_recv().ok()
    }
}
