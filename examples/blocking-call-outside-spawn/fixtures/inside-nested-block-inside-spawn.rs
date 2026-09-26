fn on_input(text: String, ready: bool) {
    thread::spawn(move || {
        if ready {
            completion::suggest(&text);
        }
    });
}
