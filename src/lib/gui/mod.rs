use cgmath;
use config::Config;
use conrod::{self, color, text, widget, Colorable, Positionable, Scalar, Sizeable, UiBuilder,
             UiCell, Widget};
use conrod::backend::glium::{glium, Renderer};
use conrod::event::Input;
use conrod::render::OwnedPrimitives;
use image;
use interaction::Interaction;
use metres::Metres;
use rosc::OscMessage;
use std;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::net::SocketAddr;
use std::ops::{Deref, DerefMut};
use std::sync::mpsc;

mod theme;

/// A convenience wrapper that borrows the GUI state necessary for instantiating the widgets.
struct Gui<'a> {
    ui: UiCell<'a>,
    /// The images used throughout the GUI.
    images: &'a Images,
    fonts: &'a Fonts,
    ids: &'a Ids,
    state: &'a mut State,
}

/// Messages received by the GUI thread.
pub enum Message {
    Osc(SocketAddr, OscMessage),
    Interaction(Interaction),
    Input(Input),
}

struct State {
    // The loaded config file.
    config: Config,
    // The camera over the 2D floorplan.
    camera: Camera,
    // A log of the most recently received OSC messages for testing/debugging/monitoring.
    osc_log: OscLog,
    // A log of the most recently received Interactions for testing/debugging/monitoring.
    interaction_log: InteractionLog,
    // Menu states.
    side_menu_is_open: bool,
    osc_log_is_open: bool,
    interaction_log_is_open: bool,
}

impl<'a> Deref for Gui<'a> {
    type Target = UiCell<'a>;
    fn deref(&self) -> &Self::Target {
        &self.ui
    }
}

impl<'a> DerefMut for Gui<'a> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.ui
    }
}

type ImageMap = conrod::image::Map<glium::texture::Texture2d>;

#[derive(Clone, Copy, Debug)]
struct Image {
    id: conrod::image::Id,
    width: Scalar,
    height: Scalar,
}

#[derive(Debug)]
struct Images {
    floorplan: Image,
}

#[derive(Debug)]
struct Fonts {
    notosans_regular: text::font::Id,
}

// A 2D camera used to navigate around the floorplan visualisation.
#[derive(Debug)]
struct Camera {
    // The number of floorplan pixels per metre.
    floorplan_pixels_per_metre: f64,
    // The position of the camera over the floorplan.
    //
    // [0.0, 0.0] - the centre of the floorplan.
    position: cgmath::Point2<Metres>,
    // The higher the zoom, the closer the floorplan appears.
    //
    // The zoom can be multiplied by a distance in metres to get the equivalent distance as a GUI
    // scalar value.
    //
    // 1.0 - Original resolution.
    // 0.5 - 50% view.
    zoom: Scalar,
}

impl Camera {
    /// Convert from metres to the GUI scalar value.
    fn metres_to_scalar(&self, Metres(metres): Metres) -> Scalar {
        self.zoom * metres * self.floorplan_pixels_per_metre
    }

    /// Convert from the GUI scalar value to metres.
    fn scalar_to_metres(&self, scalar: Scalar) -> Metres {
        Metres((scalar / self.zoom) / self.floorplan_pixels_per_metre)
    }
}

struct Log<T> {
    // Newest to oldest is stored front to back respectively.
    deque: VecDeque<T>,
    // The index of the oldest message currently stored in the deque.
    start_index: usize,
    // The max number of messages stored in the log at one time.
    limit: usize,
}

type OscLog = Log<(SocketAddr, OscMessage)>;
type InteractionLog = Log<Interaction>;

impl<T> Log<T> {
    // Construct an OscLog that stores the given max number of messages.
    fn with_limit(limit: usize) -> Self {
        Log {
            deque: VecDeque::new(),
            start_index: 0,
            limit,
        }
    }

    // Push a new OSC message to the log.
    fn push_msg(&mut self, msg: T) {
        self.deque.push_front(msg);
        while self.deque.len() > self.limit {
            self.deque.pop_back();
            self.start_index += 1;
        }
    }
}

impl OscLog {
    // Format the log in a single string of messages.
    fn format(&self) -> String {
        let mut s = String::new();
        let mut index = self.start_index + self.deque.len();
        for &(ref addr, ref msg) in &self.deque {
            let addr_string = format!("{}: [{}{}]\n", index, addr, msg.addr);
            s.push_str(&addr_string);

            // Arguments.
            if let Some(ref args) = msg.args {
                for arg in args {
                    s.push_str(&format!("    {:?}\n", arg));
                }
            }

            index -= 1;
        }
        s
    }
}

impl InteractionLog {
    // Format the log in a single string of messages.
    fn format(&self) -> String {
        let mut s = String::new();
        let mut index = self.start_index + self.deque.len();
        for &interaction in &self.deque {
            let line = format!("{}: {:?}\n", index, interaction);
            s.push_str(&line);
            index -= 1;
        }
        s
    }
}

impl<T> Deref for Log<T> {
    type Target = VecDeque<T>;
    fn deref(&self) -> &Self::Target {
        &self.deque
    }
}

/// The directory in which all fonts are stored.
fn fonts_directory(assets: &Path) -> PathBuf {
    assets.join("fonts")
}

/// The directory in which all images are stored.
fn images_directory(assets: &Path) -> PathBuf {
    assets.join("images")
}

/// Load the image at the given path into a texture.
///
/// Returns the dimensions of the image alongside the texture.
fn load_image(
    path: &Path,
    display: &glium::Display,
) -> ((Scalar, Scalar), glium::texture::Texture2d) {
    let rgba_image = image::open(&path).unwrap().to_rgba();
    let (w, h) = rgba_image.dimensions();
    let raw_image =
        glium::texture::RawImage2d::from_raw_rgba_reversed(&rgba_image.into_raw(), (w, h));
    let texture = glium::texture::Texture2d::new(display, raw_image).unwrap();
    ((w as Scalar, h as Scalar), texture)
}

/// Insert the image at the given path into the given `ImageMap`.
///
/// Return its Id and Dimensions in the form of an `Image`.
fn insert_image(path: &Path, display: &glium::Display, image_map: &mut ImageMap) -> Image {
    let ((width, height), texture) = load_image(path, display);
    let id = image_map.insert(texture);
    let image = Image { id, width, height };
    image
}

/// Spawn the GUI thread.
///
/// The GUI thread is driven by input sent from the main thread. It sends back graphics primitives
/// when a received `Message` would require redrawing the GUI.
pub fn spawn(
    assets: &Path,
    config: Config,
    display: &glium::Display,
    events_loop_proxy: glium::glutin::EventsLoopProxy,
    osc_msg_rx: mpsc::Receiver<(SocketAddr, OscMessage)>,
    interaction_rx: mpsc::Receiver<Interaction>,
) -> (Renderer, ImageMap, mpsc::Sender<Message>, mpsc::Receiver<OwnedPrimitives>) {
    // Use the width and height of the display as the initial size for the Ui.
    let (display_w, display_h) = display.gl_window().get_inner_size_points().unwrap();
    let ui_dimensions = [display_w as Scalar, display_h as Scalar];
    let theme = theme::construct();
    let mut ui = UiBuilder::new(ui_dimensions).theme(theme).build();

    // The type containing the unique ID for each widget in the GUI.
    let ids = Ids::new(ui.widget_id_generator());

    // Load and insert the fonts to be used.
    let font_path = fonts_directory(assets).join("NotoSans/NotoSans-Regular.ttf");
    let notosans_regular = ui.fonts.insert_from_file(font_path).unwrap();
    let fonts = Fonts { notosans_regular };

    // Load and insert the images to be used.
    let mut image_map = ImageMap::new();
    let floorplan_path = images_directory(assets).join("floorplan.png");
    let floorplan = insert_image(&floorplan_path, display, &mut image_map);
    let images = Images { floorplan };

    // State that is specific to the GUI itself.
    let mut state = State {
        config,
        // TODO: Possibly load camera from file.
        camera: Camera {
            floorplan_pixels_per_metre: config.floorplan_pixels_per_metre,
            position: cgmath::Point2 { x: Metres(0.0), y: Metres(0.0) },
            zoom: 0.0,
        },
        osc_log: Log::with_limit(config.osc_log_limit),
        interaction_log: Log::with_limit(config.interaction_log_limit),
        side_menu_is_open: true,
        osc_log_is_open: true,
        interaction_log_is_open: true,
    };

    // A renderer from conrod primitives to the OpenGL display.
    let renderer = Renderer::new(display).unwrap();

    // Channels for communication with the main thread.
    let (msg_tx, msg_rx) = mpsc::channel();
    let (render_tx, render_rx) = mpsc::channel();

    // Spawn a thread that converts the OSC messages to GUI messages.
    let msg_tx_clone = msg_tx.clone();
    std::thread::Builder::new()
        .name("osc_to_gui_msg".into())
        .spawn(move || {
            for (addr, msg) in osc_msg_rx {
                if msg_tx_clone.send(Message::Osc(addr, msg)).is_err() {
                    break;
                }
            }
        })
        .unwrap();

    // Spawn a thread that converts the Interaction messages to GUI messages.
    let msg_tx_clone = msg_tx.clone();
    std::thread::Builder::new()
        .name("interaction_to_gui_msg".into())
        .spawn(move || {
            for interaction in interaction_rx {
                if msg_tx_clone.send(Message::Interaction(interaction)).is_err() {
                    break;
                }
            }
        })
        .unwrap();

    // Spawn the main GUI thread.
    std::thread::Builder::new()
        .name("conrod_gui".into())
        .spawn(move || {

            // Many widgets require another frame to finish drawing after clicks or hovers, so we
            // insert an update into the conrod loop using this `bool` after each event.
            let mut needs_update = true;

            // A buffer for collecting OSC messages.
            let mut msgs = Vec::new();

            'conrod: loop {

                // Collect any pending messages.
                msgs.extend(msg_rx.try_iter());

                // If there are no messages pending, wait for them.
                if msgs.is_empty() && !needs_update {
                    match msg_rx.recv() {
                        Ok(msg) => msgs.push(msg),
                        Err(_) => break 'conrod,
                    };
                }

                needs_update = false;
                for msg in msgs.drain(..) {
                    match msg {
                        Message::Osc(addr, osc) =>
                            state.osc_log.push_msg((addr, osc)),
                        Message::Interaction(interaction) =>
                            state.interaction_log.push_msg(interaction),
                        Message::Input(input) => {
                            ui.handle_event(input);
                            needs_update = true;
                        },
                    }
                }

                // Instantiate the widgets.
                {
                    let mut gui = Gui {
                        ui: ui.set_widgets(),
                        ids: &ids,
                        images: &images,
                        fonts: &fonts,
                        state: &mut state,
                    };
                    set_widgets(&mut gui);
                }

                // Render the `Ui` to a list of primitives that we can send to the main thread for
                // display. Wakeup `winit` for rendering.
                if let Some(primitives) = ui.draw_if_changed() {
                    if render_tx.send(primitives.owned()).is_err() ||
                        events_loop_proxy.wakeup().is_err()
                    {
                        break 'conrod;
                    }
                }
            }
        })
        .unwrap();

    (renderer, image_map, msg_tx, render_rx)
}

/// Draws the given `primitives` to the given `Display`.
pub fn draw(
    display: &glium::Display,
    renderer: &mut Renderer,
    image_map: &ImageMap,
    primitives: &OwnedPrimitives,
) {
    use conrod::backend::glium::glium::Surface;
    renderer.fill(display, primitives.walk(), &image_map);
    let mut target = display.draw();
    target.clear_color(0.0, 0.0, 0.0, 1.0);
    renderer.draw(display, &mut target, &image_map).unwrap();
    target.finish().unwrap();
}

// A unique ID foor each widget in the GUI.
widget_ids! {
    pub struct Ids {
        // The backdrop for all widgets.
        background,

        // The canvas for the menu to the left of the GUI.
        side_menu,
        // The menu button at the top of the sidebar.
        side_menu_button,
        side_menu_button_line_top,
        side_menu_button_line_middle,
        side_menu_button_line_bottom,
        // OSC Log.
        osc_log,
        osc_log_text,
        osc_log_scrollbar_y,
        osc_log_scrollbar_x,
        // Interaction Log.
        interaction_log,
        interaction_log_text,
        interaction_log_scrollbar_y,
        interaction_log_scrollbar_x,

        // The floorplan image and the canvas on which it is placed.
        floorplan_canvas,
        floorplan,
    }
}

// Set the widgets in the side menu.
fn set_side_menu_widgets(gui: &mut Gui) {

    // Begin building a `CollapsibleArea` for the sidebar.
    fn collapsible_area(is_open: bool, text: &str, side_menu_id: widget::Id)
        -> widget::CollapsibleArea
    {
        widget::CollapsibleArea::new(is_open, text)
            .w_of(side_menu_id)
            .h(30.0)
    }

    // Begin building a basic info text block.
    fn info_text(text: &str) -> widget::Text {
        widget::Text::new(&text)
            .font_size(12)
            .line_spacing(6.0)
    }

    // The log of received OSC messages.
    let last_area_id = {
        let is_open = gui.state.osc_log_is_open;
        let log_canvas_h = 300.0;
        let (area, event) = collapsible_area(is_open, "OSC Input Log", gui.ids.side_menu)
            .align_middle_x_of(gui.ids.side_menu)
            .down_from(gui.ids.side_menu_button, 0.0)
            .set(gui.ids.osc_log, gui);
        if let Some(event) = event {
            gui.state.osc_log_is_open = event.is_open();
        }
        if let Some(area) = area {

            // The canvas on which the log will be placed.
            let canvas = widget::Canvas::new()
                .scroll_kids()
                .pad(10.0)
                .h(log_canvas_h);
            area.set(canvas, gui);

            // The text widget used to display the log.
            let log_string = match gui.state.osc_log.len() {
                0 => format!("No messages received yet.\nListening on port {}...",
                             gui.state.config.osc_input_port),
                _ => gui.state.osc_log.format(),
            };
            info_text(&log_string)
                .top_left_of(area.id)
                .kid_area_w_of(area.id)
                .set(gui.ids.osc_log_text, gui);

            // Scrollbars.
            widget::Scrollbar::y_axis(area.id)
                .color(color::LIGHT_CHARCOAL)
                .auto_hide(false)
                .set(gui.ids.osc_log_scrollbar_y, gui);
            widget::Scrollbar::x_axis(area.id)
                .color(color::LIGHT_CHARCOAL)
                .auto_hide(true)
                .set(gui.ids.osc_log_scrollbar_x, gui);

            area.id
        } else {
            gui.ids.osc_log
        }
    };

    // The log of received Interactions.
    let last_area_id = {
        let is_open = gui.state.interaction_log_is_open;
        let log_canvas_h = 300.0;
        let (area, event) = collapsible_area(is_open, "Interaction Log", gui.ids.side_menu)
            .align_middle_x_of(gui.ids.side_menu)
            .down_from(last_area_id, 0.0)
            .set(gui.ids.interaction_log, gui);
        if let Some(event) = event {
            gui.state.interaction_log_is_open = event.is_open();
        }

        if let Some(area) = area {
            // The canvas on which the log will be placed.
            let canvas = widget::Canvas::new()
                .scroll_kids()
                .pad(10.0)
                .h(log_canvas_h);
            area.set(canvas, gui);

            // The text widget used to display the log.
            let log_string = match gui.state.interaction_log.len() {
                0 => format!("No interactions received yet.\nListening on port {}...",
                             gui.state.config.osc_input_port),
                _ => gui.state.interaction_log.format(),
            };
            info_text(&log_string)
                .top_left_of(area.id)
                .kid_area_w_of(area.id)
                .set(gui.ids.interaction_log_text, gui);

            // Scrollbars.
            widget::Scrollbar::y_axis(area.id)
                .color(color::LIGHT_CHARCOAL)
                .auto_hide(false)
                .set(gui.ids.interaction_log_scrollbar_y, gui);
            widget::Scrollbar::x_axis(area.id)
                .color(color::LIGHT_CHARCOAL)
                .auto_hide(true)
                .set(gui.ids.interaction_log_scrollbar_x, gui);

            area.id
        } else {
            gui.ids.interaction_log
        }
    };
}

// Update all widgets in the GUI with the given state.
fn set_widgets(gui: &mut Gui) {

    let background_color = color::WHITE;

    // The background for the main `UI` window.
    widget::Canvas::new()
        .color(background_color)
        .pad(0.0)
        .parent(gui.window)
        .middle_of(gui.window)
        .wh_of(gui.window)
        .set(gui.ids.background, gui);

    // A thin menu bar on the left.
    //
    // The menu bar is collapsed by default, and shows three lines at the top.
    // Pressing these three lines opens the menu, revealing a list of options.
    const CLOSED_SIDE_MENU_W: conrod::Scalar = 40.0;
    const OPEN_SIDE_MENU_W: conrod::Scalar = 300.0;
    let side_menu_is_open = gui.state.side_menu_is_open;
    let side_menu_w = match side_menu_is_open {
        false => CLOSED_SIDE_MENU_W,
        true => OPEN_SIDE_MENU_W,
    };

    // The canvas on which all side_menu widgets are placed.
    widget::Canvas::new()
        .w(side_menu_w)
        .h_of(gui.ids.background)
        .mid_left_of(gui.ids.background)
        .pad(0.0)
        .color(color::rgb(0.1, 0.13, 0.15))
        .set(gui.ids.side_menu, gui);

    // The classic three line menu button for opening the side_menu.
    for _click in widget::Button::new()
        .w_h(side_menu_w, CLOSED_SIDE_MENU_W)
        .mid_top_of(gui.ids.side_menu)
        //.color(color::BLACK)
        .color(color::rgb(0.07, 0.08, 0.09))
        .set(gui.ids.side_menu_button, gui)
    {
        gui.state.side_menu_is_open = !side_menu_is_open;
    }

    // Draw the three lines using rectangles.
    fn menu_button_line(menu_button: widget::Id) -> widget::Rectangle {
        let line_h = 2.0;
        let line_w = CLOSED_SIDE_MENU_W / 3.0;
        widget::Rectangle::fill([line_w, line_h])
            .color(color::WHITE)
            .graphics_for(menu_button)
    }

    let margin = CLOSED_SIDE_MENU_W / 3.0;
    menu_button_line(gui.ids.side_menu_button)
        .mid_top_with_margin_on(gui.ids.side_menu_button, margin)
        .set(gui.ids.side_menu_button_line_top, gui);
    menu_button_line(gui.ids.side_menu_button)
        .middle_of(gui.ids.side_menu_button)
        .set(gui.ids.side_menu_button_line_middle, gui);
    menu_button_line(gui.ids.side_menu_button)
        .mid_bottom_with_margin_on(gui.ids.side_menu_button, margin)
        .set(gui.ids.side_menu_button_line_bottom, gui);

    // If the side_menu is open, set all the side_menu widgets.
    if side_menu_is_open {
        set_side_menu_widgets(gui);
    }

    // The canvas on which the floorplan will be displayed.
    let background_rect = gui.rect_of(gui.ids.background).unwrap();
    let floorplan_canvas_w = background_rect.w() - side_menu_w;
    let floorplan_canvas_h = background_rect.h();
    widget::Canvas::new()
        .w_h(floorplan_canvas_w, floorplan_canvas_h)
        .h_of(gui.ids.background)
        .color(color::WHITE)
        .align_right_of(gui.ids.background)
        .align_middle_y_of(gui.ids.background)
        .crop_kids()
        .set(gui.ids.floorplan_canvas, gui);

    let floorplan_pixels_per_metre = gui.state.camera.floorplan_pixels_per_metre;
    let metres_from_floorplan_pixels = |px| Metres(px / floorplan_pixels_per_metre);
    let metres_to_floorplan_pixels = |Metres(m)| m * floorplan_pixels_per_metre;

    let floorplan_w_metres = metres_from_floorplan_pixels(gui.images.floorplan.width);
    let floorplan_h_metres = metres_from_floorplan_pixels(gui.images.floorplan.height);

    // The amount which the image must be scaled to fill the floorplan_canvas while preserving
    // aspect ratio.
    let full_scale_w = floorplan_canvas_w / gui.images.floorplan.width;
    let full_scale_h = floorplan_canvas_h / gui.images.floorplan.height;
    let floorplan_w = full_scale_w * gui.images.floorplan.width;
    let floorplan_h = full_scale_h * gui.images.floorplan.height;

    // If the floorplan was scrolled, adjust the camera zoom.
    let total_scroll = gui.widget_input(gui.ids.floorplan)
        .scrolls()
        .fold(0.0, |acc, scroll| acc + scroll.y);
    gui.state.camera.zoom = (gui.state.camera.zoom - total_scroll / 200.0)
        .max(full_scale_w.min(full_scale_h))
        .min(1.0);

    // Move the camera by clicking with the left mouse button and dragging.
    let total_drag = gui.widget_input(gui.ids.floorplan)
        .drags()
        .left()
        .map(|drag| drag.delta_xy)
        .fold([0.0, 0.0], |acc, dt| [acc[0] + dt[0], acc[1] + dt[1]]);
    gui.state.camera.position.x -= gui.state.camera.scalar_to_metres(total_drag[0]);
    gui.state.camera.position.y -= gui.state.camera.scalar_to_metres(total_drag[1]);

    // The part of the image visible from the camera.
    let visible_w_m = gui.state.camera.scalar_to_metres(floorplan_canvas_w);
    let visible_h_m = gui.state.camera.scalar_to_metres(floorplan_canvas_h);

    // Clamp the camera's position so it doesn't go out of bounds.
    let invisible_w_m = floorplan_w_metres - visible_w_m;
    let invisible_h_m = floorplan_h_metres - visible_h_m;
    let half_invisible_w_m = invisible_w_m * 0.5;
    let half_invisible_h_m = invisible_h_m * 0.5;
    let centre_x_m = floorplan_w_metres * 0.5;
    let centre_y_m = floorplan_h_metres * 0.5;
    let min_cam_x_m = centre_x_m - half_invisible_w_m;
    let max_cam_x_m = centre_x_m + half_invisible_w_m;
    let min_cam_y_m = centre_y_m - half_invisible_h_m;
    let max_cam_y_m = centre_y_m + half_invisible_h_m;
    gui.state.camera.position.x = gui.state.camera.position.x.max(min_cam_x_m).min(max_cam_x_m);
    gui.state.camera.position.y = gui.state.camera.position.y.max(min_cam_y_m).min(max_cam_y_m);

    let visible_x = metres_to_floorplan_pixels(gui.state.camera.position.x);
    let visible_y = metres_to_floorplan_pixels(gui.state.camera.position.y);
    let visible_w = metres_to_floorplan_pixels(visible_w_m);
    let visible_h = metres_to_floorplan_pixels(visible_h_m);
    let visible_rect = conrod::Rect::from_xy_dim([visible_x, visible_y], [visible_w, visible_h]);

    // Display the floorplan.
    widget::Image::new(gui.images.floorplan.id)
        .source_rectangle(visible_rect)
        .w_h(floorplan_w, floorplan_h)
        .middle_of(gui.ids.floorplan_canvas)
        .set(gui.ids.floorplan, gui);
}
