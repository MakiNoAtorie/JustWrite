#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use justwrite::ui::app::{App, UserEvent};
use std::time::Instant;
use winit::event_loop::{ControlFlow, EventLoop};

fn main() {
    let started = Instant::now();
    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .build()
        .expect("Cannot create native event loop");
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = App::new(event_loop.create_proxy(), started);
    if let Err(error) = event_loop.run_app(&mut app) {
        eprintln!("{error}");
    }
}
