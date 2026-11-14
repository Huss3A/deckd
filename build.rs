// Gives deckd.exe its icon (Explorer, taskbar pins, Task Manager). The icon is drawn by src/icon.rs.
#[path = "src/icon.rs"]
mod icon;

fn main() {
    println!("cargo:rerun-if-changed=src/icon.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let path = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("deckd.ico");
    std::fs::write(&path, icon::ico_file(&[16, 32, 48, 256])).unwrap();
    let mut res = winresource::WindowsResource::new();
    res.set_icon(path.to_str().unwrap());
    res.set("ProductName", "deckd");
    res.set("FileDescription", "deckd - Ulanzi D200H deck");
    if let Err(e) = res.compile() {
        println!("cargo:warning=could not embed the exe icon: {e}");
    }
}
