//! The M0 counter app, written by hand against the guest SDK.

use plinth_guest::{Ui, button_role, prop, tone};
use std::cell::Cell;
use std::rc::Rc;

plinth_guest::app!(|ui: &mut Ui| {
    let screen = ui.screen("Counter", "number");
    let section = ui.section(screen, Some("Value"));
    let value = ui.heading(section, 1, "0");
    let parity = ui.text(section, "even");
    ui.set(parity, prop::TONE, plinth_guest::Value::Enum(tone::MUTED));

    let count = Rc::new(Cell::new(0i32));
    let show = move |ui: &mut Ui, n: i32| {
        ui.set_text(value, &n.to_string());
        ui.set_text(parity, if n % 2 == 0 { "even" } else { "odd" });
    };

    let actions = ui.section(screen, Some("Actions"));
    let (c, s) = (count.clone(), show.clone());
    ui.button(actions, "Increment", button_role::PRIMARY, move |ui| {
        c.set(c.get().wrapping_add(1));
        s(ui, c.get());
    });
    let (c, s) = (count.clone(), show.clone());
    ui.button(actions, "Decrement", button_role::DEFAULT, move |ui| {
        c.set(c.get().wrapping_sub(1));
        s(ui, c.get());
    });
    let (c, s) = (count, show);
    ui.button(actions, "Reset", button_role::DESTRUCTIVE, move |ui| {
        c.set(0);
        s(ui, 0);
    });

    ui.set_root(0, screen);
});
