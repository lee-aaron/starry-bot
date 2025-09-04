#[cfg(all(windows, not(debug_assertions)))]
fn main() {
    // Only embed manifest for release builds on Windows
    println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg=/MANIFESTINPUT:../interface/manifest.xml");
    println!("cargo:rerun-if-changed=../interface/manifest.xml");
}

#[cfg(not(all(windows, not(debug_assertions))))]
fn main() {
    // Do nothing for debug builds or non-Windows platforms
}
