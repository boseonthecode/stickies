use gdk4 as gdk;
use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;

pub const WIDGET_NAME: &str = "note-window";

pub fn present(app: &adw::Application, number: u64) {
    let title = format!("Note {number}");
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title(&title)
        .default_width(440)
        .default_height(360)
        .build();
    window.set_widget_name(WIDGET_NAME);

    let title_label = gtk::Label::new(Some(&title));
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&title_label));

    let editor = gtk::TextView::builder()
        .wrap_mode(gtk::WrapMode::WordChar)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();
    editor.add_css_class("monospace");

    let editor_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&editor)
        .vexpand(true)
        .build();

    let attachments_title = gtk::Label::new(Some("Attachments"));
    attachments_title.set_halign(gtk::Align::Start);
    attachments_title.add_css_class("heading");

    let attachments_placeholder =
        gtk::Label::new(Some("Dropped files and pasted images appear here"));
    attachments_placeholder.set_halign(gtk::Align::Start);
    attachments_placeholder.add_css_class("dim-label");

    let attachments = gtk::Box::new(gtk::Orientation::Vertical, 6);
    attachments.set_margin_top(6);
    attachments.set_margin_bottom(12);
    attachments.set_margin_start(12);
    attachments.set_margin_end(12);
    attachments.append(&attachments_title);
    attachments.append(&attachments_placeholder);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&editor_scroll);
    content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    content.append(&attachments);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&content));

    window.set_content(Some(&toolbar));
    window.connect_close_request(|_| glib::Propagation::Proceed);

    let key_controller = gtk::EventControllerKey::new();
    let window_weak = window.downgrade();
    key_controller.connect_key_pressed(move |_, key, _, _| {
        if key == gdk::Key::Escape {
            if let Some(window) = window_weak.upgrade() {
                window.close();
            }
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    window.add_controller(key_controller);

    window.present();
    editor.grab_focus();
}
