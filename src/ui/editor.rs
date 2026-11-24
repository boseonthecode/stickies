use std::cell::RefCell;
use std::rc::Rc;

use gtk4 as gtk;
use gtk4::prelude::*;
use libadwaita as adw;
use sourceview5 as source;
use sourceview5::prelude::*;

const MARKDOWN_LANGUAGE: &str = "markdown";
const LIGHT_SCHEME: &str = "Adwaita";
const DARK_SCHEME: &str = "Adwaita-dark";
const FALLBACK_SCHEME: &str = "classic";
const INDENT_WIDTH: i32 = 4;
const TAB_WIDTH: u32 = 4;

pub struct Editor {
    view: source::View,
    buffer: source::Buffer,
}

impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}

impl Editor {
    pub fn new() -> Self {
        let buffer = source::Buffer::new(None);
        configure_buffer(&buffer);

        let view = source::View::with_buffer(&buffer);
        configure_view(&view);
        follow_style_manager(&view, &buffer);

        Self { view, buffer }
    }

    pub fn view(&self) -> source::View {
        self.view.clone()
    }

    pub fn buffer(&self) -> source::Buffer {
        self.buffer.clone()
    }
}

fn configure_buffer(buffer: &source::Buffer) {
    if let Some(language) = source::LanguageManager::default().language(MARKDOWN_LANGUAGE) {
        buffer.set_language(Some(&language));
    }
    buffer.set_highlight_syntax(true);
    buffer.set_highlight_matching_brackets(true);
}

fn configure_view(view: &source::View) {
    view.set_wrap_mode(gtk::WrapMode::WordChar);
    view.set_highlight_current_line(true);
    view.set_monospace(true);
    view.set_auto_indent(true);
    view.set_indent_width(INDENT_WIDTH);
    view.set_tab_width(TAB_WIDTH);
    view.set_insert_spaces_instead_of_tabs(true);
    view.set_smart_backspace(true);
    view.set_top_margin(12);
    view.set_bottom_margin(12);
    view.set_left_margin(12);
    view.set_right_margin(12);
    let widget = view.upcast_ref::<gtk::Widget>();
    widget.update_property(&[gtk::accessible::Property::Label("Note content")]);
}

fn follow_style_manager(view: &source::View, buffer: &source::Buffer) {
    let manager = adw::StyleManager::default();
    apply_style_scheme(buffer, manager.is_dark());

    let weak_buffer = buffer.downgrade();
    let handler = manager.connect_dark_notify(move |manager| {
        if let Some(buffer) = weak_buffer.upgrade() {
            apply_style_scheme(&buffer, manager.is_dark());
        }
    });

    let handler = Rc::new(RefCell::new(Some(handler)));
    let disconnect_manager = manager.clone();
    view.connect_destroy(move |_| {
        if let Some(handler) = handler.borrow_mut().take() {
            disconnect_manager.disconnect(handler);
        }
    });
}

fn apply_style_scheme(buffer: &source::Buffer, dark: bool) {
    let manager = source::StyleSchemeManager::default();
    let scheme = manager
        .scheme(style_scheme_id(dark))
        .or_else(|| manager.scheme(FALLBACK_SCHEME));
    if let Some(scheme) = scheme {
        buffer.set_style_scheme(Some(&scheme));
    }
}

fn style_scheme_id(dark: bool) -> &'static str {
    if dark { DARK_SCHEME } else { LIGHT_SCHEME }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chooses_theme_matching_scheme_ids() {
        assert_eq!(style_scheme_id(false), LIGHT_SCHEME);
        assert_eq!(style_scheme_id(true), DARK_SCHEME);
    }
}
