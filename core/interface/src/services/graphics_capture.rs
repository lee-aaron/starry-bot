use std::sync::Arc;
use std::time::{Duration, Instant};
use std::sync::atomic::{AtomicUsize, AtomicU64, Ordering};

use platforms::windows_capture::{
    capture::{CaptureControl, GraphicsCaptureApiHandler, Context},
    graphics_capture_api::InternalCaptureControl,
    settings::{
        ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
        MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
    },
    window::Window,
    dxgi_desktop_duplication::{DxgiDesktopDuplication, DxgiError},
    texture_processor::{TextureProcessor, FrameFormat as PlatformFrameFormat},
};
use tokio::sync::{broadcast, Mutex};

/// Raw frame data with metadata
#[derive(Clone, Debug)]
pub struct CapturedFrame {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub format: FrameFormat,
    pub timestamp: Instant,
    pub source: CaptureSource,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FrameFormat {
    Bgra8,
    Rgba8,
    Rgb8,
    Jpeg,
}

#[derive(Clone, Debug)]
pub enum CaptureSource {
    WindowsGraphicsCapture,
    DxgiDesktopDuplication,
}

#[derive(Debug)]
pub struct CaptureMetrics {
    pub frames_captured: AtomicUsize,
    pub frames_dropped: AtomicUsize,
    pub total_capture_time_ms: AtomicU64,
    pub active_subscribers: AtomicUsize,
}

impl CaptureMetrics {
    pub fn new() -> Self {
        Self {
            frames_captured: AtomicUsize::new(0),
            frames_dropped: AtomicUsize::new(0),
            total_capture_time_ms: AtomicU64::new(0),
            active_subscribers: AtomicUsize::new(0),
        }
    }

    pub fn get_fps(&self) -> f64 {
        let frames = self.frames_captured.load(Ordering::Relaxed) as f64;
        let time_ms = self.total_capture_time_ms.load(Ordering::Relaxed) as f64;
        if time_ms > 0.0 { (frames * 1000.0) / time_ms } else { 0.0 }
    }

    pub fn get_stats(&self) -> String {
        format!(
            "📊 Graphics Capture Service:\n\
             🎯 FPS: {:.1}\n\
             📈 Frames: {} captured, {} dropped\n\
             👥 Active subscribers: {}\n\
             📺 Source: Mixed (Windows Graphics Capture + DXGI)",
            self.get_fps(),
            self.frames_captured.load(Ordering::Relaxed),
            self.frames_dropped.load(Ordering::Relaxed),
            self.active_subscribers.load(Ordering::Relaxed)
        )
    }
}

/// High-performance graphics capture service with multiple consumers
#[derive(Clone)]
pub struct GraphicsCaptureService {
    // Broadcast channel for multiple subscribers
    frame_broadcast: broadcast::Sender<CapturedFrame>,
    
    // Current capture state
    capture_control: Arc<Mutex<Option<CaptureControl<FrameHandler, ()>>>>,
    current_window: Arc<Mutex<Option<Window>>>,
    
    // Performance metrics
    metrics: Arc<CaptureMetrics>,
    
    // DXGI fallback for high-performance mode
    dxgi_capture: Arc<Mutex<Option<DxgiCapture>>>,
}

struct FrameHandler {
    frame_broadcast: broadcast::Sender<CapturedFrame>,
    metrics: Arc<CaptureMetrics>,
}

impl GraphicsCaptureApiHandler for FrameHandler {
    type Flags = (broadcast::Sender<CapturedFrame>, Arc<CaptureMetrics>);
    type Error = ();

    fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
        Ok(Self {
            frame_broadcast: ctx.flags.0,
            metrics: ctx.flags.1,
        })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut platforms::windows_capture::frame::Frame,
        _control: InternalCaptureControl,
    ) -> Result<(), Self::Error> {
        let capture_start = Instant::now();

        // Fast frame processing - minimal work in capture callback
        if let Ok(mut frame_buffer) = frame.buffer() {
            let width = frame_buffer.width();
            let height = frame_buffer.height();
            
            if let Ok(buffer) = frame_buffer.as_nopadding_buffer() {
                // Convert BGRA to RGBA efficiently
                let mut rgba_data = Vec::with_capacity(buffer.len());
                for chunk in buffer.chunks_exact(4) {
                    rgba_data.extend_from_slice(&[chunk[2], chunk[1], chunk[0], chunk[3]]);
                }
                
                let captured_frame = CapturedFrame {
                    data: rgba_data,
                    width,
                    height,
                    format: FrameFormat::Rgba8,
                    timestamp: capture_start,
                    source: CaptureSource::WindowsGraphicsCapture,
                };

                // Broadcast to all subscribers (non-blocking)
                let subscriber_count = self.frame_broadcast.receiver_count();
                self.metrics.active_subscribers.store(subscriber_count, Ordering::Relaxed);
                
                match self.frame_broadcast.send(captured_frame) {
                    Ok(_) => {
                        self.metrics.frames_captured.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(_) => {
                        self.metrics.frames_dropped.fetch_add(1, Ordering::Relaxed);
                    }
                }

                let elapsed = capture_start.elapsed().as_millis() as u64;
                self.metrics.total_capture_time_ms.fetch_add(elapsed, Ordering::Relaxed);
            }
        }

        Ok(())
    }
}

/// DXGI Desktop Duplication for high-performance capture
struct DxgiCapture {
    duplication: DxgiDesktopDuplication,
    texture_processor: TextureProcessor,
    frame_broadcast: broadcast::Sender<CapturedFrame>,
    metrics: Arc<CaptureMetrics>,
}

impl DxgiCapture {
    pub fn new(
        frame_broadcast: broadcast::Sender<CapturedFrame>,
        metrics: Arc<CaptureMetrics>,
    ) -> Result<Self, String> {
        let mut duplication = DxgiDesktopDuplication::new()
            .map_err(|e| format!("Failed to create DXGI duplication: {}", e))?;
        
        duplication.initialize_primary_output()
            .map_err(|e| format!("Failed to initialize primary output: {}", e))?;
        
        let texture_processor = TextureProcessor::new(
            duplication.device.clone(),
            duplication.context.clone(),
        );
        
        Ok(Self {
            duplication,
            texture_processor,
            frame_broadcast,
            metrics,
        })
    }
    
    pub async fn start_capture_loop(&mut self) -> Result<(), String> {
        loop {
            let capture_start = Instant::now();
            
            match self.duplication.capture_frame() {
                Ok(Some(texture)) => {
                    // Use platforms-based texture processing
                    if let Ok(processed_frame) = self.texture_processor.extract_frame_data(&texture) {
                        // Convert from platforms format to interface format
                        let frame_data = CapturedFrame {
                            data: processed_frame.data,
                            width: processed_frame.width,
                            height: processed_frame.height,
                            format: self.convert_platform_format(processed_frame.format),
                            timestamp: processed_frame.timestamp,
                            source: CaptureSource::DxgiDesktopDuplication,
                        };
                        
                        let subscriber_count = self.frame_broadcast.receiver_count();
                        self.metrics.active_subscribers.store(subscriber_count, Ordering::Relaxed);
                        
                        match self.frame_broadcast.send(frame_data) {
                            Ok(_) => {
                                self.metrics.frames_captured.fetch_add(1, Ordering::Relaxed);
                            }
                            Err(_) => {
                                self.metrics.frames_dropped.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }
                    
                    let elapsed = capture_start.elapsed().as_millis() as u64;
                    self.metrics.total_capture_time_ms.fetch_add(elapsed, Ordering::Relaxed);
                }
                Ok(None) => {
                    // No new frame - normal for DXGI
                    continue;
                }
                Err(DxgiError::AccessLost) => {
                    // Need to recreate duplication
                    self.duplication.reset();
                    self.duplication.initialize_primary_output()
                        .map_err(|e| format!("Failed to reinitialize after access lost: {}", e))?;
                    continue;
                }
                Err(DxgiError::Timeout) => {
                    // No new frame - normal
                    continue;
                }
                Err(e) => return Err(format!("DXGI capture error: {}", e)),
            }
            
            // Small delay to target ~30 FPS
            tokio::time::sleep(Duration::from_millis(33)).await;
        }
    }

    pub async fn capture_frame_for_minimap(&mut self) -> Result<CapturedFrame, String> {
        // Use the DXGI capture_frame_for_minimap method which handles texture processing internally
        let processed_frame = self.duplication.capture_frame_for_minimap()
            .map_err(|e| format!("Failed to capture frame for minimap: {}", e))?;
        
        if let Some(frame) = processed_frame {
            // Convert from platforms format to interface format
            Ok(CapturedFrame {
                data: frame.data,
                width: frame.width,
                height: frame.height,
                format: self.convert_platform_format(frame.format),
                timestamp: frame.timestamp,
                source: CaptureSource::DxgiDesktopDuplication,
            })
        } else {
            Err("No frame available".to_string())
        }
    }
    
    fn convert_platform_format(&self, format: PlatformFrameFormat) -> FrameFormat {
        match format {
            PlatformFrameFormat::Bgra8 => FrameFormat::Bgra8,
            PlatformFrameFormat::Rgba8 => FrameFormat::Rgba8,
            PlatformFrameFormat::Rgb8 => FrameFormat::Rgb8,
            PlatformFrameFormat::Jpeg => FrameFormat::Jpeg,
        }
    }
}

impl GraphicsCaptureService {
    pub fn new() -> Self {
        // Create broadcast channel with buffer for multiple subscribers
        let (frame_broadcast, _) = broadcast::channel(100);
        let metrics = Arc::new(CaptureMetrics::new());
        
        Self {
            frame_broadcast,
            capture_control: Arc::new(Mutex::new(None)),
            current_window: Arc::new(Mutex::new(None)),
            metrics,
            dxgi_capture: Arc::new(Mutex::new(None)),
        }
    }

    /// Subscribe to frame updates - each subscriber gets their own stream
    pub fn subscribe(&self) -> broadcast::Receiver<CapturedFrame> {
        self.frame_broadcast.subscribe()
    }

    /// Start Windows Graphics Capture for specific window
    pub async fn start_window_capture(&self, window_title: &str) -> Result<(), String> {
        let window = Window::from_contains_name(window_title)
            .map_err(|_| format!("Window '{}' not found", window_title))?;

        *self.current_window.lock().await = Some(window.clone());

        let settings = Settings::new(
            window,
            CursorCaptureSettings::WithoutCursor,
            DrawBorderSettings::Default,
            SecondaryWindowSettings::Default,
            MinimumUpdateIntervalSettings::Custom(Duration::from_millis(33)), // 30 FPS target
            DirtyRegionSettings::Default,
            ColorFormat::Bgra8,
            (self.frame_broadcast.clone(), self.metrics.clone()),
        );

        match FrameHandler::start_free_threaded(settings) {
            Ok(capture_control) => {
                *self.capture_control.lock().await = Some(capture_control);
                Ok(())
            }
            Err(_) => Err("Failed to start Windows Graphics Capture".to_string()),
        }
    }

    /// Start DXGI Desktop Duplication for maximum performance
    pub async fn start_dxgi_capture(&self) -> Result<(), String> {
        let dxgi = DxgiCapture::new(self.frame_broadcast.clone(), self.metrics.clone())
            .map_err(|e| format!("Failed to create DXGI capture: {:?}", e))?;

        // Store the capture instance
        *self.dxgi_capture.lock().await = Some(dxgi);

        // Start capture loop in background task
        let dxgi_ref = self.dxgi_capture.clone();
        tokio::spawn(async move {
            if let Some(dxgi) = dxgi_ref.lock().await.as_mut() {
                if let Err(e) = dxgi.start_capture_loop().await {
                    eprintln!("DXGI capture failed: {:?}", e);
                }
            }
        });

        Ok(())
    }

    /// Stop all capture
    pub async fn stop_capture(&self) {
        // Stop Windows Graphics Capture
        if let Some(control) = self.capture_control.lock().await.take() {
            let _ = control.stop();
        }

        // Stop DXGI capture
        *self.dxgi_capture.lock().await = None;
    }

    /// Get performance metrics
    pub fn get_metrics(&self) -> String {
        self.metrics.get_stats()
    }

    /// Check if actively capturing
    pub async fn is_capturing(&self) -> bool {
        self.capture_control.lock().await.is_some() || 
        self.dxgi_capture.lock().await.is_some()
    }
    
    /// Configure GPU processing for DXGI capture
    /// Set to false to use CPU processing (more stable, slower)
    /// Set to true to use GPU processing (faster, may have compatibility issues)
    pub async fn set_gpu_processing(&self, enabled: bool) {
        if let Some(dxgi) = self.dxgi_capture.lock().await.as_mut() {
            dxgi.duplication.set_gpu_processing(enabled);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_multiple_subscribers() {
        let service = GraphicsCaptureService::new();
        
        // Create multiple subscribers
        let sub1 = service.subscribe();
        let sub2 = service.subscribe();
        
        // Both should receive the same frames
        // Test would require actual capture to validate
        assert!(sub1.len() == 0);
        assert!(sub2.len() == 0);
    }
}
