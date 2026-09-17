static HOME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub(crate) struct Sandbox {
    _guard: std::sync::MutexGuard<'static, ()>,
    directory: tempfile::TempDir,
}

impl Sandbox {
    pub(crate) fn new() -> Self {
        let guard = HOME_LOCK.lock().unwrap_or_else(|error| {
            HOME_LOCK.clear_poison();
            error.into_inner()
        });
        let directory = tempfile::tempdir().expect("temp dir");
        unsafe {
            std::env::set_var(super::HOME_VARIABLE, directory.path());
        }
        Self {
            _guard: guard,
            directory,
        }
    }

    pub(crate) fn path(&self) -> &std::path::Path {
        self.directory.path()
    }
}
