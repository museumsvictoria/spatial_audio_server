use conrod::{self, color, text, widget, Colorable, Positionable, Scalar, Sizeable, UiBuilder,
             UiCell, Widget};
use conrod::backend::glium::{glium, Renderer};
use conrod::event::Input;
use conrod::render::OwnedPrimitives;
use image;
use std;
use std::path::{Path, PathBuf};
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

struct State {
    camera: Camera,
    side_menu_is_open: bool,
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
    // The position of the camera over the floorplan.
    //
    // [0.0, 0.0] - the centre of the floorplan.
    xy: [Scalar; 2],
    // The higher the zoom, the closer the floorplan appears.
    //
    // 1.0 - 100% zoom.
    // 2.0 - 200% zoom.
    // 0.0 - infinite zoom out.
    zoom: Scalar,
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
    display: &glium::Display,
    events_loop_proxy: glium::glutin::EventsLoopProxy,
) -> (Renderer, ImageMap, mpsc::Sender<Input>, mpsc::Receiver<OwnedPrimitives>) {
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
        camera: Camera {
            xy: [0.0, 0.0],
            zoom: 0.0,
        },
        side_menu_is_open: true,
    };

    // A renderer from conrod primitives to the OpenGL display.
    let renderer = Renderer::new(display).unwrap();

    // Channels for communication with the main thread.
    let (input_tx, input_rx) = mpsc::channel();
    let (render_tx, render_rx) = mpsc::channel();

    std::thread::Builder::new()
        .name("conrod_gui".into())
        .spawn(move || {
            // Many widgets require another frame to finish drawing after clicks or hovers, so we
            // insert an update into the conrod loop using this `bool` after each event.
            let mut needs_update = true;
            'conrod: loop {

                // Collect any pending inputs.
                let mut inputs = Vec::new();
                while let Ok(event) = input_rx.try_recv() {
                    inputs.push(event);
                }

                // If there are no inputs pending, wait for them.
                if inputs.is_empty() && !needs_update {
                    match input_rx.recv() {
                        Ok(event) => inputs.push(event),
                        Err(_) => break 'conrod,
                    };
                }

                needs_update = false;

                // Handle the received user input.
                for input in inputs {
                    ui.handle_event(input);
                    needs_update = true;
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

    (renderer, image_map, input_tx, render_rx)
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
        widget::CollapsibleArea::new(is_open, text).w_of(side_menu_id).h(30.0)
    }

    // Begin building a basic info text block.
    fn info_text(text: &str) -> widget::Text {
        widget::Text::new(&text)
            .font_size(12)
            .line_spacing(6.0)
    }
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
    const OPEN_SIDE_MENU_W: conrod::Scalar = 200.0;
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
        .color(color::BLACK)
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
    widget::Canvas::new()
        .w_h(floorplan_canvas_w, background_rect.h())
        .h_of(gui.ids.background)
        .color(color::WHITE)
        .align_right_of(gui.ids.background)
        .align_middle_y_of(gui.ids.background)
        .crop_kids()
        .set(gui.ids.floorplan_canvas, gui);

    let scale_w = floorplan_canvas_w / gui.images.floorplan.width;
    let scale_h = background_rect.h() / gui.images.floorplan.height;
    let scale = scale_w.min(scale_h);
    let floorplan_w = scale * gui.images.floorplan.width;
    let floorplan_h = scale * gui.images.floorplan.height;

    // Display the floorplan.
    widget::Image::new(gui.images.floorplan.id)
        .w_h(floorplan_w, floorplan_h)
        .middle_of(gui.ids.floorplan_canvas)
        .set(gui.ids.floorplan, gui);
}
