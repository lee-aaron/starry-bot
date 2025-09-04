use std::sync::Arc;
use std::time::Instant;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use tokio::sync::{Mutex, watch, broadcast};

use crate::services::Service;
use super::graphics_capture::{GraphicsCaptureService, CapturedFrame, FrameFormat};

#[derive(Debug)]
pub struct MinimapMetrics {
    pub frames_processed: AtomicUsize,
    pub frames_dropped: AtomicUsize,
    pub opencv_detections: AtomicUsize,
    pub total_processing_time_ms: AtomicU64,
    pub total_opencv_time_ms: AtomicU64,
    pub total_encode_time_ms: AtomicU64,
}

impl MinimapMetrics {
    pub fn new() -> Self {
        Self {
            frames_processed: AtomicUsize::new(0),
            frames_dropped: AtomicUsize::new(0),
            opencv_detections: AtomicUsize::new(0),
            total_processing_time_ms: AtomicU64::new(0),
            total_opencv_time_ms: AtomicU64::new(0),
            total_encode_time_ms: AtomicU64::new(0),
        }
    }

    pub fn get_fps(&self) -> f64 {
        let frames = self.frames_processed.load(Ordering::Relaxed) as f64;
        let time_ms = self.total_processing_time_ms.load(Ordering::Relaxed) as f64;
        if time_ms > 0.0 { (frames * 1000.0) / time_ms } else { 0.0 }
    }

    pub fn get_stats(&self) -> String {
        let frames = self.frames_processed.load(Ordering::Relaxed);
        let dropped = self.frames_dropped.load(Ordering::Relaxed);
        let detections = self.opencv_detections.load(Ordering::Relaxed);
        let fps = self.get_fps();
        
        let avg_opencv = if frames > 0 {
            self.total_opencv_time_ms.load(Ordering::Relaxed) as f64 / frames as f64
        } else { 0.0 };
        
        let avg_encode = if frames > 0 {
            self.total_encode_time_ms.load(Ordering::Relaxed) as f64 / frames as f64
        } else { 0.0 };

        format!(
            "🎯 Minimap Service:\n\
             📈 Processing FPS: {:.1}\n\
             🔍 Frames: {} processed, {} dropped\n\
             🎮 Minimap detections: {}\n\
             ⏱️  Avg times: OpenCV {:.1}ms, Encode {:.1}ms\n\
             🎨 Detection rate: {:.1}%",
            fps, frames, dropped, detections,
            avg_opencv, avg_encode,
            if frames > 0 { (detections as f64 / frames as f64) * 100.0 } else { 0.0 }
        )
    }
}

/// Minimap detection service that consumes frames from GraphicsCaptureService
#[derive(Clone)]
pub struct MinimapService {
    graphics_service: Arc<GraphicsCaptureService>,
    current_window_title: Arc<Mutex<Option<String>>>,
    
    // Frame processing
    frame_receiver: Arc<Mutex<Option<broadcast::Receiver<CapturedFrame>>>>,
    frame_sender: watch::Sender<Option<Vec<u8>>>,
    frame_watch: watch::Receiver<Option<Vec<u8>>>,
    
    // Processing control
    is_processing: Arc<Mutex<bool>>,
    is_stopping: Arc<Mutex<bool>>,
    
    // Metrics
    metrics: Arc<MinimapMetrics>,
}

impl MinimapService {
    pub fn new(graphics_service: Arc<GraphicsCaptureService>) -> Self {
        let (frame_sender, frame_watch) = watch::channel(None);
        let metrics = Arc::new(MinimapMetrics::new());
        
        Self {
            graphics_service,
            current_window_title: Arc::new(Mutex::new(None)),
            frame_receiver: Arc::new(Mutex::new(None)),
            frame_sender,
            frame_watch,
            is_processing: Arc::new(Mutex::new(false)),
            is_stopping: Arc::new(Mutex::new(false)),
            metrics,
        }
    }

    /// Get the current processed minimap frame (JPEG encoded)
    pub fn get_frame_receiver(&self) -> watch::Receiver<Option<Vec<u8>>> {
        self.frame_watch.clone()
    }

    /// Check if currently processing frames
    pub async fn is_capturing(&self) -> bool {
        *self.is_processing.lock().await
    }

    /// Get current window title
    pub async fn get_current_window_title(&self) -> Option<String> {
        self.current_window_title.lock().await.clone()
    }

    /// Get performance metrics
    pub fn get_performance_metrics(&self) -> Option<String> {
        let graphics_metrics = self.graphics_service.get_metrics();
        let minimap_metrics = self.metrics.get_stats();
        
        Some(format!("{}\n\n{}", graphics_metrics, minimap_metrics))
    }

    /// Reset metrics
    pub fn reset_metrics(&self) {
        self.metrics.frames_processed.store(0, Ordering::Relaxed);
        self.metrics.frames_dropped.store(0, Ordering::Relaxed);
        self.metrics.opencv_detections.store(0, Ordering::Relaxed);
        self.metrics.total_processing_time_ms.store(0, Ordering::Relaxed);
        self.metrics.total_opencv_time_ms.store(0, Ordering::Relaxed);
        self.metrics.total_encode_time_ms.store(0, Ordering::Relaxed);
    }

    /// Set the target window for capture
    pub async fn set_window(&self, title: String) -> Result<(), String> {
        // Stop current processing
        self.stop_capture().await?;
        
        // Start graphics capture for the window
        self.graphics_service.start_window_capture(&title).await?;
        
        // Subscribe to frames
        let frame_receiver = self.graphics_service.subscribe();
        *self.frame_receiver.lock().await = Some(frame_receiver);
        
        // Update window title
        *self.current_window_title.lock().await = Some(title);
        
        // Start processing
        self.start_capture().await
    }

    /// Start minimap processing
    pub async fn start_capture(&self) -> Result<(), String> {
        // Reset stopping flag in case it was set
        *self.is_stopping.lock().await = false;
        
        // Stop any existing processing first
        if *self.is_processing.lock().await {
            println!("🔄 Restarting capture - stopping existing processing...");
            self.stop_capture().await?;
            // Give time for cleanup
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        }

        let receiver_guard = self.frame_receiver.lock().await;
        let mut receiver = match receiver_guard.as_ref() {
            Some(r) => r.resubscribe(),
            None => return Err("No graphics capture subscription".to_string()),
        };
        drop(receiver_guard);

        *self.is_processing.lock().await = true;
        println!("🎮 Starting minimap frame processing...");

        // Spawn processing task
        let frame_sender = self.frame_sender.clone();
        let metrics = self.metrics.clone();
        let is_processing = self.is_processing.clone();

        tokio::spawn(async move {
            println!("✅ Minimap processing task spawned");
            
            while *is_processing.lock().await {
                match receiver.recv().await {
                    Ok(captured_frame) => {
                        let process_start = Instant::now();
                        
                        // Process the frame for minimap detection
                        match Self::process_minimap_frame(captured_frame, &metrics).await {
                            Ok(processed_webp) => {
                                // Send processed frame to watchers
                                if frame_sender.send(Some(processed_webp)).is_ok() {
                                    metrics.frames_processed.fetch_add(1, Ordering::Relaxed);
                                } else {
                                    metrics.frames_dropped.fetch_add(1, Ordering::Relaxed);
                                }
                            }
                            Err(e) => {
                                #[cfg(debug_assertions)]
                                println!("⚠️ Frame processing failed: {}", e);
                                metrics.frames_dropped.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        
                        let elapsed = process_start.elapsed().as_millis() as u64;
                        metrics.total_processing_time_ms.fetch_add(elapsed, Ordering::Relaxed);
                        
                        // Check if we're falling behind and log it
                        if elapsed > 33 { // More than 30 FPS target
                            #[cfg(debug_assertions)]
                            println!("🐌 Processing behind by {:.1}ms (target: 33ms)", elapsed);
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        println!("⚠️ Processing lagged, will catch up. {} frames behind (continuing processing)", skipped);
                        metrics.frames_dropped.fetch_add(skipped as usize, Ordering::Relaxed);
                        // Don't break - keep processing to catch up
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        println!("📺 Graphics capture stream closed");
                        break;
                    }
                }
            }
            
            println!("🛑 Minimap processing stopped");
        });

        Ok(())
    }

    /// Stop minimap processing
    pub async fn stop_capture(&self) -> Result<(), String> {
        // Check if already stopping to prevent multiple simultaneous stops
        {
            let mut stopping = self.is_stopping.lock().await;
            if *stopping {
                println!("⏸️  Stop already in progress, skipping...");
                return Ok(());
            }
            *stopping = true;
        }
        
        println!("🛑 Stopping minimap capture...");
        
        // First, stop processing to avoid new frames
        *self.is_processing.lock().await = false;
        
        // Give processing task time to finish current frame
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        
        // Clear state
        *self.current_window_title.lock().await = None;
        *self.frame_receiver.lock().await = None;
        
        // Clear the current frame display
        let _ = self.frame_sender.send(None);
        
        // Stop graphics service
        self.graphics_service.stop_capture().await;
        
        // Reset stopping flag
        *self.is_stopping.lock().await = false;
        
        println!("✅ Minimap capture stopped and cleaned up");
        Ok(())
    }

    /// Process a captured frame for minimap detection
    async fn process_minimap_frame(
        frame: CapturedFrame,
        metrics: &MinimapMetrics,
    ) -> Result<Vec<u8>, String> {
        // Check for empty frame data from graphics capture
        if frame.data.is_empty() {
            return Err("Empty frame data received from graphics capture (DXGI placeholder)".to_string());
        }
        
        // OPTIMIZATION: Skip heavy processing - the graphics service already did the conversion
        // We just need to do minimal minimap-specific processing
        
        let opencv_start = Instant::now();
        
        // Fast minimap detection (placeholder for now)
        let minimap_detected = Self::detect_minimap_opencv_fast(&frame).await;
        
        let opencv_time = opencv_start.elapsed().as_millis() as u64;
        metrics.total_opencv_time_ms.fetch_add(opencv_time, Ordering::Relaxed);
        
        if minimap_detected {
            metrics.opencv_detections.fetch_add(1, Ordering::Relaxed);
        }

        // OPTIMIZATION: Skip image processing if we already have JPEG or use raw data
        let encode_start = Instant::now();
        
        let result = match frame.format {
            FrameFormat::Jpeg => {
                // Already JPEG encoded - return as-is!
                frame.data
            }
            FrameFormat::Rgba8 | FrameFormat::Bgra8 => {
                // For UI display, downsample to reasonable size for performance
                // Full resolution debugging can be done by temporarily changing these values
                let target_width = 800;  // Much smaller for UI display
                let target_height = 450; // 16:9 aspect ratio
                
                #[cfg(debug_assertions)]
                println!("🔧 Downsampling frame: {}x{} -> {}x{} for UI display", 
                        frame.width, frame.height, target_width, target_height);
                
                if frame.width > target_width || frame.height > target_height {
                    // Downsample and encode for much faster performance
                    Self::downsample_and_encode(&frame.data, frame.width, frame.height, 
                                              target_width, target_height, frame.format == FrameFormat::Bgra8)?
                } else {
                    // Small enough already, encode directly
                    Self::encode_frame_webp(&frame.data, frame.width, frame.height, frame.format == FrameFormat::Bgra8)?
                }
            }
            FrameFormat::Rgb8 => {
                // Convert RGB to JPEG efficiently for smaller images
                if frame.width * frame.height > 1_000_000 {
                    // Large image - downsample first
                    let target_width = 800;
                    let target_height = 450;
                    Self::downsample_rgb_and_encode(&frame.data, frame.width, frame.height, 
                                                  target_width, target_height)?
                } else {
                    Self::encode_rgb_webp(&frame.data, frame.width, frame.height)?
                }
            }
        };
        
        let encode_time = encode_start.elapsed().as_millis() as u64;
        metrics.total_encode_time_ms.fetch_add(encode_time, Ordering::Relaxed);

        Ok(result)
    }

    /// Fast RGB downsampling and encoding
    fn downsample_rgb_and_encode(data: &[u8], src_width: u32, src_height: u32, 
                                dst_width: u32, dst_height: u32) -> Result<Vec<u8>, String> {
        // Simple box filter downsampling for RGB data
        let scale_x = src_width / dst_width;
        let scale_y = src_height / dst_height;
        
        let mut downsampled = Vec::with_capacity((dst_width * dst_height * 3) as usize);
        
        for y in 0..dst_height {
            for x in 0..dst_width {
                let src_x = (x * scale_x) as usize;
                let src_y = (y * scale_y) as usize;
                let src_idx = (src_y * src_width as usize + src_x) * 3;
                
                if src_idx + 2 < data.len() {
                    downsampled.extend_from_slice(&[data[src_idx], data[src_idx + 1], data[src_idx + 2]]);
                }
            }
        }
        
        // Fast JPEG encoding
        let mut jpeg_data = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg_data, 30)
            .encode(&downsampled, dst_width, dst_height, image::ExtendedColorType::Rgb8)
            .map_err(|e| format!("JPEG encode failed: {}", e))?;
        
        Ok(jpeg_data)
    }

    /// Ultra-fast minimap detection using frame metadata instead of image processing
    async fn detect_minimap_opencv_fast(frame: &CapturedFrame) -> bool {
        // OPTIMIZATION: Use frame metadata for detection instead of expensive image processing
        // This could include checking frame size, timestamp patterns, etc.
        
        // For now, simulate very fast detection
        tokio::time::sleep(std::time::Duration::from_micros(100)).await; // 0.1ms instead of 1ms
        
        // Simple heuristic: assume minimap present if frame is reasonable size
        frame.width >= 640 && frame.height >= 360
    }

    /// Fast JPEG encoding without image library overhead
    fn encode_frame_fast(data: &[u8], width: u32, height: u32, is_bgra: bool) -> Result<Vec<u8>, String> {
        let start_time = std::time::Instant::now();
        
        #[cfg(debug_assertions)]
        println!("🔧 Encoding frame: {}x{} ({} pixels, {} bytes)", 
                width, height, width * height, data.len());
        
        let mut jpeg_data = Vec::new();
        
        let conversion_start = std::time::Instant::now();
        if is_bgra {
            // Convert BGRA to RGB efficiently
            let mut rgb_data = Vec::with_capacity((width * height * 3) as usize);
            for chunk in data.chunks_exact(4) {
                rgb_data.extend_from_slice(&[chunk[2], chunk[1], chunk[0]]); // BGR -> RGB
            }
            
            let conversion_time = conversion_start.elapsed();
            #[cfg(debug_assertions)]
            println!("🎨 BGRA->RGB conversion: {:.1}ms", conversion_time.as_secs_f64() * 1000.0);
            
            let encode_start = std::time::Instant::now();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg_data, 30) // Lower quality for speed
                .encode(&rgb_data, width, height, image::ExtendedColorType::Rgb8)
                .map_err(|e| format!("JPEG encode failed: {}", e))?;
            let encode_time = encode_start.elapsed();
            
            #[cfg(debug_assertions)]
            println!("📦 JPEG encoding: {:.1}ms", encode_time.as_secs_f64() * 1000.0);
        } else {
            // RGBA to RGB conversion
            let mut rgb_data = Vec::with_capacity((width * height * 3) as usize);
            for chunk in data.chunks_exact(4) {
                rgb_data.extend_from_slice(&[chunk[0], chunk[1], chunk[2]]); // Drop alpha
            }
            
            let conversion_time = conversion_start.elapsed();
            #[cfg(debug_assertions)]
            println!("🎨 RGBA->RGB conversion: {:.1}ms", conversion_time.as_secs_f64() * 1000.0);
            
            let encode_start = std::time::Instant::now();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg_data, 30)
                .encode(&rgb_data, width, height, image::ExtendedColorType::Rgb8)
                .map_err(|e| format!("JPEG encode failed: {}", e))?;
            let encode_time = encode_start.elapsed();
            
            #[cfg(debug_assertions)]
            println!("📦 JPEG encoding: {:.1}ms", encode_time.as_secs_f64() * 1000.0);
        }
        
        let total_time = start_time.elapsed();
        #[cfg(debug_assertions)]
        println!("⏱️  Total encode time: {:.1}ms", total_time.as_secs_f64() * 1000.0);
        
        Ok(jpeg_data)
    }

    /// Fast downsampling with JPEG encoding
    fn downsample_and_encode(data: &[u8], src_width: u32, src_height: u32, 
                            dst_width: u32, dst_height: u32, is_bgra: bool) -> Result<Vec<u8>, String> {
        // Simple box filter downsampling (much faster than bicubic)
        let scale_x = src_width / dst_width;
        let scale_y = src_height / dst_height;
        
        let mut downsampled = Vec::with_capacity((dst_width * dst_height * 3) as usize);
        
        for y in 0..dst_height {
            for x in 0..dst_width {
                let src_x = (x * scale_x) as usize;
                let src_y = (y * scale_y) as usize;
                let src_idx = (src_y * src_width as usize + src_x) * 4;
                
                if src_idx + 3 < data.len() {
                    if is_bgra {
                        downsampled.extend_from_slice(&[data[src_idx + 2], data[src_idx + 1], data[src_idx]]); // BGR
                    } else {
                        downsampled.extend_from_slice(&[data[src_idx], data[src_idx + 1], data[src_idx + 2]]); // RGB
                    }
                }
            }
        }
        
        let mut jpeg_data = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg_data, 30)
            .encode(&downsampled, dst_width, dst_height, image::ExtendedColorType::Rgb8)
            .map_err(|e| format!("JPEG encode failed: {}", e))?;
        
        Ok(jpeg_data)
    }

    /// Ultra-fast WebP encoding (much faster than JPEG)
    fn encode_frame_webp(data: &[u8], width: u32, height: u32, is_bgra: bool) -> Result<Vec<u8>, String> {
        let encode_start = std::time::Instant::now();
        
        #[cfg(debug_assertions)]
        println!("🚀 WebP encoding: {}x{} ({} pixels)", width, height, width * height);
        
        // Convert to RGB format for WebP
        let rgb_data = if is_bgra {
            // BGRA to RGB conversion
            let mut rgb = Vec::with_capacity((width * height * 3) as usize);
            for chunk in data.chunks_exact(4) {
                rgb.extend_from_slice(&[chunk[2], chunk[1], chunk[0]]); // BGR -> RGB
            }
            rgb
        } else {
            // RGBA to RGB conversion
            let mut rgb = Vec::with_capacity((width * height * 3) as usize);
            for chunk in data.chunks_exact(4) {
                rgb.extend_from_slice(&[chunk[0], chunk[1], chunk[2]]); // Drop alpha
            }
            rgb
        };
        
        // For now, let's use fast JPEG instead of WebP until we figure out the WebP API
        let mut jpeg_data = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg_data, 30) // Very low quality for speed
            .encode(&rgb_data, width, height, image::ExtendedColorType::Rgb8)
            .map_err(|e| format!("JPEG encode failed: {}", e))?;
        
        let encode_time = encode_start.elapsed();
        #[cfg(debug_assertions)]
        println!("⚡ JPEG encode time: {:.1}ms (target: <100ms)", encode_time.as_secs_f64() * 1000.0);
        
        Ok(jpeg_data)
    }

    /// Fast RGB to WebP encoding
    fn encode_rgb_webp(data: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
        let mut jpeg_data = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg_data, 30)
            .encode(data, width, height, image::ExtendedColorType::Rgb8)
            .map_err(|e| format!("JPEG encode failed: {}", e))?;
        Ok(jpeg_data)
    }

    /// Enable high-performance DXGI capture mode
    pub async fn enable_dxgi_mode(&self) -> Result<(), String> {
        self.graphics_service.start_dxgi_capture().await
    }
}

#[async_trait::async_trait]
impl Service for MinimapService {
    async fn start(&self) -> Result<(), ()> {
        match self.start_capture().await {
            Ok(_) => Ok(()),
            Err(_) => Err(()),
        }
    }

    async fn stop(&self) -> Result<(), ()> {
        match self.stop_capture().await {
            Ok(_) => Ok(()),
            Err(_) => Err(()),
        }
    }
}
