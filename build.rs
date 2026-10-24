// build.rs
//
// Embeds the Windows executable icon and version metadata.
// Runs only when building for Windows; safe to keep in the tree for
// cross-platform CI because the CARGO_CFG_TARGET_OS guard skips it.

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winres::WindowsResource::new();
        res.set_icon("assets/reeve.ico");
        res.set("ProductName", "Reeve");
        res.set("FileDescription", "Native PWA Launcher");
        res.set("LegalCopyright", "MIT License");
        res.set("ProductVersion", "0.1.0");
        res.set("FileVersion", "0.1.0");
        res.set("OriginalFilename", "reeve.exe");
        res.compile().expect("failed to compile Windows resources");
    }
}