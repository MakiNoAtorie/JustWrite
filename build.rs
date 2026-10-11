fn main() {
    // The icon of the Windows program file; the id is the one the window asks for.
    #[cfg(windows)]
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        winresource::WindowsResource::new()
            .set_icon_with_id("packaging/windows/justwrite.ico", "1")
            .compile()
            .expect("Cannot put the icon in the program file");
    }
}
