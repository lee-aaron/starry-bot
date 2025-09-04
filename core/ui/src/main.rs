use iced::widget::{button, column, container, pick_list, text, image, row};
use iced::{Element, Fill, Length, Task, Theme, Subscription};
use interface::{list_window_handles, services::{GraphicsCaptureService, MinimapServiceV2}};
use std::sync::Arc;
use tokio_stream::{wrappers::WatchStream, StreamExt};

/// Convert JPEG bytes to an iced image handle
fn jpeg_bytes_to_image_handle(jpeg_bytes: &[u8]) -> image::Handle {
    image::Handle::from_bytes(jpeg_bytes.to_vec())
}

fn main() -> iced::Result {
    iced::application("Starry Bot", StarryApp::update, StarryApp::view)
        .subscription(StarryApp::subscription)
        .theme(|_| Theme::Dark)
        .run_with(|| (StarryApp::default(), Task::perform(async { 
            list_window_handles() 
        }, Message::WindowsRefreshed)))
}

#[derive(Debug, Clone)]
pub enum Message {
    WindowSelected(String),
    RefreshWindows,
    StartCapture,
    StopCapture,
    WindowsRefreshed(Vec<String>),
    CaptureStarted,
    CaptureStopped,
    CaptureError(String),
    FrameReceived(Option<Vec<u8>>),
    CheckServiceStatus,
    ServiceStatusChecked(bool, Option<String>),
    ShowMetrics,
    MetricsReceived(Option<String>),
    DxgiModeResult(Result<(), String>),
}

pub struct StarryApp {
    graphics_service: Arc<GraphicsCaptureService>,
    minimap_service: MinimapServiceV2,
    available_windows: Vec<String>,
    selected_window: Option<String>,
    service_running: bool,
    service_stopping: bool,
    current_frame: Option<image::Handle>,
    error_message: Option<String>,
}

impl Default for StarryApp {
    fn default() -> Self {
        let graphics_service = Arc::new(GraphicsCaptureService::new());
        let minimap_service = MinimapServiceV2::new(graphics_service.clone());
        
        Self {
            graphics_service,
            minimap_service,
            available_windows: Vec::new(),
            selected_window: None,
            service_running: false,
            service_stopping: false,
            current_frame: None,
            error_message: None,
        }
    }
}

impl StarryApp {
    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::RefreshWindows => {
                Task::perform(
                    async {
                        list_window_handles()
                    },
                    Message::WindowsRefreshed,
                )
            },
            Message::WindowsRefreshed(windows) => {
                self.available_windows = windows;
                
                // Try to automatically select a Unity window (or any predefined window)
                let predefined_windows = ["BPSR"];
                for predefined in &predefined_windows {
                    if let Some(window) = self.available_windows.iter()
                        .find(|w| w.to_lowercase().contains(&predefined.to_lowercase())) {
                        println!("🎯 Auto-selecting window: {}", window);
                        self.selected_window = Some(window.clone());
                        self.error_message = None;
                        let service = self.minimap_service.clone();
                        let window_title = window.clone();
                        return Task::perform(
                            async move {
                                match service.set_window(window_title).await {
                                    Ok(_) => Message::CaptureStarted,
                                    Err(e) => Message::CaptureError(e),
                                }
                            },
                            |result| result,
                        );
                    }
                }
                println!("❌ No matching window found for: {:?}", predefined_windows);
                Task::none()
            },
            Message::WindowSelected(window) => {
                self.selected_window = Some(window.clone());
                self.error_message = None; // Clear any previous errors
                let service = self.minimap_service.clone();
                Task::perform(
                    async move {
                        match service.set_window(window).await {
                            Ok(_) => Message::CaptureStarted,
                            Err(e) => Message::CaptureError(e),
                        }
                    },
                    |result| result,
                )
            },
            Message::StartCapture => {
                if let Some(window_title) = &self.selected_window {
                    self.error_message = None; // Clear any previous errors
                    let service = self.minimap_service.clone();
                    let window_title = window_title.clone();
                    Task::perform(
                        async move {
                            match service.set_window(window_title).await {
                                Ok(_) => Message::CaptureStarted,
                                Err(e) => Message::CaptureError(e),
                            }
                        },
                        |result| result,
                    )
                } else {
                    self.error_message = Some("No window selected".to_string());
                    Task::none()
                }
            },
            Message::StopCapture => {
                if self.service_stopping {
                    // Already stopping, ignore additional stop requests
                    return Task::none();
                }
                
                self.service_stopping = true;
                let service = self.minimap_service.clone();
                Task::perform(
                    async move {
                        match service.stop_capture().await {
                            Ok(_) => Message::CaptureStopped,
                            Err(e) => Message::CaptureError(e),
                        }
                    },
                    |result| result,
                )
            },
            Message::CaptureStarted => {
                self.service_running = true;
                self.error_message = None;
                
                println!("✅ Capture started successfully!");
                
                // Automatically enable high-performance DXGI mode
                let service = self.minimap_service.clone();
                let service2 = self.minimap_service.clone();
                Task::batch([
                    // Enable DXGI mode for high performance
                    Task::perform(
                        async move {
                            match service.enable_dxgi_mode().await {
                                Ok(_) => {
                                    println!("🚀 High-performance DXGI mode enabled automatically");
                                    Ok(())
                                }
                                Err(e) => {
                                    println!("⚠️  DXGI mode failed, using standard capture: {}", e);
                                    Err(e)
                                }
                            }
                        },
                        Message::DxgiModeResult,
                    ),
                    // Check service status
                    Task::perform(
                        async move {
                            let is_capturing = service2.is_capturing().await;
                            let current_window = service2.get_current_window_title().await;
                            (is_capturing, current_window)
                        },
                        |(is_capturing, current_window)| Message::ServiceStatusChecked(is_capturing, current_window),
                    ),
                ])
            },
            Message::CaptureStopped => {
                self.service_running = false;
                self.service_stopping = false;
                self.current_frame = None;
                self.error_message = None;
                Task::none()
            },
            Message::CaptureError(error) => {
                self.service_running = false;
                self.service_stopping = false;
                self.current_frame = None;
                self.error_message = Some(error);
                Task::none()
            },
            Message::CheckServiceStatus => {
                let service = self.minimap_service.clone();
                Task::perform(
                    async move {
                        let is_capturing = service.is_capturing().await;
                        let current_window = service.get_current_window_title().await;
                        (is_capturing, current_window)
                    },
                    |(is_capturing, current_window)| Message::ServiceStatusChecked(is_capturing, current_window),
                )
            },
            Message::ServiceStatusChecked(is_capturing, current_window) => {
                // Synchronize UI state with actual service state
                self.service_running = is_capturing;
                if let Some(window_title) = current_window {
                    if !self.available_windows.contains(&window_title) {
                        // Window might have been closed, refresh the list
                        return Task::perform(async { list_window_handles() }, Message::WindowsRefreshed);
                    }
                    self.selected_window = Some(window_title);
                } else if !is_capturing {
                    // Service stopped but UI doesn't know why
                    self.current_frame = None;
                }
                Task::none()
            },
            Message::FrameReceived(frame_data) => {
                if let Some(jpeg_bytes) = frame_data {
                    self.current_frame = Some(jpeg_bytes_to_image_handle(&jpeg_bytes));
                } else {
                    self.current_frame = None;
                }
                Task::none()
            },
            Message::ShowMetrics => {
                let service = self.minimap_service.clone();
                Task::perform(
                    async move {
                        service.get_performance_metrics()
                    },
                    Message::MetricsReceived,
                )
            },
            Message::MetricsReceived(metrics) => {
                if let Some(metrics_text) = metrics {
                    println!("\n{}", metrics_text);
                    
                    // Also show graphics service metrics separately
                    let graphics_metrics = self.graphics_service.get_metrics();
                    println!("\n📊 Graphics Service Only:\n{}", graphics_metrics);
                }
                Task::none()
            },
            Message::DxgiModeResult(result) => {
                match result {
                    Ok(_) => {
                        println!("✅ DXGI high-performance mode enabled!");
                        self.error_message = None;
                    },
                    Err(e) => {
                        println!("❌ Failed to enable DXGI mode: {}", e);
                        self.error_message = Some(format!("DXGI mode failed: {}", e));
                    }
                }
                Task::none()
            },
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        let frame_subscription = if self.service_running {
            // Create a subscription that listens to frame updates using WatchStream
            let receiver = self.minimap_service.get_frame_receiver();
            
            Subscription::run_with_id(
                "frame_receiver",
                WatchStream::new(receiver).map(Message::FrameReceived)
            )
        } else {
            Subscription::none()
        };

        let status_check_subscription = iced::time::every(std::time::Duration::from_secs(2))
            .map(|_| Message::CheckServiceStatus);

        Subscription::batch([frame_subscription, status_check_subscription])
    }

    fn view(&self) -> Element<'_, Message> {
        // Left column: Minimap display
        let minimap_display = if let Some(frame_handle) = &self.current_frame {
            column![
                text("Current Minimap:").size(16),
                image(frame_handle.clone())
                    .width(Length::Fixed(400.0))
                    .height(Length::Fixed(225.0))
            ]
            .spacing(10)
        } else {
            column![
                container(text("Waiting for capture..."))
                    .width(Length::Fixed(400.0))
                    .height(Length::Fixed(225.0))
                    .style(|_theme: &iced::Theme| {
                        iced::widget::container::Style {
                            background: Some(iced::Background::Color(iced::Color::from_rgba(0.1, 0.1, 0.1, 0.8))),
                            border: iced::Border {
                                color: iced::Color::from_rgba(0.3, 0.3, 0.3, 0.8),
                                width: 1.0,
                                radius: 5.0.into(),
                            },
                            ..Default::default()
                        }
                    })
                    .center_x(Fill)
                    .center_y(Fill)
            ]
            .spacing(10)
        };

        // Right column: Controls and information
        let window_picker = column![
            text("Select Window:").size(16),
            pick_list(
                &self.available_windows[..],
                self.selected_window.as_ref(),
                Message::WindowSelected,
            )
            .placeholder("Select a window to capture..."),
            button("Refresh Windows")
                .on_press(Message::RefreshWindows)
                .width(Length::Fill),
        ]
        .spacing(10);

        let capture_controls = if self.service_running && !self.service_stopping {
            column![
                button("Stop Capture")
                    .on_press(Message::StopCapture)
                    .width(Length::Fill),
                button("Show Performance Metrics")
                    .on_press(Message::ShowMetrics)
                    .width(Length::Fill)
            ].spacing(5)
        } else if self.service_stopping {
            column![
                button("Stopping...")
                    .width(Length::Fill), // Disabled button while stopping
                button("Show Performance Metrics")
                    .on_press(Message::ShowMetrics)
                    .width(Length::Fill)
            ].spacing(5)
        } else {
            column![
                button("Start Capture")
                    .on_press_maybe(self.selected_window.as_ref().map(|_| Message::StartCapture))
                    .width(Length::Fill)
            ]
        };

        let status_text = if self.service_stopping {
            "Stopping minimap capture...".to_string()
        } else if self.service_running {
            if let Some(window) = &self.selected_window {
                format!("Minimap capture is running ({})", window)
            } else {
                "Minimap capture is running".to_string()
            }
        } else {
            "Minimap capture is stopped".to_string()
        };

        let error_display = if let Some(error) = &self.error_message {
            Some(column![
                text("Error:").size(16),
                text(error.clone()).size(14)
            ]
            .spacing(5))
        } else {
            None
        };

        let mut right_column_elements = vec![
            window_picker.into(),
            capture_controls.into(),
            text(status_text).size(14).into(),
        ];

        if let Some(error_widget) = error_display {
            right_column_elements.push(error_widget.into());
        }

        let right_column = column(right_column_elements)
            .spacing(20)
            .width(Length::Fixed(300.0));

        // Main two-column layout
        let main_content = row![
            container(minimap_display)
                .width(Length::Fixed(420.0))
                .padding(10),
            container(right_column)
                .width(Length::Fixed(320.0))
                .padding(10)
        ]
        .spacing(20);

        // Debug panel - only show in debug builds as a separate right panel
        #[cfg(debug_assertions)]
        {
            let debug_panel = container(
                column![
                    text("🐛 Debug Panel").size(16).color([0.8, 0.4, 0.4]),
                    text(format!("Build: Debug")).size(12).color([0.6, 0.6, 0.6]),
                    text(format!("Selected Window: {:?}", self.selected_window)).size(12).color([0.6, 0.6, 0.6]),
                    text(format!("Service Running: {}", self.service_running)).size(12).color([0.6, 0.6, 0.6]),
                    text(format!("Error Message: {:?}", self.error_message)).size(12).color([0.6, 0.6, 0.6]),
                    text(format!("Available Windows: {}", self.available_windows.len())).size(12).color([0.6, 0.6, 0.6]),
                ]
                .spacing(5)
                .padding(10)
            )
            .style(|_theme: &iced::Theme| {
                iced::widget::container::Style {
                    background: Some(iced::Background::Color(iced::Color::from_rgba(0.2, 0.2, 0.2, 0.8))),
                    border: iced::Border {
                        color: iced::Color::from_rgba(0.8, 0.4, 0.4, 0.6),
                        width: 1.0,
                        radius: 5.0.into(),
                    },
                    ..Default::default()
                }
            })
            .width(Length::Fixed(300.0))
            .height(Length::Fill);
            
            let content_with_debug = row![
                main_content,
                container(debug_panel)
                    .padding(10)
            ]
            .spacing(10);
            
            container(
                column![
                    text("Starry Bot Minimap").size(24),
                    content_with_debug
                ]
                .spacing(20)
                .padding(20)
            )
            .width(Fill)
            .height(Fill)
            .center_x(Fill)
            .center_y(Fill)
            .into()
        }
        
        #[cfg(not(debug_assertions))]
        {
            container(
                column![
                    text("Starry Bot Minimap").size(24),
                    main_content
                ]
                .spacing(20)
                .padding(20)
            )
            .width(Fill)
            .height(Fill)
            .center_x(Fill)
            .center_y(Fill)
            .into()
        }
    }
}
