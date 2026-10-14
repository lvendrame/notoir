//! Native file dialogs without blocking the UI thread (spec 31): each dialog
//! runs as an async rfd dialog on a helper thread (rfd dispatches to the main
//! run loop where the OS requires it) and reports through a callback.

use std::path::PathBuf;

#[derive(Clone, Debug, Default)]
pub struct FileDialogOptions {
    pub title: Option<String>,
    pub directory: Option<PathBuf>,
    pub file_name: Option<String>,
}

fn dialog(opts: &FileDialogOptions) -> rfd::AsyncFileDialog {
    let mut d = rfd::AsyncFileDialog::new();
    if let Some(t) = &opts.title {
        d = d.set_title(t);
    }
    if let Some(dir) = &opts.directory {
        d = d.set_directory(dir);
    }
    if let Some(name) = &opts.file_name {
        d = d.set_file_name(name);
    }
    d
}

fn spawn(name: &str, f: impl FnOnce() + Send + 'static) {
    if let Err(e) = std::thread::Builder::new().name(name.into()).spawn(f) {
        tracing::error!(error = %e, "could not start dialog thread");
    }
}

/// Multi-select open dialog; `done` gets the chosen files (empty = cancelled).
pub fn pick_files(opts: FileDialogOptions, done: impl FnOnce(Vec<PathBuf>) + Send + 'static) {
    spawn("notoir-dialog", move || {
        let files = pollster::block_on(dialog(&opts).pick_files()).unwrap_or_default();
        done(files.into_iter().map(|f| f.path().to_path_buf()).collect());
    });
}

/// Folder picker (Open Workspace…).
pub fn pick_folder(opts: FileDialogOptions, done: impl FnOnce(Option<PathBuf>) + Send + 'static) {
    spawn("notoir-dialog", move || {
        let folder = pollster::block_on(dialog(&opts).pick_folder());
        done(folder.map(|f| f.path().to_path_buf()));
    });
}

/// Save dialog; the OS asks before replacing an existing file.
pub fn save_file(opts: FileDialogOptions, done: impl FnOnce(Option<PathBuf>) + Send + 'static) {
    spawn("notoir-dialog", move || {
        let file = pollster::block_on(dialog(&opts).save_file());
        done(file.map(|f| f.path().to_path_buf()));
    });
}
