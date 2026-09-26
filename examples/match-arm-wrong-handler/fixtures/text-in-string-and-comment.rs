struct Dispatcher;

impl Dispatcher {
    fn dispatch(&self, path: &str) {
        // Action::Copy => self.move_document(path), would be a bug here
        let note = "Action::Copy => self.move_document(path)";
        log(&note);
    }

    fn move_document(&self, path: &str) {
        let _ = path;
    }

    fn copy_document(&self, path: &str) {
        let _ = path;
    }
}
