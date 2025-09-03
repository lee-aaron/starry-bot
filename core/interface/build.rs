fn main() {
    #[allow(unused_mut)]
    let mut windows = tauri_build::WindowsAttributes::new();

    #[cfg(all(windows, debug_assertions))]
    {
        windows = windows.app_manifest(include_str!("manifest.xml"));
    }

    let attrs = tauri_build::Attributes::new().windows_attributes(windows);
    tauri_build::try_build(attrs).expect("failed to run build script");
}
