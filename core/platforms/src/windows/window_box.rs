use std::{
    ffi::c_void,
    num::NonZeroU32,
    rc::Rc,
    sync::{Arc, Barrier, Mutex},
    thread::{self},
};

use softbuffer::{Context, Surface};
use tao::{
    dpi::{PhysicalPosition, PhysicalSize},
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder},
    platform::{
        run_return::EventLoopExtRunReturn,
        windows::{EventLoopBuilderExtWindows, WindowBuilderExtWindows},
    },
    rwh_06::{HasWindowHandle, RawWindowHandle},
    window::WindowBuilder,
};
use tokio::sync::oneshot::{self, Sender};
use windows::Win32::Foundation::HWND;

use super::{BitBltCapture, Handle};
use crate::{Result, capture::Frame, windows::HandleKind};

#[derive(Debug)]
pub struct WindowBoxCapture {
    handle: Handle,
    position: Arc<Mutex<Option<PhysicalPosition<i32>>>>,
    close_tx: Option<Sender<()>>,
    capture: BitBltCapture,
}

impl Default for WindowBoxCapture {
    fn default() -> Self {
        let handle = Arc::new(Mutex::new(None));
        let handle_clone = handle.clone();
        let barrier = Arc::new(Barrier::new(2));
        let barrier_clone = barrier.clone();
        let position = Arc::new(Mutex::new(None));
        let position_clone = position.clone();
        let (close_tx, mut close_rx) = oneshot::channel();

        thread::spawn(move || {
            let handle = handle_clone;
            let position = position_clone;
            let mut event_loop = EventLoopBuilder::new().with_any_thread(true).build();
            let window = WindowBuilder::new()
                .with_title("Capture Area")
                .with_decorations(true)
                .with_minimizable(false)
                .with_closable(false)
                .with_transparent(true)
                .with_resizable(true)
                .with_drag_and_drop(false)
                .with_min_inner_size(PhysicalSize::new(800, 600))
                .with_max_inner_size(PhysicalSize::new(1920, 1080))
                .build(&event_loop)
                .unwrap();
            let window = Rc::new(window);
            let context = Context::new(window.clone()).unwrap();
            let mut surface = Surface::new(&context, window.clone()).unwrap();
            let window = Some(window);

            *handle.lock().unwrap() =
                window
                    .as_ref()
                    .unwrap()
                    .window_handle()
                    .ok()
                    .map(|handle| match handle.as_raw() {
                        RawWindowHandle::Win32(handle) => handle.hwnd,
                        _ => unreachable!(),
                    });
            *position.lock().unwrap() = window.as_ref().unwrap().inner_position().ok();
            barrier_clone.wait();

            event_loop.run_return(|event, _, control_flow| {
                *control_flow = ControlFlow::Poll;
                if close_rx.try_recv().is_ok() {
                    *control_flow = ControlFlow::Exit;
                    return;
                }

                match event {
                    Event::WindowEvent {
                        window_id: _,
                        event: WindowEvent::Moved(updated),
                        ..
                    } => {
                        if let Some(ref window) = window {
                            *position.lock().unwrap() =
                                window.inner_position().ok().or(Some(updated));
                        }
                    }
                    Event::RedrawRequested(_) => {
                        if let Some(ref window) = window {
                            let size = window.inner_size();
                            let Some(width) = NonZeroU32::new(size.width) else {
                                return;
                            };
                            let Some(height) = NonZeroU32::new(size.height) else {
                                return;
                            };
                            surface.resize(width, height).unwrap();
                            let mut buffer = surface.buffer_mut().unwrap();
                            buffer.fill(0);
                            buffer.present().unwrap();
                        }
                    }
                    Event::MainEventsCleared => {
                        if let Some(ref window) = window {
                            window.request_redraw();
                        }
                    }
                    _ => (),
                }
            });
        });
        barrier.wait();
        let handle = HWND(handle.lock().unwrap().unwrap().get() as *mut c_void);
        let handle = Handle::new(HandleKind::Fixed(handle));
        let capture = BitBltCapture::new(handle, true);

        Self {
            handle,
            position,
            close_tx: Some(close_tx),
            capture,
        }
    }
}

impl WindowBoxCapture {
    pub fn handle(&self) -> Handle {
        self.handle
    }

    pub fn grab(&mut self) -> Result<Frame> {
        self.capture.grab_inner_offset(self.position())
    }

    #[inline]
    fn position(&self) -> Option<(i32, i32)> {
        self.position
            .lock()
            .unwrap()
            .map(|position| (position.x, position.y))
    }
}

impl Drop for WindowBoxCapture {
    fn drop(&mut self) {
        if let Some(tx) = self.close_tx.take() {
            tx.send(()).unwrap();
        }
    }
}
