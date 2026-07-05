//! Support for linking from an in-memory "shadow" filesystem instead of the real one.
//!
//! When a [`Vfs`] is attached to the linker arguments (see `Args::set_vfs`), it is used
//! exclusively: all input file reads and existence checks are answered from the in-memory map and
//! the real filesystem is never consulted for inputs.
//! TODO:

use crate::error;
use crate::error::Result;
use hashbrown::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;

pub(crate) struct Vfs {
    /// Input files, keyed by lexically-normalised path. Values are reference counted so that the
    /// same file can be "opened" multiple times without copying.
    input_files: HashMap<PathBuf, Arc<Vec<u8>>>,

    /// Files produced by the link. Written from multiple threads (output-file creation can happen
    /// on a background thread), hence the mutex.
    output_file: Mutex<Option<Vec<u8>>>,
}

impl Vfs {
    pub(crate) fn new(files: HashMap<PathBuf, Vec<u8>>) -> Self {
        Self {
            input_files: files
                .into_iter()
                .map(|(path, bytes)| (path, Arc::new(bytes)))
                .collect(),
            output_file: Mutex::new(None),
        }
    }

    pub(crate) fn contains(&self, path: &Path) -> bool {
        self.input_files.contains_key(path)
    }

    pub(crate) fn read(&self, path: &Path) -> Result<Arc<Vec<u8>>> {
        self.input_files.get(path).cloned().ok_or_else(|| {
            error!(
                "File `{}` is not present in the supplied in-memory filesystem",
                path.display()
            )
        })
    }

    pub(crate) fn set_output(&self, bytes: Vec<u8>) {
        *self.output_file.lock().unwrap() = Some(bytes);
    }

    pub(crate) fn take_output(&self) -> Option<Vec<u8>> {
        self.output_file.lock().unwrap().take()
    }
}
