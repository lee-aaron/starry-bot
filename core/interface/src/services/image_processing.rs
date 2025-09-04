use std::sync::Arc;
use std::time::Instant;
use std::sync::atomic::{AtomicUsize, AtomicU64, Ordering};

use tokio::sync::{Mutex, broadcast};

use super::graphics_capture::{GraphicsCaptureService, CapturedFrame};

#[derive(Debug)]
pub struct ImageProcessingMetrics {
    pub frames_processed: AtomicUsize,
    pub frames_dropped: AtomicUsize,
    pub total_processing_time_ms: AtomicU64,
    pub feature_detections: AtomicUsize,
}

impl ImageProcessingMetrics {
    pub fn new() -> Self {
        Self {
            frames_processed: AtomicUsize::new(0),
            frames_dropped: AtomicUsize::new(0),
            total_processing_time_ms: AtomicU64::new(0),
            feature_detections: AtomicUsize::new(0),
        }
    }

    pub fn get_stats(&self) -> String {
        let frames = self.frames_processed.load(Ordering::Relaxed);
        let dropped = self.frames_dropped.load(Ordering::Relaxed);
        let detections = self.feature_detections.load(Ordering::Relaxed);
        
        let fps = if self.total_processing_time_ms.load(Ordering::Relaxed) > 0 {
            (frames as f64 * 1000.0) / self.total_processing_time_ms.load(Ordering::Relaxed) as f64
        } else { 0.0 };

        format!(
            "🔍 Image Processing Service:\n\
             📊 FPS: {:.1}\n\
             📈 Frames: {} processed, {} dropped\n\
             🎯 Features detected: {}",
            fps, frames, dropped, detections
        )
    }
}

/// Example service showing how to consume graphics frames for other computer vision tasks
pub struct ImageProcessingService {
    graphics_service: Arc<GraphicsCaptureService>,
    frame_receiver: Arc<Mutex<Option<broadcast::Receiver<CapturedFrame>>>>,
    is_processing: Arc<Mutex<bool>>,
    metrics: Arc<ImageProcessingMetrics>,
}

impl ImageProcessingService {
    pub fn new(graphics_service: Arc<GraphicsCaptureService>) -> Self {
        Self {
            graphics_service,
            frame_receiver: Arc::new(Mutex::new(None)),
            is_processing: Arc::new(Mutex::new(false)),
            metrics: Arc::new(ImageProcessingMetrics::new()),
        }
    }

    /// Start processing frames for computer vision tasks
    pub async fn start_processing(&self) -> Result<(), String> {
        if *self.is_processing.lock().await {
            return Ok(()); // Already processing
        }

        // Subscribe to graphics frames
        let mut receiver = self.graphics_service.subscribe();
        *self.frame_receiver.lock().await = Some(receiver.resubscribe());
        *self.is_processing.lock().await = true;

        let metrics = self.metrics.clone();
        let is_processing = self.is_processing.clone();

        tokio::spawn(async move {
            println!("🔍 Starting image processing...");

            while *is_processing.lock().await {
                match receiver.recv().await {
                    Ok(frame) => {
                        let process_start = Instant::now();
                        
                        // Example processing tasks
                        match Self::process_frame_for_features(frame, &metrics).await {
                            Ok(_) => {
                                metrics.frames_processed.fetch_add(1, Ordering::Relaxed);
                            }
                            Err(_) => {
                                metrics.frames_dropped.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        
                        let elapsed = process_start.elapsed().as_millis() as u64;
                        metrics.total_processing_time_ms.fetch_add(elapsed, Ordering::Relaxed);
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        println!("⚠️  Image processing lagged, skipped {} frames", skipped);
                        metrics.frames_dropped.fetch_add(skipped as usize, Ordering::Relaxed);
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        println!("📺 Graphics stream closed");
                        break;
                    }
                }
            }

            println!("🛑 Image processing stopped");
        });

        Ok(())
    }

    /// Stop processing
    pub async fn stop_processing(&self) {
        *self.is_processing.lock().await = false;
        *self.frame_receiver.lock().await = None;
    }

    /// Get performance metrics
    pub fn get_metrics(&self) -> String {
        self.metrics.get_stats()
    }

    /// Example frame processing for various computer vision tasks
    async fn process_frame_for_features(
        frame: CapturedFrame,
        metrics: &ImageProcessingMetrics,
    ) -> Result<(), String> {
        // Example processing tasks you might do:
        
        // 1. Object Detection
        // - Detect enemies, items, UI elements
        // - Use YOLO, SSD, or other detection models
        
        // 2. Text Recognition (OCR)
        // - Read health/mana values, chat text, scores
        // - Use Tesseract or modern OCR models
        
        // 3. Feature Matching
        // - Template matching for specific UI elements
        // - SIFT/SURF/ORB feature detection
        
        // 4. Color Analysis
        // - Detect health bars by color
        // - Analyze minimap for enemy positions
        
        // 5. Motion Detection
        // - Track moving objects
        // - Detect scene changes
        
        // Placeholder simulation
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        
        // Simulate feature detection success rate
        if frame.width > 1000 && frame.height > 600 {
            metrics.feature_detections.fetch_add(1, Ordering::Relaxed);
        }
        
        Ok(())
    }
}

/// Example usage showing how multiple services can consume the same graphics stream
pub async fn example_usage() -> Result<(), String> {
    // Create shared graphics capture service
    let graphics_service = Arc::new(GraphicsCaptureService::new());
    
    // Create multiple consumer services
    let minimap_service = super::minimap_v2::MinimapService::new(graphics_service.clone());
    let image_processor = ImageProcessingService::new(graphics_service.clone());
    
    // Start capture for a specific window
    graphics_service.start_window_capture("Game Window").await?;
    
    // Both services will receive the same frames
    minimap_service.start_capture().await?;
    image_processor.start_processing().await?;
    
    // Let them run for a while
    tokio::time::sleep(std::time::Duration::from_secs(10)).await;
    
    // Get metrics from all services
    println!("{}", graphics_service.get_metrics());
    println!("{}", minimap_service.get_performance_metrics().unwrap_or_default());
    println!("{}", image_processor.get_metrics());
    
    // Clean shutdown
    minimap_service.stop_capture().await?;
    image_processor.stop_processing().await;
    graphics_service.stop_capture().await;
    
    Ok(())
}
