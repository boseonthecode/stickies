use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gdk4 as gdk;
use gdk4::prelude::*;
use gtk4 as gtk;
use gtk4::prelude::*;
use uuid::Uuid;

use crate::storage::attachments::AttachmentEntry;
use crate::storage::note::{AttachmentKind, AttachmentMeta};

const THUMBNAIL_WIDTH: i32 = 160;
const THUMBNAIL_HEIGHT: i32 = 120;
const THUMBNAIL_DISPLAY_WIDTH: i32 = 120;
const THUMBNAIL_DISPLAY_HEIGHT: i32 = 90;
const MAX_CONTENT_HEIGHT: i32 = 220;
const THUMBNAIL_CACHE_LIMIT: usize = 128;

type OpenHandler = Rc<dyn Fn(AttachmentEntry)>;
type RemoveHandler = Rc<dyn Fn(AttachmentMeta)>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentDisplay {
    pub title: String,
    pub details: String,
    pub icon_name: &'static str,
    pub thumbnail: bool,
    pub missing: bool,
}

pub fn describe(attachment: &AttachmentMeta, missing: bool) -> AttachmentDisplay {
    let image = attachment.kind == AttachmentKind::Image;
    let icon_name = if missing {
        "image-missing-symbolic"
    } else if image {
        "image-x-generic-symbolic"
    } else if attachment.mime.starts_with("text/") {
        "text-x-generic-symbolic"
    } else {
        "x-office-document-symbolic"
    };

    AttachmentDisplay {
        title: attachment.original_name.clone(),
        details: if missing {
            "Missing file".to_owned()
        } else {
            format_size(attachment.size_bytes)
        },
        icon_name,
        thumbnail: image && !missing,
        missing,
    }
}

pub fn format_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;

    if bytes < 1024 {
        format!("{bytes} B")
    } else if (bytes as f64) < MB {
        format!("{:.1} KB", bytes as f64 / KB)
    } else if (bytes as f64) < GB {
        format!("{:.1} MB", bytes as f64 / MB)
    } else {
        format!("{:.1} GB", bytes as f64 / GB)
    }
}

#[derive(Clone)]
pub struct AttachmentView {
    container: gtk::Box,
    items: gtk::Box,
    scroll: gtk::ScrolledWindow,
    placeholder: gtk::Label,
    thumbnails: Rc<RefCell<HashMap<Uuid, gdk::Texture>>>,
    open_handler: Rc<RefCell<Option<OpenHandler>>>,
    remove_handler: Rc<RefCell<Option<RemoveHandler>>>,
}

impl Default for AttachmentView {
    fn default() -> Self {
        Self::new()
    }
}

impl AttachmentView {
    pub fn new() -> Self {
        let title = gtk::Label::new(Some("Attachments"));
        title.set_halign(gtk::Align::Start);
        title.add_css_class("heading");

        let placeholder = gtk::Label::new(Some("Dropped files and pasted images appear here"));
        placeholder.set_halign(gtk::Align::Start);
        placeholder.add_css_class("dim-label");

        let items = gtk::Box::new(gtk::Orientation::Vertical, 4);
        items.set_margin_top(4);
        items.set_margin_bottom(4);
        items.set_margin_start(6);
        items.set_margin_end(6);

        let scroll = gtk::ScrolledWindow::new();
        scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroll.set_max_content_height(MAX_CONTENT_HEIGHT);
        scroll.set_propagate_natural_height(true);
        scroll.set_child(Some(&items));
        scroll.set_visible(false);

        let container = gtk::Box::new(gtk::Orientation::Vertical, 6);
        container.set_margin_top(6);
        container.set_margin_bottom(12);
        container.set_margin_start(12);
        container.set_margin_end(12);
        container.append(&title);
        container.append(&placeholder);
        container.append(&scroll);

        Self {
            container,
            items,
            scroll,
            placeholder,
            thumbnails: Rc::new(RefCell::new(HashMap::new())),
            open_handler: Rc::new(RefCell::new(None)),
            remove_handler: Rc::new(RefCell::new(None)),
        }
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.container
    }

    pub fn set_open_handler(&self, handler: impl Fn(AttachmentEntry) + 'static) {
        *self.open_handler.borrow_mut() = Some(Rc::new(handler));
    }

    pub fn set_remove_handler(&self, handler: impl Fn(AttachmentMeta) + 'static) {
        *self.remove_handler.borrow_mut() = Some(Rc::new(handler));
    }

    pub fn refresh(&self, entries: &[AttachmentEntry]) {
        while let Some(child) = self.items.first_child() {
            self.items.remove(&child);
        }

        self.placeholder.set_visible(entries.is_empty());
        self.scroll.set_visible(!entries.is_empty());

        for entry in entries {
            self.items.append(&self.build_row(entry));
        }
    }

    fn build_row(&self, entry: &AttachmentEntry) -> gtk::Box {
        let display = describe(&entry.attachment, entry.missing);
        let visual = if display.thumbnail {
            self.thumbnail(entry)
                .map(|texture| {
                    let picture = gtk::Picture::for_paintable(&texture);
                    picture.set_content_fit(gtk::ContentFit::Contain);
                    picture.set_can_shrink(true);
                    picture.set_size_request(THUMBNAIL_DISPLAY_WIDTH, THUMBNAIL_DISPLAY_HEIGHT);
                    picture.upcast::<gtk::Widget>()
                })
                .unwrap_or_else(|| icon_widget(display.icon_name))
        } else {
            icon_widget(display.icon_name)
        };

        let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
        text.set_hexpand(true);
        text.set_valign(gtk::Align::Center);

        let title = gtk::Label::new(Some(&display.title));
        title.set_xalign(0.0);
        title.set_hexpand(true);
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);

        let details = gtk::Label::new(Some(&display.details));
        details.set_xalign(0.0);
        details.add_css_class("dim-label");
        if display.missing {
            details.add_css_class("error");
        }

        text.append(&title);
        text.append(&details);

        let content = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        content.append(&visual);
        content.append(&text);

        let open_button = gtk::Button::new();
        open_button.add_css_class("flat");
        open_button.set_hexpand(true);
        open_button.set_tooltip_text(Some(&format!("Open {}", display.title)));
        open_button.update_property(&[gtk::accessible::Property::Label(&format!(
            "Open {}",
            display.title
        ))]);
        open_button.set_child(Some(&content));

        let open_handler = self.open_handler.borrow().clone();
        let open_entry = entry.clone();
        open_button.connect_clicked(move |_| {
            if let Some(handler) = &open_handler {
                handler(open_entry.clone());
            }
        });

        let remove_button = gtk::Button::from_icon_name("user-trash-symbolic");
        remove_button.add_css_class("flat");
        remove_button.set_valign(gtk::Align::Center);
        remove_button.set_tooltip_text(Some(&format!("Remove {}", display.title)));
        remove_button.update_property(&[gtk::accessible::Property::Label(&format!(
            "Remove {}",
            display.title
        ))]);

        let remove_handler = self.remove_handler.borrow().clone();
        let remove_attachment = entry.attachment.clone();
        remove_button.connect_clicked(move |_| {
            if let Some(handler) = &remove_handler {
                handler(remove_attachment.clone());
            }
        });

        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        row.append(&open_button);
        row.append(&remove_button);
        row
    }

    fn thumbnail(&self, entry: &AttachmentEntry) -> Option<gdk::Texture> {
        let mut cache = self.thumbnails.borrow_mut();

        if let Some(texture) = cache.get(&entry.attachment.id) {
            return Some(texture.clone());
        }

        let path = entry.path.as_ref()?;
        let pixbuf = gdk::gdk_pixbuf::Pixbuf::from_file_at_scale(
            path,
            THUMBNAIL_WIDTH,
            THUMBNAIL_HEIGHT,
            true,
        )
        .ok()?;
        let texture = gdk::Texture::for_pixbuf(&pixbuf);

        if cache.len() >= THUMBNAIL_CACHE_LIMIT {
            cache.clear();
        }
        cache.insert(entry.attachment.id, texture.clone());

        Some(texture)
    }
}

fn icon_widget(icon_name: &str) -> gtk::Widget {
    let image = gtk::Image::from_icon_name(icon_name);
    image.set_pixel_size(32);
    image.set_size_request(THUMBNAIL_DISPLAY_WIDTH, THUMBNAIL_DISPLAY_HEIGHT);
    image.upcast()
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;

    fn attachment(kind: AttachmentKind, mime: &str, size_bytes: u64) -> AttachmentMeta {
        AttachmentMeta {
            id: Uuid::now_v7(),
            stored_name: "stored".to_owned(),
            original_name: "report.pdf".to_owned(),
            mime: mime.to_owned(),
            size_bytes,
            kind,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn formats_sizes_for_display() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(1023), "1023 B");
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(1024 * 1024), "1.0 MB");
        assert_eq!(format_size(100 * 1024 * 1024), "100.0 MB");
    }

    #[test]
    fn describes_images_and_document_attachments() {
        let image = attachment(AttachmentKind::Image, "image/png", 2048);
        let image_view = describe(&image, false);

        assert_eq!(image_view.title, "report.pdf");
        assert_eq!(image_view.details, "2.0 KB");
        assert!(image_view.thumbnail);
        assert!(!image_view.missing);

        let document = attachment(AttachmentKind::File, "application/pdf", 4096);
        let document_view = describe(&document, false);

        assert_eq!(document_view.icon_name, "x-office-document-symbolic");
        assert!(!document_view.thumbnail);

        let text = attachment(AttachmentKind::File, "text/plain", 12);
        assert_eq!(describe(&text, false).icon_name, "text-x-generic-symbolic");
    }

    #[test]
    fn describes_missing_attachments() {
        let image = attachment(AttachmentKind::Image, "image/png", 2048);
        let view = describe(&image, true);

        assert!(view.missing);
        assert!(!view.thumbnail);
        assert_eq!(view.details, "Missing file");
        assert_eq!(view.icon_name, "image-missing-symbolic");
    }
}
