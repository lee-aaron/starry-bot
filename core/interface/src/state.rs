use crate::services::MinimapService;

pub struct AppState {
    pub minimap: Box<MinimapService>,
}
