//! Windows: Null's icon and version built into null.exe (packaging/windows). Only when
//! building on Windows, where its resource compiler is; elsewhere (and when a Mac
//! type-checks for Windows) nothing to do.

fn main() {
    println!("cargo:rerun-if-changed=packaging/windows/null.rc");
    println!("cargo:rerun-if-changed=packaging/windows/null.ico");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let dir = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
        let version = env!("CARGO_PKG_VERSION");
        let template = std::fs::read_to_string(dir.join("packaging/windows/null.rc")).unwrap();
        // (The resource compiler reads `\` as an escape: the icon's path with `/`.)
        let icon = dir.join("packaging/windows/null.ico").display().to_string().replace('\\', "/");
        let filled = template
            .replace("__ICON__", &icon)
            .replace("__COMMAS__", &format!("{},0", version.replace('.', ",")))
            .replace("__VERSION__", version);
        let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("null.rc");
        std::fs::write(&out, filled).unwrap();
        embed_resource::compile(&out, embed_resource::NONE).manifest_optional().unwrap();
    }
}
