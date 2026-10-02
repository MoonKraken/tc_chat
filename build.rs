fn main() {
    topcoat::tailwind::BuildConfig::new()
        .input("src/app.css")
        .render()
        .unwrap();
}
