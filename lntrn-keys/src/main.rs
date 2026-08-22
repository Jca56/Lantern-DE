//! `lkeys` — terminal UI for the Lantern keychain (libsecret client).

use std::process::ExitCode;

mod actions;
mod input;
mod secret;
mod state;
mod term;
mod ui;

use input::Key;
use secret::Client;
use state::{AddStage, Mode, State};

fn main() -> ExitCode {
    let mut client = match Client::connect() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Cannot connect to org.freedesktop.secrets: {e}");
            eprintln!("Is lntrn-keychain running and unlocked?");
            return ExitCode::from(1);
        }
    };

    let items = match client.list_items() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("Failed to list items: {e}");
            return ExitCode::from(2);
        }
    };

    let mut state = State::new(items);
    let mut term = match ui::Term::new() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("Cannot enter raw mode: {e}");
            return ExitCode::from(3);
        }
    };

    loop {
        let size = term.size();
        let frame = ui::render(&state, size);
        term.write_all(&frame);
        term.flush();

        let key = match input::read_key() {
            Ok(k) => k,
            Err(_) => break,
        };
        if !handle_key(key, &mut state, &mut client) {
            break;
        }
    }
    drop(term);
    ExitCode::SUCCESS
}

/// Returns false when the user wants to quit.
fn handle_key(key: Key, state: &mut State, client: &mut Client) -> bool {
    // Modal modes intercept first.
    match state.mode.clone() {
        Mode::Revealing(_) => {
            state.mode = Mode::List;
            return true;
        }
        Mode::ConfirmDelete => {
            match key {
                Key::Char('y') | Key::Char('Y') => {
                    if actions::confirm_delete(state, client) {
                        refresh_items(state, client);
                    }
                }
                _ => state.set_status("Cancelled."),
            }
            state.mode = Mode::List;
            return true;
        }
        Mode::Filtering => {
            match key {
                Key::Enter | Key::Esc => state.mode = Mode::List,
                Key::Backspace => {
                    state.filter.pop();
                    state.cursor = 0;
                }
                Key::Char(c) => {
                    state.filter.push(c);
                    state.cursor = 0;
                }
                Key::Paste(s) => {
                    for c in s.chars().filter(|c| !c.is_control()) {
                        state.filter.push(c);
                    }
                    state.cursor = 0;
                }
                _ => {}
            }
            return true;
        }
        Mode::Adding(stage) => {
            return handle_add(stage, key, state, client);
        }
        Mode::List => {}
    }

    match key {
        Key::Char('q') | Key::CtrlC | Key::CtrlD => return false,
        Key::Char('a') => actions::start_add(state),
        Key::Char('d') => {
            if state.selected().is_some() {
                state.mode = Mode::ConfirmDelete;
            }
        }
        Key::Char('/') => {
            state.mode = Mode::Filtering;
            state.filter.clear();
            state.cursor = 0;
        }
        Key::Char(' ') => actions::reveal_selected(state, client),
        Key::Enter => actions::copy_selected(state, client),
        Key::Up | Key::Char('k') => state.move_cursor(-1),
        Key::Down | Key::Char('j') => state.move_cursor(1),
        Key::PageUp => state.move_cursor(-10),
        Key::PageDown => state.move_cursor(10),
        Key::Home => state.cursor = 0,
        Key::End => {
            let n = state.visible().len();
            state.cursor = n.saturating_sub(1);
        }
        Key::Char('r') => refresh_items(state, client),
        _ => {}
    }
    true
}

fn handle_add(stage: AddStage, key: Key, state: &mut State, client: &mut Client) -> bool {
    let mut new_stage = stage.clone();
    match key {
        Key::Esc => {
            state.mode = Mode::List;
            state.set_status("Add cancelled.");
            return true;
        }
        Key::Backspace => match &mut new_stage {
            AddStage::Name(s) | AddStage::Secret { value: s, .. } => {
                s.pop();
            }
        },
        Key::Char(c) => match &mut new_stage {
            AddStage::Name(s) | AddStage::Secret { value: s, .. } => {
                s.push(c);
            }
        },
        Key::Paste(s) => {
            // Strip trailing whitespace (most clipboard managers add a \n).
            // For multi-line input (e.g. an SSH private key), keep newlines
            // in the secret stage but drop them in the name stage.
            let trimmed = s.trim_end();
            match &mut new_stage {
                AddStage::Name(field) => {
                    for c in trimmed.chars().filter(|c| !c.is_control()) {
                        field.push(c);
                    }
                }
                AddStage::Secret { value, .. } => {
                    // Allow embedded \n for multi-line secrets; just strip the
                    // trailing one that turned into Enter trouble before.
                    value.push_str(trimmed);
                }
            }
        }
        Key::Enter => {
            new_stage = advance_add(new_stage, state, client);
            if matches!(state.mode, Mode::List) {
                return true;
            }
        }
        _ => {}
    }
    state.mode = Mode::Adding(new_stage);
    true
}

fn advance_add(stage: AddStage, state: &mut State, client: &mut Client) -> AddStage {
    match stage {
        AddStage::Name(name) => {
            if name.trim().is_empty() {
                state.set_status("Name can't be empty.");
                return AddStage::Name(name);
            }
            AddStage::Secret {
                name,
                value: String::new(),
            }
        }
        AddStage::Secret { name, value } => {
            if value.is_empty() {
                state.set_status("Secret can't be empty.");
                return AddStage::Secret { name, value };
            }
            let ok = actions::finish_add(state, client, name, value);
            state.mode = Mode::List;
            if ok {
                refresh_items(state, client);
            }
            AddStage::Name(String::new())
        }
    }
}

fn refresh_items(state: &mut State, client: &mut Client) {
    match client.list_items() {
        Ok(items) => {
            state.all = items;
            let n = state.visible().len();
            state.cursor = state.cursor.min(n.saturating_sub(1));
        }
        Err(e) => state.set_status(format!("Refresh failed: {e}")),
    }
}
