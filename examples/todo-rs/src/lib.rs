//! The M0 todo app, written by hand against the guest SDK.
//!
//! It has two primary screens, so the host shows its navigation shell.

use plinth_guest::{NodeId, Ui, Value, button_role, prop, tone};
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone)]
struct Task {
    id: u32,
    title: String,
    done: bool,
}

struct Model {
    tasks: Vec<Task>,
    next_id: u32,
    draft: String,
    show_completed: bool,
}

/// Node ids of the parts of the view that change.
struct View {
    field: NodeId,
    section: NodeId,
    list: NodeId,
    summary: NodeId,
    /// The Empty control. It exists only while the list has no rows.
    empty: Option<NodeId>,
    rows: Vec<(u32, NodeId)>,
}

struct App {
    model: RefCell<Model>,
    view: RefCell<View>,
}

type Shared = Rc<App>;

fn add_task(ui: &mut Ui, app: &Shared) {
    {
        let mut m = app.model.borrow_mut();
        let title = m.draft.trim().to_owned();
        if title.is_empty() {
            return;
        }
        let id = m.next_id;
        m.next_id += 1;
        m.tasks.push(Task { id, title, done: false });
        m.draft.clear();
    }
    // Clear the field. The host shows the new value at once.
    let field = app.view.borrow().field;
    ui.set(field, prop::VALUE, "");
    refresh(ui, app);
}

fn refresh(ui: &mut Ui, app: &Shared) {
    let (visible, remaining, total) = {
        let m = app.model.borrow();
        let visible: Vec<Task> = m.tasks.iter().filter(|t| m.show_completed || !t.done).cloned().collect();
        (visible, m.tasks.iter().filter(|t| !t.done).count(), m.tasks.len())
    };

    let mut view = app.view.borrow_mut();
    let view = &mut *view;
    ui.reconcile(view.list, &mut view.rows, &visible, |t| t.id, |ui, t| make_row(ui, app, t), update_row);

    let empty_title = if total == 0 { "No tasks yet" } else { "Nothing to show" };
    match (visible.is_empty(), view.empty) {
        (true, None) => view.empty = Some(ui.empty(view.section, empty_title, "Add a task above.")),
        (true, Some(empty)) => ui.set(empty, prop::TITLE, empty_title),
        (false, Some(empty)) => {
            ui.remove(empty);
            view.empty = None;
        }
        (false, None) => {}
    }
    ui.set_text(view.summary, &format!("{remaining} of {total} remaining"));
}

fn make_row(ui: &mut Ui, app: &Shared, t: &Task) -> NodeId {
    let row = ui.row(0, &t.title);
    let task_id = t.id;
    let a = app.clone();
    ui.toggle(row, "Done", t.done, move |ui, done| {
        if let Some(task) = a.model.borrow_mut().tasks.iter_mut().find(|x| x.id == task_id) {
            task.done = done;
        }
        refresh(ui, &a);
    });
    let a = app.clone();
    ui.button(row, "Delete", button_role::DESTRUCTIVE, move |ui| {
        a.model.borrow_mut().tasks.retain(|x| x.id != task_id);
        refresh(ui, &a);
    });
    update_row(ui, row, t);
    row
}

fn update_row(ui: &mut Ui, row: NodeId, t: &Task) {
    ui.set(row, prop::TITLE, t.title.as_str());
    ui.set(row, prop::SUBTITLE, if t.done { "Done" } else { "Open" });
    let toggle = ui.children(row)[0];
    ui.set(toggle, prop::VALUE, t.done);
}

plinth_guest::app!(|ui: &mut Ui| {
    // -- Tasks screen ------------------------------------------------------
    let tasks = ui.screen("Tasks", "list");

    let new_section = ui.section(tasks, Some("New task"));
    let field = ui.text_field(new_section, "Task", "What needs to be done?", |_, _| {});
    let add = ui.button(new_section, "Add task", button_role::PRIMARY, |_| {});

    let list_section = ui.section(tasks, Some("Tasks"));
    let summary = ui.text(list_section, "0 of 0 remaining");
    ui.set(summary, prop::TONE, Value::Enum(tone::MUTED));
    let list = ui.list(list_section);

    // -- Settings screen ---------------------------------------------------
    let settings = ui.screen("Settings", "gear");
    let display = ui.section(settings, Some("Display"));
    let show_toggle = ui.toggle(display, "Show completed tasks", true, |_, _| {});
    let data = ui.section(settings, Some("Data"));
    let clear = ui.button(data, "Clear completed tasks", button_role::DESTRUCTIVE, |_| {});
    let about = ui.section(settings, Some("About"));
    ui.text(about, "A hand-written M0 guest. The Plinth TS compiler replaces it in M1.");

    let app: Shared = Rc::new(App {
        model: RefCell::new(Model { tasks: Vec::new(), next_id: 1, draft: String::new(), show_completed: true }),
        view: RefCell::new(View { field, section: list_section, list, summary, empty: None, rows: Vec::new() }),
    });

    // Handlers need the shared app, so they are bound after it exists.
    let a = app.clone();
    ui.on(field, plinth_guest::event::CHANGE, move |_, v| {
        a.model.borrow_mut().draft = v.as_str().unwrap_or_default().to_owned();
    });
    let a = app.clone();
    ui.on(field, plinth_guest::event::SUBMIT, move |ui, _| add_task(ui, &a));
    let a = app.clone();
    ui.on(add, plinth_guest::event::PRESS, move |ui, _| add_task(ui, &a));
    let a = app.clone();
    ui.on(show_toggle, plinth_guest::event::CHANGE, move |ui, v| {
        a.model.borrow_mut().show_completed = v.as_bool().unwrap_or(true);
        refresh(ui, &a);
    });
    let a = app.clone();
    ui.on(clear, plinth_guest::event::PRESS, move |ui, _| {
        a.model.borrow_mut().tasks.retain(|t| !t.done);
        refresh(ui, &a);
    });

    refresh(ui, &app);
    for title in ["Read SPEC.md", "Build the M0 host", "Write the compiler"] {
        app.model.borrow_mut().draft = title.to_owned();
        add_task(ui, &app);
    }

    ui.set_root(0, tasks);
    ui.set_root(1, settings);
});
