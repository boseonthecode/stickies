use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;

pub const WIDGET_NAME: &str = "browser-window";

pub fn present(app: &adw::Application) {
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Stickies")
        .default_width(820)
        .default_height(560)
        .build();
    window.set_widget_name(WIDGET_NAME);

    let search_entry = gtk::SearchEntry::builder()
        .placeholder_text("Search notes")
        .width_chars(32)
        .build();

    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&search_entry));

    let list_page = placeholder_page("Notes will appear here");
    let grid_page = placeholder_page("Note cards will appear here");

    let stack = gtk::Stack::new();
    stack.add_titled(&list_page, Some("list"), "List");
    stack.add_titled(&grid_page, Some("grid"), "Grid");
    stack.set_visible_child_name("list");

    let switcher = gtk::StackSwitcher::builder().stack(&stack).build();
    header.pack_end(&switcher);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&stack));

    window.set_content(Some(&toolbar));
    window.present();
    search_entry.grab_focus();
}

fn placeholder_page(text: &str) -> gtk::Widget {
    let label = gtk::Label::new(Some(text));
    label.add_css_class("dim-label");
    label.set_halign(gtk::Align::Center);
    label.set_valign(gtk::Align::Center);
    label.set_hexpand(true);
    label.set_vexpand(true);
    label.upcast()
}
