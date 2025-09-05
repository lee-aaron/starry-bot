
mod graphics_capture;
pub mod minimap_v2;

pub use graphics_capture::GraphicsCaptureService;
pub use minimap_v2::{MinimapService as MinimapServiceV2, ServiceState};

#[async_trait::async_trait]
pub trait Service: Send + Sync {
  async fn start(&self) -> Result<(), ()>;
  async fn stop(&self) -> Result<(), ()>;
}
