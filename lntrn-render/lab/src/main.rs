use anyhow::Result;
use lntrn_lab::{LabCommand, RendererLab};
use lntrn_render::{GpuContext, Painter, SurfaceError, TextRenderer};
use winit::{
    application::ApplicationHandler,
    dpi::PhysicalSize,
    event::{ElementState, MouseButton, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{Window, WindowAttributes, WindowId},
};

fn main() -> Result<()> {
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Wait);

    let mut app = LabApp::default();
    event_loop.run_app(&mut app)?;
    Ok(())
}

#[derive(Default)]
struct LabApp {
    state: Option<LabState>,
}

struct LabState {
    window: Window,
    window_id: WindowId,
    gpu: GpuContext,
    painter: Painter,
    text: TextRenderer,
    lab: RendererLab,
}

impl LabState {
    fn new(window: Window) -> Self {
        let size = window.inner_size();
        let gpu = GpuContext::from_window(&window, size.width.max(1), size.height.max(1))
            .expect("failed to create GPU context");
        let painter = Painter::new(&gpu);
        let text = TextRenderer::new(&gpu);
        let window_id = window.id();

        Self {
            window,
            window_id,
            gpu,
            painter,
            text,
            lab: RendererLab::new(),
        }
    }

    fn render(&mut self) -> Result<(), SurfaceError> {
        let size = self.window.inner_size();
        self.lab
            .render((size.width, size.height), &self.gpu, &mut self.painter, &mut self.text)
    }
}

impl LabApp {
    fn shutdown(&mut self, event_loop: &ActiveEventLoop) {
        self.state = None;
        event_loop.exit();
    }
}

impl ApplicationHandler for LabApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }

        let attrs = WindowAttributes::default()
            .with_title("Lantern Lab")
            .with_inner_size(PhysicalSize::new(1280, 800))
            .with_min_inner_size(PhysicalSize::new(640, 480))
            .with_resizable(true)
            .with_transparent(true);

        let window = event_loop
            .create_window(attrs)
            .expect("failed to create lab window");

        let state = LabState::new(window);
        state.window.request_redraw();
        self.state = Some(state);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(state) = self.state.as_mut() else {
            return;
        };
        if state.window_id != window_id {
            return;
        }

        match event {
            WindowEvent::CloseRequested => self.shutdown(event_loop),
            WindowEvent::Resized(size) => {
                state.gpu.resize(size.width.max(1), size.height.max(1));
                state.window.request_redraw();
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                let size = state.window.inner_size();
                state.gpu.resize(size.width.max(1), size.height.max(1));
                state.window.request_redraw();
            }
            WindowEvent::CursorMoved { position, .. } => {
                let size = state.window.inner_size();
                state.lab.on_cursor_moved((size.width, size.height), position.x as f32, position.y as f32);
                state.window.request_redraw();
            }
            WindowEvent::CursorLeft { .. } => {
                state.lab.on_cursor_left();
                state.window.request_redraw();
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let dy = match delta {
                    winit::event::MouseScrollDelta::LineDelta(_, y) => y,
                    winit::event::MouseScrollDelta::PixelDelta(pos) => pos.y as f32 / 40.0,
                };
                state.lab.on_scroll(dy);
                state.window.request_redraw();
            }
            WindowEvent::MouseInput {
                state: button_state,
                button: MouseButton::Left,
                ..
            } => {
                match button_state {
                    ElementState::Pressed => {
                        let size = state.window.inner_size();
                        if state.lab.on_left_pressed((size.width, size.height)) {
                            self.shutdown(event_loop);
                            return;
                        }
                    }
                    ElementState::Released => state.lab.on_left_released(),
                }
                state.window.request_redraw();
            }
            WindowEvent::MouseInput {
                state: button_state,
                button: MouseButton::Right,
                ..
            } => {
                if button_state == ElementState::Pressed {
                    let size = state.window.inner_size();
                    state.lab.on_right_pressed((size.width, size.height));
                    state.window.request_redraw();
                }
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed && !event.repeat => {
                match event.physical_key {
                    PhysicalKey::Code(KeyCode::Escape) => {
                        self.shutdown(event_loop);
                        return;
                    }
                    PhysicalKey::Code(KeyCode::KeyR) => state.lab.on_command(LabCommand::ResetOrb),
                    _ => {}
                }
                state.window.request_redraw();
            }
            WindowEvent::RedrawRequested => match state.render() {
                Ok(()) => {}
                Err(SurfaceError::Outdated | SurfaceError::Lost) => {
                    let size = state.window.inner_size();
                    state.gpu.resize(size.width.max(1), size.height.max(1));
                    state.window.request_redraw();
                }
                Err(SurfaceError::OutOfMemory) => event_loop.exit(),
                Err(SurfaceError::Timeout | SurfaceError::Other) => {
                    state.window.request_redraw();
                }
            },
            _ => {}
        }
    }


}
