fn main() {
    glib_build_tools::compile_resources(
        &["src/assets"],
        "src/assets/mimick.gresource.xml",
        "mimick.gresource",
    );
}
