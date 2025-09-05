use iced::widget::{button, column, container, pick_list, text, image, row};
use iced::{Element, Fill, Length, Task, Theme, Subscription};
use interface::{list_window_handles, services::{GraphicsCaptureService, MinimapServiceV2, ServiceState}};
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
    ServiceStatusChecked(ServiceState),
    ShowMetrics,
    MetricsReceived(Option<String>),
    UpdateMetrics,
    DxgiModeResult(Result<(), String>),
}

pub struct StarryApp {
    graphics_service: Arc<GraphicsCaptureService>,
    minimap_service: MinimapServiceV2,
    available_windows: Vec<String>,
    selected_window: Option<String>,
    service_state: ServiceState,
    current_frame: Option<image::Handle>,
    error_message: Option<String>,
    metrics_text: Option<String>,
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
            service_state: ServiceState::Stopped,
            current_frame: None,
            error_message: None,
            metrics_text: None,
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
                    self.service_state = ServiceState::Starting;
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
                // Only stop if not already stopping
                if self.service_state != ServiceState::Stopping {
                    self.service_state = ServiceState::Stopping;
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
                } else {
                    Task::none()
                }
            },
            Message::CaptureStarted => {
                self.service_state = ServiceState::Running;
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
                            service2.get_service_state().await
                        },
                        Message::ServiceStatusChecked,
                    ),
                ])
            },
            Message::CaptureStopped => {
                self.service_state = ServiceState::Stopped;
                self.current_frame = None;
                self.error_message = None;
                Task::none()
            },
            Message::CaptureError(error) => {
                self.service_state = ServiceState::Stopped;
                self.current_frame = None;
                self.error_message = Some(error);
                Task::none()
            },
            Message::CheckServiceStatus => {
                let service = self.minimap_service.clone();
                Task::perform(
                    async move {
                        service.get_service_state().await
                    },
                    Message::ServiceStatusChecked,
                )
            },
            Message::ServiceStatusChecked(service_state) => {
                // Synchronize UI state with actual service state
                self.service_state = service_state;
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
                    // Store metrics for display in debug panel instead of printing to console
                    let graphics_metrics = self.graphics_service.get_metrics();
                    let combined_metrics = format!("{}\n\n📊 Graphics Service:\n{}", metrics_text, graphics_metrics);
                    self.metrics_text = Some(combined_metrics);
                }
                Task::none()
            },
            Message::UpdateMetrics => {
                // Auto-update metrics every 3-5 seconds
                if self.service_state == ServiceState::Running {
                    let service = self.minimap_service.clone();
                    Task::perform(
                        async move {
                            service.get_performance_metrics()
                        },
                        Message::MetricsReceived,
                    )
                } else {
                    Task::none()
                }
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
        let frame_subscription = if self.service_state == ServiceState::Running {
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

        // Auto-update metrics every 4 seconds when running
        let metrics_update_subscription = if self.service_state == ServiceState::Running {
            iced::time::every(std::time::Duration::from_secs(4))
                .map(|_| Message::UpdateMetrics)
        } else {
            Subscription::none()
        };

        Subscription::batch([frame_subscription, status_check_subscription, metrics_update_subscription])
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

        let capture_controls = match self.service_state {
            ServiceState::Running => {
                column![
                    button("Stop Capture")
                        .on_press(Message::StopCapture)
                        .width(Length::Fill),
                    button("Show Performance Metrics")
                        .on_press(Message::ShowMetrics)
                        .width(Length::Fill)
                ].spacing(5)
            },
            ServiceState::Stopping => {
                column![
                    button("Stopping...")
                        .width(Length::Fill), // Disabled button while stopping
                    button("Show Performance Metrics")
                        .on_press(Message::ShowMetrics)
                        .width(Length::Fill)
                ].spacing(5)
            },
            ServiceState::Starting => {
                column![
                    button("Starting...")
                        .width(Length::Fill), // Disabled button while starting
                ].spacing(5)
            },
            ServiceState::Stopped => {
                column![
                    button("Start Capture")
                        .on_press_maybe(self.selected_window.as_ref().map(|_| Message::StartCapture))
                        .width(Length::Fill)
                ]
            }
        };

        let status_text = match self.service_state {
            ServiceState::Stopping => "Stopping minimap capture...".to_string(),
            ServiceState::Starting => "Starting minimap capture...".to_string(),
            ServiceState::Running => {
                if let Some(window) = &self.selected_window {
                    format!("Minimap capture is running ({})", window)
                } else {
                    "Minimap capture is running".to_string()
                }
            },
            ServiceState::Stopped => "Minimap capture is stopped".to_string(),
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
            let metrics_display = self.metrics_text.as_ref()
                .map(|s| s.as_str())
                .unwrap_or("Click 'Show Performance Metrics' to see data");
                
            let debug_panel = container(
                column![
                    text("🐛 Debug Panel").size(16).color([0.8, 0.4, 0.4]),
                    text(format!("Build: Debug")).size(12).color([0.6, 0.6, 0.6]),
                    text(format!("Selected Window: {:?}", self.selected_window)).size(12).color([0.6, 0.6, 0.6]),
                    text(format!("Service State: {:?}", self.service_state)).size(12).color([0.6, 0.6, 0.6]),
                    text(format!("Error Message: {:?}", self.error_message)).size(12).color([0.6, 0.6, 0.6]),
                    text(format!("Available Windows: {}", self.available_windows.len())).size(12).color([0.6, 0.6, 0.6]),
                    text("").size(8), // Spacer
                    text("📊 Performance Metrics:").size(14).color([0.4, 0.8, 0.4]),
                    text(metrics_display)
                        .size(10)
                        .color([0.8, 0.8, 0.8])
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
