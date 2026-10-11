#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use justwrite::ui::app::{App, UserEvent};
use std::time::Instant;
use winit::event_loop::{ControlFlow, EventLoop};

fn main() {
    let started = Instant::now();
    let mut builder = EventLoop::<UserEvent>::with_user_event();
    // The stock macOS menu quits on Cmd+Q without asking about unsaved text and takes Cmd+H.
    #[cfg(target_os = "macos")]
    winit::platform::macos::EventLoopBuilderExtMacOS::with_default_menu(&mut builder, false);
    let event_loop = builder
        .build()
        .expect("Cannot create native event loop");
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = App::new(event_loop.create_proxy(), started);
    if let Err(error) = event_loop.run_app(&mut app) {
        eprintln!("{error}");
    }
}
