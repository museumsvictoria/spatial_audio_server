// Extend the macro recursion limit to allow for many GUI widget IDs.
#![recursion_limit = "128"]

#[macro_use]
extern crate conrod;
#[macro_use]
extern crate conrod_derive;
#[macro_use]
extern crate custom_derive;
extern crate hound; // wav loading
extern crate nannou;
#[macro_use]
extern crate newtype_derive;
extern crate pitch_calc;
extern crate rustfft;
extern crate serde; // serialization
#[macro_use]
extern crate serde_derive;
extern crate serde_json;
extern crate time_calc;
extern crate toml;
extern crate utils as mindtree_utils;
extern crate walkdir;

use nannou::prelude::*;
use soundscape::Soundscape;
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
mod utils;

pub fn run() {
    nannou::app(model, event, view).exit(exit).run();
}

/// The model of the application state.
///
/// This is the state stored and updated on the main thread.
struct Model {
    gui: gui::Model,
    soundscape: Soundscape,
    audio_monitor: thread::JoinHandle<()>,
}

// Initialise the state of the application.
fn model(app: &App) -> Model {
    // Don't keep looping, just wait for events.
    app.set_loop_mode(LoopMode::wait(3));

    // Find the assets directory.
    let assets = app.assets_path()
        .expect("could not find assets directory");

    // Load the configuration struct.
    let config_path = assets.join("config.toml");
    let config = config::load(&config_path)
        .unwrap_or_else(|err| panic!("could not load {}: {}", config_path.display(), err));

    // Create a window.
    let window = app.new_window()
        .with_title("Audio Server")
        .with_dimensions(config.window_width, config.window_height)
        .with_vsync(true)
        .with_multisampling(4)
        .build()
        .expect("failed to create window");

    // Spawn the OSC input thread.
    let osc_receiver = nannou::osc::receiver(config.osc_input_port)
        .unwrap_or_else(|err| {
            panic!("failed to create OSC receiver bound to port {}: {}", config.osc_input_port, err)
        });
    let (_osc_in_thread_handle, osc_in_log_rx, interaction_rx) = osc::input::spawn(osc_receiver);

    // Spawn the OSC output thread.
    let (_osc_out_thread_handle, osc_out_msg_tx, osc_out_log_rx) = osc::output::spawn();

    // A channel for sending active sound info from the audio thread to the GUI.
    let app_proxy = app.create_proxy();
    let (audio_monitor, audio_monitor_tx, audio_monitor_rx) = gui::monitor::spawn(app_proxy)
        .expect("failed to spawn audio_monitor thread");

    // A channel for sending and receiving on the soundscape thread.
    let (soundscape_tx, soundscape_rx) = mpsc::channel();

    // Initialise the audio input model and create the input stream.
    let input_device = app.audio.default_input_device()
        .expect("no default input device available on the system");
    let max_supported_input_channels = if cfg!(feature = "test_with_stereo") {
        std::cmp::min(input_device.max_supported_input_channels(), 2)
    } else {
        input_device.max_supported_input_channels()
    };
    let max_supported_input_channels = std::cmp::min(max_supported_input_channels, 2);
    let audio_input_model = audio::input::Model::new();
    let audio_input_stream = app.audio
        .new_input_stream(audio_input_model, audio::input::capture)
        .sample_rate(audio::SAMPLE_RATE as u32)
        .frames_per_buffer(audio::FRAMES_PER_BUFFER)
        .channels(max_supported_input_channels)
        .device(input_device)
        .build()
        .expect("failed to build audio input stream");

    // Initialise the audio output model and create the output stream.
    let output_device = app.audio.default_output_device().unwrap();
    let max_supported_output_channels = if cfg!(feature = "test_with_stereo") {
        std::cmp::min(output_device.max_supported_output_channels(), 2)
    } else {
        output_device.max_supported_output_channels()
    };
    let audio_output_model = audio::output::Model::new(
        audio_monitor_tx,
        osc_out_msg_tx.clone(),
        soundscape_tx.clone(),
    );
    let audio_output_stream = app.audio
        .new_output_stream(audio_output_model, audio::output::render)
        .sample_rate(audio::SAMPLE_RATE as u32)
        .frames_per_buffer(audio::FRAMES_PER_BUFFER)
        .channels(std::cmp::min(
            max_supported_output_channels,
            audio::MAX_CHANNELS,
        ))
        .device(output_device)
        .build()
        .expect("failed to build audio output stream");

    // To be shared between the `Composer` and `GUI` threads as both are responsible for creating
    // sounds and sending them to the audio thread.
    let sound_id_gen = audio::sound::IdGenerator::new();

    // Spawn the composer thread.
    let soundscape = soundscape::spawn(
        config.seed,
        soundscape_tx,
        soundscape_rx,
        audio_input_stream.clone(),
        audio_output_stream.clone(),
        sound_id_gen.clone(),
    );

    // Initalise the GUI model.
    let gui_channels = gui::Channels::new(
        osc_in_log_rx,
        osc_out_log_rx,
        osc_out_msg_tx,
        interaction_rx,
        soundscape.clone(),
        audio_input_stream.clone(),
        audio_output_stream.clone(),
        audio_monitor_rx,
    );
    let gui = gui::Model::new(
        &assets,
        config,
        app,
        window,
        gui_channels,
        sound_id_gen,
        max_supported_input_channels,
    );

    Model {
        soundscape,
        gui,
        audio_monitor,
    }
}

// Update the application in accordance with the given event.
fn event(app: &App, mut model: Model, event: Event) -> Model {
    match event {
        Event::WindowEvent {
            simple: Some(_event),
            ..
        } => {}
        Event::Update(_update) => {
            model.gui.update();

            // If there are active sounds playing we should loop at a consistent rate for
            // visualisation. Otherwise, only update on interactions.
            if model.gui.is_animating() {
                app.set_loop_mode(LoopMode::rate_fps(60.0));
            } else {
                app.set_loop_mode(LoopMode::wait(3));
            }
        }
        _ => (),
    }
    model
}

// Draw the state of the application to the screen.
fn view(app: &App, model: &Model, frame: Frame) -> Frame {
    model.gui.ui.draw_to_frame(app, &frame).expect("failed to draw to frame");
    frame
}

// Re-join with spawned threads on application exit.
fn exit(_app: &App, model: Model) {
    let Model {
        gui,
        soundscape,
        audio_monitor,
        ..
    } = model;

    gui.exit();

    // Wait for the audio monitoring thread to close
    //
    // This should be instant as `GUI` has exited and the receiving channel should be dropped.
    audio_monitor.join().expect("failed to join audio_monitor thread when exiting");

    // Send exit signals to the audio and composer threads.
    let soundscape_thread = soundscape.exit().expect("only the main thread should exit soundscape");

    // Wait for the composer thread to finish.
    soundscape_thread.join().expect("failed to join the soundscape thread when exiting");

}
