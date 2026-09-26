fn on_input(text: String) {
    thread::spawn(move || {
        completion::suggest(&text);
    });
}
