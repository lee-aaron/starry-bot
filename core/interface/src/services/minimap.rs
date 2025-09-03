use std::sync::Arc;

use image::{imageops::FilterType, DynamicImage, ImageBuffer, RgbaImage};
use platforms::Window;
use tauri::Emitter;
use tokio::sync::{oneshot, Mutex};

use crate::services::Service;

#[derive(Clone)]
pub struct MinimapService {
    stop_tx: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    window_pairs_stop_tx: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    window: Arc<Mutex<Option<Window>>>,
    app_handle: tauri::AppHandle,
}

impl MinimapService {
    pub fn new(app_handle: tauri::AppHandle) -> Self {
        MinimapService {
            stop_tx: Arc::new(Mutex::new(None)),
            window_pairs_stop_tx: Arc::new(Mutex::new(None)),
            window: Arc::new(Mutex::new(Some(Window::new("UnityWndClass")))),
            app_handle,
        }
    }

    pub async fn set_window(&self, title: String) -> Result<(), ()> {
        let mut window_lock = self.window.lock().await;

        let windows = platforms::capture::query_capture_name_window_pairs().map_err(|_| ())?;
        if let Some((_, win)) = windows.into_iter().find(|(name, _)| *name == title) {
            log::info!("Setting capture window to: {:?}", win);
            *window_lock = Some(win);
            Ok(())
        } else {
            Err(())
        }
    }

    pub async fn capture_game_window(&self) -> Result<(), ()> {
        let window_lock = self.window.lock().await;
        let window = match window_lock.as_ref() {
            Some(w) => w,
            None => return Err(()),
        };
        // Debug: print which window handle is being used
        log::info!("Capturing from window: {:?}", window);

        let mut capture = platforms::capture::Capture::new(*window).map_err(|e| {
            log::info!("Failed to create capture: {:?}", e);
        })?;

        let frame = capture.grab().map_err(|e| {
            log::info!("Failed to capture frame from {:?}: {:?}", window, e);
        })?;

        let width = frame.width as u32;
        let height = frame.height as u32;
        let bgra = &frame.data;

        // Convert BGRA to RGBA
        let mut rgba = Vec::with_capacity(bgra.len());
        for chunk in bgra.chunks(4) {
            // BGRA -> RGBA
            rgba.push(chunk[2]); // R
            rgba.push(chunk[1]); // G
            rgba.push(chunk[0]); // B
            rgba.push(chunk[3]); // A
        }

        let img_buf: RgbaImage = ImageBuffer::from_raw(width, height, rgba).ok_or_else(|| {
            log::info!("Failed to create RgbaImage from raw BGRA data");
            ()
        })?;
        let img = DynamicImage::ImageRgba8(img_buf);
        let resized = img.resize_exact(1920, 1080, FilterType::Lanczos3);
        let resized_rgba = resized.to_rgba8();
        let buffer = resized_rgba.into_raw(); // Vec<u8> of RGBA

        let _ = self.app_handle.emit(
            "minimap-update",
            serde_json::json!({
                "buffer": buffer,
                "width": 1920,
                "height": 1080
            }),
        );
        Ok(())
    }

    pub fn list_windows(&self) -> Result<Vec<String>, ()> {
        let windows = platforms::capture::query_capture_name_window_pairs();
        match windows {
            Ok(pairs) => {
                let names: Vec<String> = pairs.iter().map(|(name, _)| name.clone()).collect();
                Ok(names)
            }
            Err(e) => {
                log::info!("Failed to list window pairs: {:?}", e);
                Err(())
            }
        }
    }

    pub async fn start_window_pairs_loop(&self) {
        let (tx, mut rx) = oneshot::channel();
        *self.window_pairs_stop_tx.lock().await = Some(tx);

        let this = self.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut rx => {
                        log::info!("Window pairs loop stopping...");
                        break;
                    }
                    _ = tokio::time::sleep(tokio::time::Duration::from_secs(10)) => {}
                }
                let _ = this.list_windows().map(|n| {
                    log::info!("Current windows: {:?}", n);
                });
            }
        });
    }

    pub async fn stop_window_pairs_loop(&self) {
        if let Some(tx) = self.window_pairs_stop_tx.lock().await.take() {
            let _ = tx.send(());
        }
    }
}

#[async_trait::async_trait]
impl Service for MinimapService {
    async fn start(&self) -> Result<(), ()> {
        let (tx, mut rx) = oneshot::channel();
        *self.stop_tx.lock().await = Some(tx);

        let this = self.clone();

        tauri::async_runtime::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut rx => {
                        log::info!("MiniMap Service stopping...");
                        break;
                    }
                    _ = tokio::time::sleep(tokio::time::Duration::from_millis(1500)) => {}
                }
                let _ = this.capture_game_window().await;
            }
        });

        #[cfg(all(debug_assertions, feature = "local"))]
        {
            self.start_window_pairs_loop().await;
        }
        Ok(())
    }

    async fn stop(&self) -> Result<(), ()> {
        if let Some(tx) = self.stop_tx.lock().await.take() {
            log::info!("Stopping MiniMap service...");
            let _ = tx.send(());
        }
        #[cfg(all(debug_assertions, feature = "local"))]
        {
            self.stop_window_pairs_loop().await;
        }
        Ok(())
    }
}
