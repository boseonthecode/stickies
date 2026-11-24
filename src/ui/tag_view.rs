use std::cell::RefCell;
use std::rc::Rc;

use gtk4 as gtk;
use gtk4::prelude::*;

use crate::storage::tags;

type AddHandler = Box<dyn Fn(Vec<String>)>;
type RemoveHandler = Box<dyn Fn(String)>;

#[derive(Default)]
struct Handlers {
    add: RefCell<Option<AddHandler>>,
    remove: RefCell<Option<RemoveHandler>>,
}

pub struct TagView {
    root: gtk::Box,
    flow: gtk::FlowBox,
    entry: gtk::Entry,
    tags: Rc<RefCell<Vec<String>>>,
    handlers: Rc<Handlers>,
}

impl Default for TagView {
    fn default() -> Self {
        Self::new()
    }
}

impl TagView {
    pub fn new() -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 6);
        root.set_margin_top(8);
        root.set_margin_bottom(8);
        root.set_margin_start(12);
        root.set_margin_end(12);

        let flow = gtk::FlowBox::new();
        flow.set_selection_mode(gtk::SelectionMode::None);
        flow.set_column_spacing(6);
        flow.set_row_spacing(6);
        flow.set_max_children_per_line(8);
        flow.set_halign(gtk::Align::Start);
        flow.set_hexpand(true);

        let entry = gtk::Entry::new();
        entry.set_placeholder_text(Some("Add tags (comma or space separated)"));
        entry.set_hexpand(true);
        entry.update_property(&[gtk::accessible::Property::Label("Tags")]);

        root.append(&flow);
        root.append(&entry);

        let tags = Rc::new(RefCell::new(Vec::new()));
        let handlers = Rc::new(Handlers::default());
        let weak_flow = flow.downgrade();

        let enter_tags = Rc::clone(&tags);
        let enter_handlers = Rc::clone(&handlers);
        entry.connect_activate(move |entry| {
            let parsed = tags::parse_tags(&entry.text());
            if parsed.is_empty() {
                return;
            }

            let mut added = Vec::new();
            {
                let mut current = enter_tags.borrow_mut();
                for tag in parsed {
                    if !tags::contains_tag(&current, &tag) {
                        current.push(tag.clone());
                        added.push(tag);
                    }
                }
            }

            if added.is_empty() {
                entry.set_text("");
                return;
            }

            if let Some(flow) = weak_flow.upgrade() {
                for tag in &added {
                    append_chip(&flow, tag, &enter_tags, &enter_handlers);
                }
            }
            if let Some(handler) = enter_handlers.add.borrow().as_ref() {
                handler(added);
            }
            entry.set_text("");
        });

        Self {
            root,
            flow,
            entry,
            tags,
            handlers,
        }
    }

    pub fn widget(&self) -> gtk::Box {
        self.root.clone()
    }

    pub fn set_tags(&self, tags: &[String]) {
        *self.tags.borrow_mut() = tags::normalize_tags(tags);
        self.render();
    }

    pub fn set_add_handler(&self, handler: impl Fn(Vec<String>) + 'static) {
        *self.handlers.add.borrow_mut() = Some(Box::new(handler));
    }

    pub fn set_remove_handler(&self, handler: impl Fn(String) + 'static) {
        *self.handlers.remove.borrow_mut() = Some(Box::new(handler));
    }

    fn render(&self) {
        self.flow.remove_all();

        for tag in self.tags.borrow().iter() {
            append_chip(&self.flow, tag, &self.tags, &self.handlers);
        }
    }

    pub fn entry(&self) -> &gtk::Entry {
        &self.entry
    }
}

fn append_chip(
    flow: &gtk::FlowBox,
    tag: &str,
    tags: &Rc<RefCell<Vec<String>>>,
    handlers: &Rc<Handlers>,
) {
    let button = gtk::Button::with_label(&format!("#{tag}"));
    button.add_css_class("pill");
    button.set_tooltip_text(Some(&format!("Remove #{tag}")));
    button.update_property(&[gtk::accessible::Property::Label(&format!(
        "Remove tag #{tag}"
    ))]);

    let chip_tag = tag.to_owned();
    let chip_tags = Rc::clone(tags);
    let chip_handlers = Rc::clone(handlers);
    let weak_flow = flow.downgrade();
    button.connect_clicked(move |button| {
        let removed = {
            let mut current = chip_tags.borrow_mut();
            tags::remove_tag(&mut current, &chip_tag)
        };
        if !removed {
            return;
        }

        if let Some(flow) = weak_flow.upgrade() {
            flow.remove(button);
        }
        if let Some(handler) = chip_handlers.remove.borrow().as_ref() {
            handler(chip_tag.clone());
        }
    });

    flow.append(&button);
}
