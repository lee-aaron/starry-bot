
mod graphics_capture;
mod minimap_v2;
mod image_processing;

pub use graphics_capture::GraphicsCaptureService;
pub use minimap_v2::MinimapService as MinimapServiceV2;
pub use image_processing::ImageProcessingService;

#[async_trait::async_trait]
pub trait Service: Send + Sync {
  async fn start(&self) -> Result<(), ()>;
  async fn stop(&self) -> Result<(), ()>;
}
