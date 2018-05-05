// Extend the macro recursion limit to allow for many GUI widget IDs.
#![recursion_limit = "256"]

#[macro_use]
extern crate conrod;
#[macro_use]
extern crate conrod_derive;
#[macro_use]
extern crate custom_derive;
extern crate fxhash;
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
extern crate slug;
extern crate time_calc;
extern crate utils as mindtree_utils;
extern crate walkdir;

use config::Config;
use nannou::prelude::*;
use soundscape::Soundscape;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc};
use std::sync::atomic::AtomicUsize;

mod audio;
mod camera;
mod config;
mod gui;
mod installation;
mod master;
mod metres;
mod project;
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
    config: Config,
    audio_monitor: gui::monitor::Monitor,
    wav_reader: audio::source::wav::reader::Handle,
}

// The path to the server's config file.
fn config_path<P>(assets: P) -> PathBuf
where
    P: AsRef<Path>,
{
    assets.as_ref().join("config.json")
}

// Initialise the state of the application.
fn model(app: &App) -> Model {
    // If on macos, set the loop to wait mode.
    //
    // We only do this on macos as the app::Proxy is still a bit buggy on linux and windows has not
    // yet been tested.
    if cfg!(target_os = "macos") {
        // Set the app to wait on events.
        //
        // We will wake it up if it is necessary to re-instantiate and redraw the GUI.
        app.set_loop_mode(LoopMode::wait(3));
    }

    // Find the assets directory.
    let assets = app.assets_path()
        .expect("could not find assets directory");

    // Load the configuration struct.
    let config_path = config_path(&assets);
    let config: Config = utils::load_from_json_or_default(&config_path);

    // Spawn the OSC input thread.
    let osc_receiver = nannou::osc::receiver(config.osc_input_port)
        .unwrap_or_else(|err| {
            panic!("failed to create OSC receiver bound to port {}: {}", config.osc_input_port, err)
        });
    let (_osc_in_thread_handle, osc_in_log_rx, control_rx) = osc::input::spawn(osc_receiver);

    // Spawn the OSC output thread.
    let (_osc_out_thread_handle, osc_out_msg_tx, osc_out_log_rx) = osc::output::spawn();

    // A channel for sending active sound info from the audio thread to the GUI.
    let app_proxy = app.create_proxy();
    let (audio_monitor, audio_monitor_tx, audio_monitor_rx) = gui::monitor::spawn(app_proxy)
        .expect("failed to spawn audio_monitor thread");

    // Spawn the thread used for reading wavs.
    let wav_reader = audio::source::wav::reader::spawn();

    // A channel for sending and receiving on the soundscape thread.
    let (soundscape_tx, soundscape_rx) = mpsc::channel();

    // The playhead frame count shared between GUI, soundscape and audio output thread for
    // synchronising continuous WAV soures.
    let frame_count = Arc::new(AtomicUsize::new(0));

    // Initialise the audio input model and create the input stream.
    let input_device = app.audio.default_input_device()
        .expect("no default input device available on the system");
    let max_supported_input_channels = input_device.max_supported_input_channels();
    let audio_input_channels = std::cmp::min(max_supported_input_channels, audio::MAX_CHANNELS);
    let audio_input_model = audio::input::Model::new();
    let audio_input_stream = app.audio
        .new_input_stream(audio_input_model, audio::input::capture)
        .sample_rate(audio::SAMPLE_RATE as u32)
        .frames_per_buffer(audio::FRAMES_PER_BUFFER)
        .channels(audio_input_channels)
        .device(input_device)
        .build()
        .expect("failed to build audio input stream");

    // Initialise the audio output model and create the output stream.
    let output_device = app.audio.default_output_device()
        .expect("no default output device available on the system");
    let max_supported_output_channels = output_device.max_supported_output_channels();
    let audio_output_channels = std::cmp::min(max_supported_output_channels, audio::MAX_CHANNELS);
    let audio_output_model = audio::output::Model::new(
        frame_count.clone(),
        audio_monitor_tx,
        osc_out_msg_tx.clone(),
        soundscape_tx.clone(),
        wav_reader.clone(),
    );
    let audio_output_stream = app.audio
        .new_output_stream(audio_output_model, audio::output::render)
        .sample_rate(audio::SAMPLE_RATE as u32)
        .frames_per_buffer(audio::FRAMES_PER_BUFFER)
        .channels(audio_output_channels)
        .device(output_device)
        .build()
        .expect("failed to build audio output stream");

    // To be shared between the `Composer` and `GUI` threads as both are responsible for creating
    // sounds and sending them to the audio thread.
    let sound_id_gen = audio::sound::IdGenerator::new();

    // Spawn the composer thread.
    let soundscape = soundscape::spawn(
        frame_count.clone(),
        config.seed,
        soundscape_tx,
        soundscape_rx,
        wav_reader.clone(),
        audio_input_stream.clone(),
        audio_output_stream.clone(),
        sound_id_gen.clone(),
    );

    // Create a window.
    let window = app.new_window()
        .with_title("Audio Server")
        .with_dimensions(config.window_width, config.window_height)
        .with_vsync(true)
        .with_multisampling(4)
        .build()
        .expect("failed to create window");

    // Initalise the GUI model.
    let gui_channels = gui::Channels::new(
        frame_count,
        osc_in_log_rx,
        osc_out_log_rx,
        osc_out_msg_tx,
        control_rx,
        soundscape.clone(),
        wav_reader.clone(),
        audio_input_stream.clone(),
        audio_output_stream.clone(),
        audio_monitor_rx,
    );
    let gui = gui::Model::new(
        &assets,
        config.clone(),
        app,
        window,
        gui_channels,
        sound_id_gen,
        audio_input_channels,
        audio_output_channels,
    );

    Model {
        soundscape,
        config,
        gui,
        audio_monitor,
        wav_reader,
    }
}

// Update the application in accordance with the given event.
fn event(_app: &App, mut model: Model, event: Event) -> Model {
    match event {
        Event::WindowEvent {
            simple: Some(_event),
            ..
        } => {}
        Event::Update(_update) => {
            let Model { ref mut gui, ref config, .. } = model;
            gui.update(&config.project_default);
        }
        _ => (),
    }
    model
}

// Draw the state of the application to the screen.
fn view(app: &App, model: &Model, frame: Frame) -> Frame {
    model.gui.ui.draw_to_frame_if_changed(app, &frame).expect("failed to draw to frame");
    frame
}

// Re-join with spawned threads on application exit.
fn exit(app: &App, model: Model) {
    let Model {
        gui,
        mut config,
        soundscape,
        audio_monitor,
        wav_reader,
        ..
    } = model;

    /*
    let mut profile_path = std::env::current_dir().unwrap();
    profile_path.push("flame-graph.html");

    let p_threads = flame::threads().into_iter();
    let all_spans = p_threads.fold(Vec::<flame::Span>::new(), |mut acc, mut sp| {
        {
            acc.append(&mut sp.spans);
        }
        acc
    });


    html::dump_html_custom(&mut File::create(profile_path).unwrap(), &all_spans[..]).unwrap();
    */

    // Update whether or not cpu saving mode should be enabled when re-opening.
    config.cpu_saving_mode = gui.cpu_saving_mode;

    // Update the selected project directory slug if necessary.
    if let Some(selected_project_slug) = gui.selected_project_slug() {
        config.selected_project_slug = selected_project_slug;
    }

    // Find the assets directory so we can save state before closing.
    let assets = app.assets_path().expect("could not find assets directory");

    // Save the top-level json config.
    let config_path = config_path(&assets);
    if let Err(err) = utils::save_to_json(&config_path, &config) {
        eprintln!("failed to save \"assets/config.json\" during exit: {}", err);
    }

    // Save the selected gui project if there is one.
    if let Some((project, _)) = gui.project {
        if let Err(err) = project.save(&assets) {
            eprintln!("failed to save selected project during exit: {}", err);
        }
    }

    // Wait for the audio monitoring thread to close
    //
    // This should be instant as `GUI` has exited and the receiving channel should be dropped.
    audio_monitor.join().expect("failed to join audio_monitor thread when exiting");

    // Send exit signal to the composer thread.
    let soundscape_thread = soundscape.exit().expect("failed to exit soundscape thread");
    soundscape_thread.join().expect("failed to join the soundscape thread when exiting");

    // Send exit signal to the wav reader thread.
    let wav_reader_thread = wav_reader.exit().expect("failed to exit wav_reader thread");
    wav_reader_thread.join().expect("failed to join the wav_reader thread when exiting");
}
