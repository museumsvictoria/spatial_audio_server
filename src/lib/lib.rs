// Extend the macro recursion limit to allow for many widget IDs.
#![recursion_limit="128"]

#[macro_use] extern crate conrod;
#[macro_use] extern crate conrod_derive;
#[macro_use] extern crate custom_derive;
extern crate hound; // wav loading
#[macro_use] extern crate newtype_derive;
extern crate nannou;
extern crate serde; // serialization
#[macro_use] extern crate serde_derive;
extern crate serde_json;
extern crate time_calc;
extern crate toml;

use nannou::prelude::*;
use std::sync::mpsc;
use std::thread;

mod audio;
mod config;
mod gui;
mod installation;
mod interaction;
mod metres;
mod osc;
mod soundscape;

pub fn run() {
    nannou::app(model, update, draw).exit(exit).run();
}

/// The model of the application state.
///
/// This is the state stored and updated on the main thread.
struct Model {
    gui: gui::Model,
    composer_msg_tx: mpsc::Sender<soundscape::Message>,
    composer_thread_handle: thread::JoinHandle<()>,
}

// Initialise the state of the application.
fn model(app: &App) -> Model {
    // Don't keep looping, just wait for events.
    app.set_loop_mode(LoopMode::wait(3));

    // Find the assets directory.
    let assets = app.assets_path().unwrap();

    // Load the configuration struct.
    let config = config::load(&assets.join("config.toml")).unwrap();

    // Create a window.
    let window = app.new_window()
        .with_title("Audio Server")
        .with_dimensions(config.window_width, config.window_height)
        .with_vsync(true)
        .with_multisampling(4)
        .build()
        .unwrap();

    // Spawn the OSC input thread.
    let osc_receiver = nannou::osc::receiver(config.osc_input_port).unwrap();
    let (_osc_in_thread_handle, osc_in_log_rx, interaction_rx) = osc::input::spawn(osc_receiver);

    // Spawn the OSC output thread.
    let (_osc_out_thread_handle, osc_out_msg_tx, osc_out_log_rx) = osc::output::spawn();

    // Get the default device and attempt to set it up with the target number of channels.
    let device = app.audio.default_output_device().unwrap();
    let mut supported_channels = device
        .supported_formats()
        .unwrap()
        .map(|fmt| fmt.channels.len());
    let first_supported_channels = supported_channels.next().unwrap();
    let supported_channels = supported_channels.fold(first_supported_channels, std::cmp::max);

    // A channel for sending active sound info from the audio thread to the GUI.
    let (audio_monitor_tx, audio_monitor_rx) = mpsc::sync_channel(1024);

    // Initialise the audio model and create the stream.
    let audio_model = audio::Model::new(audio_monitor_tx, osc_out_msg_tx.clone());
    let audio_output_stream = app.audio
        .new_output_stream(audio_model, audio::render)
        .sample_rate(audio::SAMPLE_RATE as u32)
        .frames_per_buffer(audio::FRAMES_PER_BUFFER)
        .channels(std::cmp::min(supported_channels, audio::MAX_CHANNELS))
        .build()
        .unwrap();

    // To be shared between the `Composer` and `GUI` threads as both are responsible for creating
    // sounds and sending them to the audio thread.
    let sound_id_gen = audio::sound::IdGenerator::new();

    // Spawn the composer thread.
    let (composer_thread_handle, composer_msg_tx) =
        soundscape::spawn(audio_output_stream.clone(), sound_id_gen.clone());

    // Initalise the GUI model.
    let gui_channels = gui::Channels::new(
        osc_in_log_rx,
        osc_out_log_rx,
        osc_out_msg_tx,
        interaction_rx,
        composer_msg_tx.clone(),
        audio_output_stream.clone(),
        audio_monitor_rx,
    );
    let gui = gui::Model::new(&assets, config, app, window, gui_channels, sound_id_gen);

    Model {
        composer_thread_handle,
        composer_msg_tx,
        gui,
    }
}

// Update the application in accordance with the given event.
fn update(app: &App, mut model: Model, event: Event) -> Model {
    match event {
        Event::WindowEvent { simple: Some(_event), .. } => {
        },
        Event::Update(_update) => {
            model.gui.update();

            // If there are active sounds playing we should loop at a consistent rate for
            // visualisation. Otherwise, only update on interactions.
            if model.gui.is_animating() {
                app.set_loop_mode(LoopMode::rate_fps(60.0));
            } else {
                app.set_loop_mode(LoopMode::wait(3));
            }
        },
        _ => (),
    }
    model
}

// Draw the state of the application to the screen.
fn draw(app: &App, model: &Model, frame: Frame) -> Frame {
    model.gui.ui.draw_to_frame(app, &frame).unwrap();
    frame
}

// Re-join with spawned threads on application exit.
fn exit(_app: &App, model: Model) {
    let Model {
        gui,
        composer_msg_tx,
        composer_thread_handle,
        ..
    } = model;

    gui.exit();

    // Send exit signals to the audio and composer threads.
    composer_msg_tx.send(soundscape::Message::Exit).unwrap();

    // Wait for the composer thread to finish.
    composer_thread_handle.join().unwrap();
}
