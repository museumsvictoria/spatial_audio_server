// Extend the macro recursion limit to allow for many GUI widget IDs.
#![recursion_limit = "256"]

use config::Config;
use nannou::prelude::*;
use soundscape::Soundscape;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicUsize;
use std::sync::{mpsc, Arc};

mod audio;
mod camera;
mod config;
mod gui;
mod installation;
mod master;
mod metres;
mod osc;
mod project;
mod soundscape;
mod utils;

pub fn run() {
    nannou::app(model)
        .update(update)
        .view(view)
        .exit(exit)
        .run();
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
        app.set_loop_mode(LoopMode::Wait);
    }
    println!("Starting ...");

    // Find the assets directory.
    let assets = app.assets_path().expect("could not find assets directory");
    println!("Found assets");

    // Load the configuration struct.
    let config_path = config_path(&assets);
    let config: Config = utils::load_from_json_or_default(&config_path);
    println!("Found config");

    // Spawn the OSC input thread.
    let osc_receiver = nannou_osc::receiver(config.osc_input_port).unwrap_or_else(|err| {
        panic!(
            "failed to create OSC receiver bound to port {}: {}",
            config.osc_input_port, err
        )
    });
    let (_osc_in_thread_handle, osc_in_log_rx, control_rx) = osc::input::spawn(osc_receiver);
    println!("spawned osc input thread");

    // Spawn the OSC output thread.
    let (_osc_out_thread_handle, osc_out_msg_tx, osc_out_log_rx) = osc::output::spawn();
    println!("spawned osc output thread");

    // A channel for sending active sound info from the audio thread to the GUI.
    let app_proxy = app.create_proxy();
    let (audio_monitor, audio_monitor_tx, audio_monitor_rx) =
        gui::monitor::spawn(app_proxy).expect("failed to spawn audio_monitor thread");
    println!("Spawned gui audio_monitor thread");

    // Spawn the thread used for reading wavs.
    let wav_reader = audio::source::wav::reader::spawn();
    println!("spawned wav file reader");

    // A channel for sending and receiving on the soundscape thread.
    let (soundscape_tx, soundscape_rx) = mpsc::channel();
    println!("created soundscape mpsc channel");

    // The playhead frame count shared between GUI, soundscape and audio output thread for
    // synchronising continuous WAV soures.
    let frame_count = Arc::new(AtomicUsize::new(0));
    println!("made a frame_count Arc<AtomicUsize>");

    // Retrieve the audio host.
    let audio_host = audio::host();
    println!("retrieved the audio host");

    // Initialise the audio input model and create the input stream.
    let input_device = audio::find_input_device(&audio_host, &config.target_input_device_name)
        .expect("no input devices available on the system");
    let max_supported_input_channels = input_device.max_supported_input_channels();
    let audio_input_channels = std::cmp::min(max_supported_input_channels, audio::MAX_CHANNELS);
    println!("Selected Input Device: {:?}", input_device.name());
    let audio_input_model = audio::input::Model::new();
    let audio_input_stream = audio_host
        .new_input_stream(audio_input_model)
        .capture(audio::input::capture)
        .sample_rate(audio::SAMPLE_RATE as u32)
        .frames_per_buffer(audio::FRAMES_PER_BUFFER)
        .channels(audio_input_channels)
        .device(input_device)
        .build()
        .expect("failed to build audio input stream");
    println!("created audio input stream");

    // Initialise the audio output model and create the output stream.
    let output_device = audio::find_output_device(&audio_host, &config.target_output_device_name)
        .expect("no output devices available on the system");
    println!("Selected Output Device: {:?}", output_device.name());
    let max_supported_output_channels = output_device.max_supported_output_channels();
    let audio_output_channels = std::cmp::min(max_supported_output_channels, audio::MAX_CHANNELS);
    let audio_output_model = audio::output::Model::new(
        frame_count.clone(),
        audio_monitor_tx,
        osc_out_msg_tx.clone(),
        soundscape_tx.clone(),
        wav_reader.clone(),
    );
    let audio_output_stream = audio_host
        .new_output_stream(audio_output_model)
        .render(audio::output::render)
        .sample_rate(audio::SAMPLE_RATE as u32)
        .frames_per_buffer(audio::FRAMES_PER_BUFFER)
        .channels(audio_output_channels)
        .device(output_device)
        .build()
        .expect("failed to build audio output stream");
    println!("created audio output stream");

    // To be shared between the `Composer` and `GUI` threads as both are responsible for creating
    // sounds and sending them to the audio thread.
    let sound_id_gen = audio::sound::IdGenerator::new();
    println!("created shared sound id generator");

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
    println!("spawned soundscape thread");

    // Create a window.
    let window = app
        .new_window()
        .title("Audio Server")
        .size(config.window_width, config.window_height)
        .build()
        .expect("failed to create window");
    println!("created window");

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
    println!("initialised the gui model");

    // Now that everything is initialized, kick off the input and output streams.
    //
    // Some platforms do this automatically, but this is necessary for platforms that are paused by
    // default (e.g. ASIO). Eventually, CPAL should be made to have consistent behaviour across
    // platforms.
    if let Err(err) = audio_input_stream.play() {
        eprintln!("Failed to start playing the audio input stream: {}", err);
    }
    if let Err(err) = audio_output_stream.play() {
        eprintln!("Failed to start playing the audio output stream: {}", err);
    }
    println!("Started the audio input and audio output streams");

    Model {
        soundscape,
        config,
        gui,
        audio_monitor,
        wav_reader,
    }
}

// Update the application in accordance with the given event.
fn update(_app: &App, model: &mut Model, _update: Update) {
    let Model {
        ref mut gui,
        ref config,
        ..
    } = *model;
    gui.update(&config.project_default);
}

// Draw the state of the application to the screen.
fn view(app: &App, model: &Model, frame: Frame) {
    model
        .gui
        .ui
        .draw_to_frame_if_changed(app, &frame)
        .expect("failed to draw to frame");
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
    audio_monitor
        .join()
        .expect("failed to join audio_monitor thread when exiting");

    // Send exit signal to the composer thread.
    let soundscape_thread = soundscape.exit().expect("failed to exit soundscape thread");
    soundscape_thread
        .join()
        .expect("failed to join the soundscape thread when exiting");

    // Send exit signal to the wav reader thread.
    let wav_reader_thread = wav_reader.exit().expect("failed to exit wav_reader thread");
    wav_reader_thread
        .join()
        .expect("failed to join the wav_reader thread when exiting");
}
