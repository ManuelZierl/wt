enum Action {
    Copy,
    Move,
}

struct Dispatcher;

impl Dispatcher {
    fn dispatch(&self, action: Action, path: &str) {
        match action {
            Action::Copy => self.copy_document(path),
            Action::Move => self.move_document(path),
        }
    }

    fn move_document(&self, path: &str) {
        let _ = path;
    }

    fn copy_document(&self, path: &str) {
        let _ = path;
    }
}
