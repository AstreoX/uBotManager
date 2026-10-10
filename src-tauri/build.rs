fn main() {
    // tauri-build embeds icons/icon.ico in the Windows resource but only asks Cargo to rerun when
    // tauri.conf.json or capabilities/ change, so an existing target/ would keep the old icon.
    println!("cargo:rerun-if-changed=icons");
    tauri_build::build();
}
