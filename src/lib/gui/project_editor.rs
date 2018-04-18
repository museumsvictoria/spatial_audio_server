//! A "Projects" side-bar widget providing allowing the user to create and remove new projects.

use gui::{collapsible_area, Gui, ProjectState, State, TEXT_PAD, ITEM_HEIGHT, SMALL_FONT_SIZE};
use project::{self, Project};
use nannou::ui;
use nannou::ui::prelude::*;
use slug::slugify;
use std::fs;
use std::path::Path;

/// State related to the project editor GUI.
#[derive(Default)]
pub struct ProjectEditor {
    pub text_box_name: String,
}

pub fn set(
    gui: &mut Gui,
    project: &mut Option<(Project, ProjectState)>,
    default_project_config: &project::Config,
) -> widget::Id {
    let Gui {
        ref mut ui,
        ref ids,
        ref channels,
        ref assets,
        state: &mut State {
            ref mut is_open,
            ref mut project_editor,
            ..
        },
        ..
    } = *gui;

    // The collapsible area widget.
    let (area, event) = collapsible_area(is_open.project_editor, "Projects", ids.side_menu)
        .mid_top_of(ids.side_menu)
        .set(ids.project_editor, ui);
    if let Some(event) = event {
        is_open.project_editor = event.is_open();
    }

    // Return early if the panel is not open.
    let area = match area {
        None => return ids.project_editor,
        Some(area) => area,
    };

    const PROJECT_LIST_MAX_H: Scalar = ITEM_HEIGHT * 3.0;
    const BUTTON_H: Scalar = ITEM_HEIGHT;
    const NAME_TEXT_BOX_H: Scalar = ITEM_HEIGHT;
    const CANVAS_H: Scalar = PROJECT_LIST_MAX_H + BUTTON_H + NAME_TEXT_BOX_H;

    // The canvas on which the controls will be placed.
    let canvas = widget::Canvas::new().pad(0.0).h(CANVAS_H);
    area.set(canvas, ui);

    let button_w = ui.kid_area_of(area.id).unwrap().w() / 3.0;
    let button = || widget::Button::new()
        .color(super::DARK_A)
        .label_font_size(SMALL_FONT_SIZE)
        .w(button_w)
        .h(BUTTON_H);

    // Show the plus button at the bottom of the editor.
    for _click in button()
        .label("ADD")
        .top_left_with_margins_on(area.id, PROJECT_LIST_MAX_H, 0.0)
        .set(ids.project_editor_add, ui)
    {
        // If a project was already selected, attempt to save it before creating and loading the
        // new empty project.
        if let Some((project, _)) = project.take() {
            project
                .save(assets)
                .expect("failed to save the project before switching to the new one");
        }

        // Create a new default project.
        let new_project = Project::new(assets, default_project_config);
        new_project.save(assets).expect("failed to create new project directory");
        new_project.reset_and_sync_all_threads(channels);
        let new_project_state = ProjectState::default();
        project_editor.text_box_name = new_project.name.clone();
        *project = Some((new_project, new_project_state));
    }

    // Show the plus button at the bottom of the editor.
    for _click in button()
        .label("COPY")
        .align_top_of(ids.project_editor_add)
        .right(0.0)
        .set(ids.project_editor_copy, ui)
    {
        // If a project was already selected, attempt to save it before creating and loading the
        // new empty project.
        if let Some((old_project, _)) = project.take() {
            old_project
                .save(assets)
                .expect("failed to save the project before switching to the new one");

            // Create a new default project.
            let mut new_project = old_project;
            new_project.name = format!("{} copy", new_project.name);
            new_project.save(assets).expect("failed to create new project directory");
            new_project.reset_and_sync_all_threads(channels);
            let new_project_state = ProjectState::default();
            project_editor.text_box_name = new_project.name.clone();
            *project = Some((new_project, new_project_state));
        }
    }

    // Show the plus button at the bottom of the editor.
    for _click in button()
        .label("SAVE")
        .align_top_of(ids.project_editor_add)
        .right(0.0)
        .set(ids.project_editor_save, ui)
    {
        // If a project was already selected, attempt to save it before creating and loading the
        // new empty project.
        if let Some((project, _)) = project.take() {
            project.save(assets).expect("failed to save the project");
        }
    }

    // Collect the list of directories.
    let mut project_directories = match project::load_project_directories(assets) {
        Ok(dirs) => dirs,
        Err(err) => {
            let text = format!("Failed to load project directories!\n{}", err);
            widget::Text::new(&text)
                .padded_w_of(area.id, TEXT_PAD)
                .mid_top_with_margin_on(area.id, TEXT_PAD)
                .font_size(SMALL_FONT_SIZE)
                .center_justify()
                .set(ids.project_editor_none, ui);
            return area.id;
        },
    };

    // If there are no projects, say so!
    if project_directories.is_empty() {
        let text = format!("No Projects!\nPress the `+` button below.");
        widget::Text::new(&text)
            .padded_w_of(area.id, TEXT_PAD)
            .mid_top_with_margin_on(area.id, TEXT_PAD)
            .font_size(SMALL_FONT_SIZE)
            .center_justify()
            .set(ids.project_editor_none, ui);
        return area.id;
    }

    // Sort the directories.
    project_directories.sort();

    // Convert the directories to their file stems (project slugs).
    let mut project_slugs: Vec<_> = project_directories
        .iter()
        .map(|dir| {
            dir.file_stem()
                .expect("no file stem for loaded project directory")
                .to_str()
                .expect("could not read project directory as utf8 string")
                .to_string()
        })
        .collect();

    // The slug of the selected project if there is one.
    let mut selected_project_slug = project.as_ref().map(|&(ref p, _)| slugify(&p.name));

    // Instantiate the list of projects.
    let num_items = project_slugs.len();
    let (mut list_events, scrollbar) = widget::ListSelect::single(num_items)
        .item_size(ITEM_HEIGHT)
        .w_of(area.id)
        .h(PROJECT_LIST_MAX_H)
        .align_top_of(area.id)
        .align_middle_x_of(area.id)
        .parent(area.id)
        .scrollbar_next_to()
        .scrollbar_color(color::LIGHT_CHARCOAL)
        .set(ids.project_editor_list, ui);

    // If a project was removed, process it after the whole list is instantiated to avoid
    // invalid indices.
    let mut maybe_remove_index = None;
    while let Some(event) = list_events.next(ui, |i| {
        selected_project_slug.as_ref() == Some(&project_slugs[i])
    }) {
        use self::ui::widget::list_select::Event;
        match event {
            // Instantiate a button for each project.
            Event::Item(item) => {
                let label = &project_slugs[item.i];
                let is_selected = selected_project_slug.as_ref() == Some(label);

                // Blue if selected, gray otherwise.
                let color = if is_selected {
                    color::BLUE
                } else {
                    color::CHARCOAL
                };

                // The button widget.
                let button = widget::Button::new()
                    .label(&label)
                    .label_font_size(SMALL_FONT_SIZE)
                    .label_x(position::Relative::Place(position::Place::Start(Some(
                        10.0,
                    ))))
                    .color(color);
                item.set(button, ui);

                // If the button or any of its children are capturing the mouse, display
                // the `remove` button.
                let show_remove_button = ui.global_input()
                    .current
                    .widget_capturing_mouse
                    .map(|id| {
                        id == item.widget_id
                            || ui.widget_graph()
                                .does_recursive_depth_edge_exist(item.widget_id, id)
                    })
                    .unwrap_or(false);

                if !show_remove_button {
                    continue;
                }

                if widget::Button::new()
                    .label("X")
                    .label_font_size(SMALL_FONT_SIZE)
                    .color(color::DARK_RED.alpha(0.5))
                    .w_h(ITEM_HEIGHT, ITEM_HEIGHT)
                    .align_right_of(item.widget_id)
                    .align_middle_y_of(item.widget_id)
                    .parent(item.widget_id)
                    .set(ids.project_editor_remove, ui)
                    .was_clicked()
                {
                    maybe_remove_index = Some(item.i);
                }
            }

            // Update the selected project.
            Event::Selection(idx) => {
                // If the project no longer exists, don't change anything.
                let project_directory = &project_directories[idx];
                if !project_directory.exists() {
                    continue;
                }

                // If a project was already selected, attempt to save it before creating and loading the
                // new empty project.
                if let Some((project, _)) = project.take() {
                    project
                        .save(assets)
                        .expect("failed to save the project before switching to the new one");
                }

                // Load the project.
                let loaded_project = Project::load(assets, &project_directory, default_project_config);
                loaded_project.reset_and_sync_all_threads(channels);
                let loaded_project_state = ProjectState::default();
                selected_project_slug = Some(slugify(&loaded_project.name));
                project_editor.text_box_name = loaded_project.name.clone();
                *project = Some((loaded_project, loaded_project_state));
            },

            _ => (),
        }
    }

    // The scrollbar for the list.
    if let Some(s) = scrollbar {
        s.set(ui);
    }

    // Remove a project if necessary.
    if let Some(i) = maybe_remove_index {
        let directory = project_directories.remove(i);
        let slug = project_slugs.remove(i);

        // Unselect the project if necessary.
        if Some(slug) == selected_project_slug {
            project.take();
        }

        // Remove the project directory.
        if let Err(err) = fs::remove_dir_all(&directory) {
            eprintln!("failed to remove project directory `{}`: \"{}\"", directory.display(), err);
        }

        // Select the next project if there is one.
        if !project_directories.is_empty() {
            let n_projects = project_directories.len();
            let i = if i < n_projects { i } else { i - 1 };
            let directory = &project_directories[i];

            // Load the project.
            let loaded_project = Project::load(assets, &directory, default_project_config);
            loaded_project.reset_and_sync_all_threads(channels);
            let loaded_project_state = ProjectState::default();
            project_editor.text_box_name = loaded_project.name.clone();
            *project = Some((loaded_project, loaded_project_state));
        }
    }

    // Get the selected project in there is one.
    let project = match *project {
        Some((ref mut project, _)) => project,
        None => return area.id,
    };

    // TextBox for name.
    fn is_name_valid(projects_directory: &Path, n: &str) -> bool {
        let slug = slugify(n);
        !slug.is_empty() && !projects_directory.join(slug).exists()
    }

    let color = if project.name == project_editor.text_box_name {
        None
    } else {
        let projects_directory = project::projects_directory(assets);
        if is_name_valid(&projects_directory, &project_editor.text_box_name) {
            Some(ui::color::DARK_GREEN)
        } else {
            Some(ui::color::DARK_RED)
        }
    };
    for event in widget::TextBox::new(&project_editor.text_box_name)
        .border_color(super::DARK_A)
        .border(2.0)
        .down_from(ids.project_editor_add, 0.0)
        .align_left_of(ids.project_editor_add)
        .font_size(SMALL_FONT_SIZE)
        .and_then(color, |w, col| w.color(col))
        .w_of(area.id)
        .h(ITEM_HEIGHT)
        .set(ids.project_editor_name, ui)
    {
        use self::ui::widget::text_box::Event;
        match event {
            Event::Update(s) => project_editor.text_box_name = s,
            Event::Enter => {
                if project.name != project_editor.text_box_name {
                    let projects_directory = project::projects_directory(assets);
                    if is_name_valid(&projects_directory, &project_editor.text_box_name) {
                        let current_dir = projects_directory.join(slugify(&project.name));
                        let renamed_dir = projects_directory.join(slugify(&project_editor.text_box_name));
                        if let Err(err) = fs::rename(&current_dir, &renamed_dir) {
                            eprintln!(
                                "failed to rename \"{}\" to \"{}\": \"{}\"",
                                current_dir.display(),
                                renamed_dir.display(),
                                err,
                            );
                        } else {
                            project.name = project_editor.text_box_name.clone();
                        }
                    }
                }
            },
        }
    }

    area.id
}
