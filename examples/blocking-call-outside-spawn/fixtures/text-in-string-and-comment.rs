fn on_input(text: &str) {
    // completion::suggest(text) would block the UI thread here
    let note = "completion::suggest(text)";
    log(&note);
}
