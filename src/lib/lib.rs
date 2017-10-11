extern crate cgmath;
#[macro_use] extern crate conrod;
#[macro_use] extern crate custom_derive;
extern crate find_folder;
extern crate hound; // wav loading
extern crate image;
#[macro_use] extern crate newtype_derive;
extern crate rosc; // osc encoding/decoding
extern crate sample;
extern crate serde; // serialization
#[macro_use] extern crate serde_derive;
extern crate serde_json;
extern crate time_calc;
extern crate toml;

use conrod::backend::glium::glium;
use std::net::{Ipv4Addr, SocketAddrV4};

mod audio;
mod composer;
mod config;
mod gui;
mod interaction;
mod metres;
mod osc;

/// Run the Beyond Perception Audio Server.
pub fn run() {
    // Find the `assets` directory.
    let exe_path = std::env::current_exe().unwrap();
    let assets = find_folder::Search::ParentsThenKids(7, 3)
        .of(exe_path.parent().unwrap().into())
        .for_folder("assets")
        .unwrap();

    // Load the configuration struct.
    let config = config::load(&assets.join("config.toml")).unwrap();

    // Build the event loop and window.
    let mut events_loop = glium::glutin::EventsLoop::new();
    let window = glium::glutin::WindowBuilder::new()
        .with_title("Audio Server")
        .with_dimensions(config.window_width, config.window_height);
    let context = glium::glutin::ContextBuilder::new()
        .with_vsync(true)
        .with_multisampling(4);
    let display = glium::Display::new(window, context, &events_loop).unwrap();

    // Spawn the OSC input thread.
    let osc_input_addr = SocketAddrV4::new(Ipv4Addr::new(0, 0, 0, 0), config.osc_input_port);
    let (_osc_thread_handle, osc_msg_rx, interaction_gui_rx) = osc::input::spawn(osc_input_addr);

    // Spawn the audio engine (rendering, processing, etc).
    let (audio_thread_handle, audio_msg_tx) = audio::spawn();

    // Create the audio requester which transfers audio from the audio engine to the audio backend.
    const FRAMES_PER_BUFFER: usize = 64;
    let audio_requester = audio::Requester::new(audio_msg_tx.clone(), FRAMES_PER_BUFFER);

    // Run the CPAL audio backend for interfacing with the audio device.
    const SAMPLE_HZ: f64 = 44_100.0;
    let (_audio_backend_thread_handle, mut cpal_voice) =
        audio::backend::spawn(audio_requester, SAMPLE_HZ).unwrap();

    // Spawn the composer thread.
    let (composer_thread_handle, composer_msg_tx) = composer::spawn(audio_msg_tx.clone());

    // Spawn the GUI thread.
    //
    // The `gui_msg_tx` is a channel for sending input to the GUI thread.
    //
    // The renderer and image_map are used for rendering graphics primitives received on the
    // `gui_render_rx` channel.
    let proxy = events_loop.create_proxy();
    let (gui_thread_handle, mut renderer, image_map, gui_msg_tx, gui_render_rx) =
        gui::spawn(&assets,
                   config,
                   &display,
                   proxy,
                   osc_msg_rx,
                   interaction_gui_rx,
                   audio_msg_tx.clone(),
                   composer_msg_tx.clone());

    // Run the event loop.
    let mut closed = false;
    while !closed {

        // Draw the most recently received `conrod::render::Primitives` sent from the `Ui`.
        loop {
            match gui_render_rx.try_iter().last() {
                Some(primitives) => gui::draw(&display, &mut renderer, &image_map, &primitives),
                None => break,
            }
        }

        // Wait for the events or until we receive some graphics to draw from the GUI thread.
        events_loop.run_forever(|event| {
            // Use the `winit` backend feature to convert the winit event to a conrod one.
            if let Some(input) = conrod::backend::winit::convert_event(event.clone(), &display) {
                gui_msg_tx.send(gui::Message::Input(input)).unwrap();
            }

            match event {
                glium::glutin::Event::WindowEvent { event, .. } => match event {
                    // Break from the loop upon `Escape`.
                    glium::glutin::WindowEvent::Closed |
                    glium::glutin::WindowEvent::KeyboardInput {
                        input: glium::glutin::KeyboardInput {
                            virtual_keycode: Some(glium::glutin::VirtualKeyCode::Escape),
                            ..
                        },
                        ..
                    } => {
                        closed = true;
                        audio_msg_tx.send(audio::Message::Exit).unwrap();
                        gui_msg_tx.send(gui::Message::Exit).unwrap();
                        composer_msg_tx.send(composer::Message::Exit).unwrap();
                        return glium::glutin::ControlFlow::Break;
                    },
                    // We must re-draw on `Resized`, as the event loops become blocked during
                    // resize on macOS.
                    glium::glutin::WindowEvent::Resized(..) => {
                        if let Some(primitives) = gui_render_rx.iter().next() {
                            gui::draw(&display, &mut renderer, &image_map, &primitives);
                        }
                    },
                    _ => {},
                },
                glium::glutin::Event::Awakened => return glium::glutin::ControlFlow::Break,
                _ => (),
            }

            glium::glutin::ControlFlow::Continue
        });
    }

    // Stop the CPAL stream and join the thread.
    cpal_voice.pause();

    // Wait for the audio thread to finish.
    audio_thread_handle.join().unwrap();

    // Wait for the composer thread to finish.
    composer_thread_handle.join().unwrap();

    // Wait for the GUI thread to finish saving files, etc.
    gui_thread_handle.join().unwrap();
}
