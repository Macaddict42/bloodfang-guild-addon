fn main() {
    println!("cargo:rerun-if-env-changed=SITE_URL");
    println!("cargo:rerun-if-env-changed=ADDON_NAME");
    println!("cargo:rerun-if-env-changed=GUILD_NAME");
    println!("cargo:rerun-if-env-changed=GITHUB_REPO");
    tauri_build::build()
}
