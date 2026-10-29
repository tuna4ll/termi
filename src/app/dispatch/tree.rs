//! The file tree panel: opening it, moving through it, and creating,
//! renaming, copying, moving and deleting what it shows.

use super::enter_mode;
use crate::app::mode::Mode;
use crate::app::state::App;

pub(super) fn toggle_tree(app: &mut App) {
    app.tree_visible = !app.tree_visible;
    if !app.tree_visible {
        enter_mode(app, Mode::Normal);
        return;
    }
    if app.tree.is_none() {
        let root = app.tree_root();
        app.tree = Some(crate::filesystem::tree::Tree::new(root));
        app.refresh_tree();
    } else {
        // Opening the panel is the natural way to ask "what is in there now?",
        // so it re-reads even when the watcher is off or never saw the change.
        app.refresh_tree();
    }
    enter_mode(app, Mode::Tree);
}

pub(super) fn move_tree_selection(app: &mut App, delta: isize) {
    let Some(tree) = app.tree.as_ref() else {
        return;
    };
    let last = tree.entries().len().saturating_sub(1);
    app.tree_selected = match delta {
        isize::MIN => 0,
        isize::MAX => last,
        delta if delta < 0 => app.tree_selected.saturating_sub(delta.unsigned_abs()),
        delta => app
            .tree_selected
            .saturating_add(delta.unsigned_abs())
            .min(last),
    };
}

pub(super) fn activate_tree_entry(app: &mut App) {
    let Some(tree) = app.tree.as_mut() else {
        return;
    };
    let Some(path) = tree.activate(app.tree_selected) else {
        let last = app
            .tree
            .as_ref()
            .map_or(0, |t| t.entries().len().saturating_sub(1));
        app.tree_selected = app.tree_selected.min(last);
        return;
    };
    match app.open(path) {
        Ok(()) => {
            app.tree_visible = false;
            enter_mode(app, Mode::Normal);
        }
        Err(error) => app.error(error.to_string()),
    }
}

pub(super) fn begin_tree_creation(app: &mut App, directory: bool) {
    let Some(tree) = app.tree.as_ref() else {
        return;
    };
    let parent = tree.creation_directory(app.tree_selected);
    let command = if directory { "mkdir" } else { "touch" };
    let separator = std::path::MAIN_SEPARATOR;
    let mut prefix = parent.as_os_str().to_string_lossy().into_owned();
    if !prefix.ends_with(separator) {
        prefix.push(separator);
    }

    enter_mode(app, Mode::Command);
    app.command_line = format!("{command} {prefix}");
}

pub(super) fn begin_tree_path(app: &mut App, command: &str, keep_name: bool) {
    let Some(path) = app
        .tree
        .as_ref()
        .and_then(|tree| tree.path_at(app.tree_selected))
        .map(std::path::Path::to_path_buf)
    else {
        return;
    };
    let destination = if keep_name {
        path.clone()
    } else {
        path.parent()
            .map(std::path::Path::to_path_buf)
            .unwrap_or_default()
    };
    app.tree_source = Some(path);
    enter_mode(app, Mode::Command);
    let separator = std::path::MAIN_SEPARATOR;
    let mut destination = destination.as_os_str().to_string_lossy().into_owned();
    if !keep_name && !destination.ends_with(separator) {
        destination.push(separator);
    }
    app.command_line = format!("{command} {destination}");
}

pub(super) fn begin_tree_delete(app: &mut App) {
    let Some(path) = app
        .tree
        .as_ref()
        .and_then(|tree| tree.path_at(app.tree_selected))
        .map(std::path::Path::to_path_buf)
    else {
        return;
    };
    if app.has_dirty_under(&path) {
        app.error(format!(
            "save or close modified files before deleting {}",
            path.display()
        ));
        return;
    }
    let kind = if path.is_dir() { "directory" } else { "file" };
    app.pending_delete = Some(path.clone());
    app.popup = Some((
        format!("delete {kind} permanently?"),
        format!("{}\n\nThis cannot be undone.", path.display()),
    ));
    app.mode = Mode::Confirm;
}

pub(super) fn cancel_tree_delete(app: &mut App) {
    app.pending_delete = None;
    app.mode = if app.tree_visible {
        Mode::Tree
    } else {
        Mode::Normal
    };
}

pub(super) fn delete_tree_path(app: &mut App) {
    let Some(path) = app.pending_delete.take() else {
        return cancel_tree_delete(app);
    };
    if app.has_dirty_under(&path) {
        app.error(format!(
            "save or close modified files before deleting {}",
            path.display()
        ));
        return cancel_tree_delete(app);
    }
    match crate::filesystem::delete_path(&path) {
        Ok(()) => {
            app.close_buffers_under(&path);
            app.refresh_tree();
            app.info(format!("deleted {}", path.display()));
        }
        Err(error) => app.error(error.to_string()),
    }
    cancel_tree_delete(app);
}
