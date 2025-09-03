
mod minimap;

pub use minimap::MinimapService;

#[async_trait::async_trait]
pub trait Service: Send + Sync {
  async fn start(&self) -> Result<(), ()>;
  async fn stop(&self) -> Result<(), ()>;
}
