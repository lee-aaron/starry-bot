use platforms::windows_capture::window::Window;

pub mod services;

// Public API for the interface library
pub use services::{Service, GraphicsCaptureService, MinimapServiceV2};

/// Initialize the platforms subsystem
pub fn init() {
    platforms::init();
}

/// List all available windows by title
pub fn list_window_handles() -> Vec<String> {
    Window::enumerate()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|w| w.title().ok())
        .collect()
}
