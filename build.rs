fn main() {
    println!("cargo:rerun-if-changed=assets/icons/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icons/icon.ico")
            .set("ProductName", "MD Reader")
            .set("FileDescription", "MD Reader")
            .set("LegalCopyright", "© DeemMoor");
        res.compile().expect("failed to embed Windows resources");
    }
}
