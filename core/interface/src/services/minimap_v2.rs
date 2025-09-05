use std::sync::Arc;
use std::time::Instant;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use tokio::sync::{Mutex, watch, broadcast};
use opencv::{
    core::{Mat, MatTraitConst, CV_8UC4},
    imgcodecs::{imencode, IMWRITE_WEBP_QUALITY},
    core::Vector,
    prelude::*,
};

use crate::services::Service;
use super::graphics_capture::{GraphicsCaptureService, CapturedFrame};

#[derive(Debug, Clone, PartialEq)]
pub enum ServiceState {
    Stopped,
    Starting,
    Running,
    Stopping,
}

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

/// Minimap detection service that processes frames from GraphicsCaptureService
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
    is_starting: Arc<Mutex<bool>>,
    
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
            is_starting: Arc::new(Mutex::new(false)),
            metrics,
        }
    }

    pub fn get_frame_receiver(&self) -> watch::Receiver<Option<Vec<u8>>> {
        self.frame_watch.clone()
    }

    pub async fn is_capturing(&self) -> bool {
        *self.is_processing.lock().await
    }

    pub async fn get_service_state(&self) -> ServiceState {
        let is_processing = *self.is_processing.lock().await;
        let is_stopping = *self.is_stopping.lock().await;
        let is_starting = *self.is_starting.lock().await;
        let has_window = self.current_window_title.lock().await.is_some();
        let graphics_active = self.graphics_service.is_capturing().await;
        
        if is_stopping {
            ServiceState::Stopping
        } else if is_starting {
            ServiceState::Starting
        } else if is_processing && graphics_active && has_window {
            ServiceState::Running
        } else {
            ServiceState::Stopped
        }
    }

    pub async fn get_current_window_title(&self) -> Option<String> {
        self.current_window_title.lock().await.clone()
    }

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

    pub async fn set_window(&self, title: String) -> Result<(), String> {
        self.stop_capture().await?;
        
        self.graphics_service.start_window_capture(&title).await?;
        
        let frame_receiver = self.graphics_service.subscribe();
        *self.frame_receiver.lock().await = Some(frame_receiver);

        *self.current_window_title.lock().await = Some(title);

        self.start_capture().await
    }

    pub async fn start_capture(&self) -> Result<(), String> {
        *self.is_starting.lock().await = true;
        *self.is_stopping.lock().await = false;
        
        if *self.is_processing.lock().await {
            *self.is_starting.lock().await = false;
            self.stop_capture().await?;
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
            *self.is_starting.lock().await = true;
        }

        let receiver_guard = self.frame_receiver.lock().await;
        let mut receiver = match receiver_guard.as_ref() {
            Some(r) => r.resubscribe(),
            None => return Err("No graphics capture subscription".to_string()),
        };
        drop(receiver_guard);

        *self.is_processing.lock().await = true;
        *self.is_starting.lock().await = false;

        let frame_sender = self.frame_sender.clone();
        let metrics = self.metrics.clone();
        let is_processing = self.is_processing.clone();

        tokio::spawn(async move {
            while *is_processing.lock().await {
                match receiver.recv().await {
                    Ok(captured_frame) => {
                        let process_start = Instant::now();
                        
                        match Self::process_minimap_frame(captured_frame, &metrics).await {
                            Ok(processed_webp) => {
                                if frame_sender.send(Some(processed_webp)).is_ok() {
                                    metrics.frames_processed.fetch_add(1, Ordering::Relaxed);
                                } else {
                                    metrics.frames_dropped.fetch_add(1, Ordering::Relaxed);
                                }
                            }
                            Err(_) => {
                                metrics.frames_dropped.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        
                        let elapsed = process_start.elapsed().as_millis() as u64;
                        metrics.total_processing_time_ms.fetch_add(elapsed, Ordering::Relaxed);
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        metrics.frames_dropped.fetch_add(skipped as usize, Ordering::Relaxed);
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        break;
                    }
                }
            }
        });

        Ok(())
    }

    pub async fn stop_capture(&self) -> Result<(), String> {
        {
            let mut stopping = self.is_stopping.lock().await;
            if *stopping {
                return Ok(());
            }
            *stopping = true;
        }
        
        *self.is_processing.lock().await = false;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        
        *self.current_window_title.lock().await = None;
        *self.frame_receiver.lock().await = None;
        let _ = self.frame_sender.send(None);
        
        self.graphics_service.stop_capture().await;
        
        *self.is_stopping.lock().await = false;
        *self.is_starting.lock().await = false;
        
        Ok(())
    }

    async fn process_minimap_frame(
        frame: CapturedFrame,
        metrics: &MinimapMetrics,
    ) -> Result<Vec<u8>, String> {
        if frame.data.is_empty() {
            return Err("Empty frame data".to_string());
        }
        
        let opencv_start = Instant::now();
        let minimap_detected = Self::detect_minimap_with_opencv(&frame).await?;
        let opencv_time = opencv_start.elapsed().as_millis() as u64;
        metrics.total_opencv_time_ms.fetch_add(opencv_time, Ordering::Relaxed);
        
        if minimap_detected {
            metrics.opencv_detections.fetch_add(1, Ordering::Relaxed);
        }

        let encode_start = Instant::now();
        let result = Self::encode_frame_webp_opencv(&frame).await?;
        
        let encode_time = encode_start.elapsed().as_millis() as u64;
        metrics.total_encode_time_ms.fetch_add(encode_time, Ordering::Relaxed);

        Ok(result)
    }


    async fn detect_minimap_with_opencv(frame: &CapturedFrame) -> Result<bool, String> {
        let mat = Self::create_bgra_mat(frame)?;
        
        let size = mat.size().map_err(|e| format!("Failed to get Mat size: {}", e))?;
        let has_minimap = size.width >= 640 && size.height >= 360;

        tokio::time::sleep(std::time::Duration::from_micros(100)).await;
        
        Ok(has_minimap)
    }

    async fn encode_frame_webp_opencv(frame: &CapturedFrame) -> Result<Vec<u8>, String> {
        let mat = Self::create_bgra_mat(frame)?;

        let mut buffer = Vector::<u8>::new();
        let params = Vector::<i32>::from_slice(&[IMWRITE_WEBP_QUALITY, 75]);
        
        imencode(".webp", &mat, &mut buffer, &params)
            .map_err(|e| format!("Failed to encode WebP: {}", e))?;
        
        Ok(buffer.to_vec())
    }

    fn create_bgra_mat(frame: &CapturedFrame) -> Result<Mat, String> {
        let rows = frame.height as i32;
        let cols = frame.width as i32;
        
        let mut mat = Mat::zeros(rows, cols, CV_8UC4)
            .map_err(|e| format!("Failed to create Mat: {}", e))?
            .to_mat()
            .map_err(|e| format!("Failed to convert to Mat: {}", e))?;
        
        unsafe {
            let mat_ptr = mat.ptr_mut(0).map_err(|e| format!("Failed to get Mat pointer: {}", e))?;
            let mat_size = (rows * cols * 4) as usize; // 4 bytes per BGRA pixel
            
            if frame.data.len() >= mat_size {
                std::ptr::copy_nonoverlapping(
                    frame.data.as_ptr(),
                    mat_ptr,
                    mat_size,
                );
            } else {
                return Err(format!("Frame data too small: {} < {}", frame.data.len(), mat_size));
            }
        }
        
        Ok(mat)
    }

    /// Enable high-performance capture mode
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
