use std::cell::RefCell;
use std::collections::HashMap;

use chrono::{DateTime, Local, Utc};
use gdk4 as gdk;
use gdk4::prelude::*;
use gtk4 as gtk;
use gtk4::prelude::*;
use uuid::Uuid;

use crate::storage::attachments::{self, AttachmentEntry};
use crate::storage::index::NoteSummary;
use crate::storage::note::{AttachmentKind, Note, NoteStore};
use crate::storage::tags;
use crate::util::text::note_title;

const CARD_WIDTH: i32 = 210;
const THUMBNAIL_WIDTH: i32 = 320;
const THUMBNAIL_HEIGHT: i32 = 180;
const THUMBNAIL_DISPLAY_HEIGHT: i32 = 118;
const CACHE_LIMIT: usize = 256;

#[derive(Default)]
pub struct ThumbnailCache {
    cache: RefCell<HashMap<Uuid, Option<gdk::Texture>>>,
}

impl ThumbnailCache {
    pub fn thumbnail(&self, entry: &AttachmentEntry) -> Option<gdk::Texture> {
        if let Some(cached) = self.cache.borrow().get(&entry.attachment.id) {
            return cached.clone();
        }

        let texture = if entry.missing {
            None
        } else {
            entry.path.as_ref().and_then(|path| {
                gdk::gdk_pixbuf::Pixbuf::from_file_at_scale(
                    path,
                    THUMBNAIL_WIDTH,
                    THUMBNAIL_HEIGHT,
                    true,
                )
                .ok()
                .map(|pixbuf| gdk::Texture::for_pixbuf(&pixbuf))
            })
        };

        let mut cache = self.cache.borrow_mut();
        if cache.len() >= CACHE_LIMIT {
            cache.clear();
        }
        cache.insert(entry.attachment.id, texture.clone());

        texture
    }
}

pub fn build(note: &Note, store: &NoteStore, thumbnails: &ThumbnailCache) -> gtk::Widget {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
    card.set_size_request(CARD_WIDTH, -1);
    card.set_hexpand(true);
    card.add_css_class("card");

    let first_image = note
        .meta
        .attachments
        .iter()
        .find(|attachment| attachment.kind == AttachmentKind::Image);
    let visual = first_image
        .and_then(|attachment| {
            let entry = attachments::attachment_entry(store.paths(), note.meta.id, attachment);
            thumbnails.thumbnail(&entry)
        })
        .map(|texture| {
            let picture = gtk::Picture::for_paintable(&texture);
            picture.set_content_fit(gtk::ContentFit::Cover);
            picture.set_can_shrink(true);
            picture.set_size_request(-1, THUMBNAIL_DISPLAY_HEIGHT);
            picture.set_hexpand(true);
            picture.upcast::<gtk::Widget>()
        })
        .unwrap_or_else(placeholder_visual);

    let badges = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    badges.set_valign(gtk::Align::Center);
    if note.meta.pinned {
        let icon = gtk::Image::from_icon_name("view-pin-symbolic");
        icon.set_tooltip_text(Some("Pinned"));
        badges.append(&icon);
    }
    if !note.meta.attachments.is_empty() {
        let icon = gtk::Image::from_icon_name("mail-attachment-symbolic");
        icon.set_tooltip_text(Some("Has attachments"));
        badges.append(&icon);
    }

    let title = gtk::Label::new(Some(&note_title(&note.body)));
    title.set_xalign(0.0);
    title.set_hexpand(true);
    title.set_lines(2);
    title.set_wrap(true);
    title.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);

    let subtitle = gtk::Label::new(Some(&note_subtitle(note)));
    subtitle.set_xalign(0.0);
    subtitle.set_hexpand(true);
    subtitle.set_lines(2);
    subtitle.set_wrap(true);
    subtitle.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    subtitle.set_ellipsize(gtk::pango::EllipsizeMode::End);
    subtitle.add_css_class("dim-label");

    let text = gtk::Box::new(gtk::Orientation::Vertical, 4);
    text.set_margin_top(4);
    text.set_margin_bottom(8);
    text.set_margin_start(8);
    text.set_margin_end(8);
    if badges.first_child().is_some() {
        text.append(&badges);
    }
    text.append(&title);
    text.append(&subtitle);

    card.append(&visual);
    card.append(&text);
    card.upcast()
}

pub(crate) fn note_subtitle(note: &Note) -> String {
    subtitle(
        &note.meta.tags,
        note.meta.trashed,
        note.meta.trashed_at,
        note.meta.modified_at,
    )
}

pub(crate) fn summary_subtitle(summary: &NoteSummary) -> String {
    subtitle(
        &summary.tags,
        summary.trashed,
        summary.trashed_at,
        summary.modified_at,
    )
}

fn subtitle(
    tags: &[String],
    trashed: bool,
    trashed_at: Option<DateTime<Utc>>,
    modified_at: DateTime<Utc>,
) -> String {
    let mut parts = Vec::new();

    let note_tags = tags::normalize_tags(tags);
    if !note_tags.is_empty() {
        parts.push(
            note_tags
                .iter()
                .map(|tag| format!("#{tag}"))
                .collect::<Vec<_>>()
                .join(" "),
        );
    }

    let (label, time) = if trashed {
        ("Trashed", trashed_at.unwrap_or(modified_at))
    } else {
        ("Modified", modified_at)
    };
    parts.push(format!("{label} {}", format_time(time)));

    parts.join("  ·  ")
}

fn format_time(time: DateTime<Utc>) -> String {
    time.with_timezone(&Local)
        .format("%Y-%m-%d %H:%M")
        .to_string()
}

fn placeholder_visual() -> gtk::Widget {
    let holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
    holder.set_size_request(-1, THUMBNAIL_DISPLAY_HEIGHT);
    holder.set_hexpand(true);
    holder.set_valign(gtk::Align::Fill);
    holder.add_css_class("dim-label");

    let icon = gtk::Image::from_icon_name("text-x-generic-symbolic");
    icon.set_pixel_size(48);
    icon.set_halign(gtk::Align::Center);
    icon.set_valign(gtk::Align::Center);
    icon.set_vexpand(true);
    holder.append(&icon);

    holder.upcast()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subtitles_include_normalized_tags() {
        let mut note = Note::new();
        note.meta.tags = vec![" Work ".to_owned(), "work".to_owned(), "#idea".to_owned()];

        let subtitle = note_subtitle(&note);

        assert!(subtitle.starts_with("#Work #idea"));
        assert!(subtitle.contains("Modified"));
    }

    #[test]
    fn summary_subtitles_include_normalized_tags() {
        let mut note = Note::new();
        note.meta.tags = vec![" Work ".to_owned(), "work".to_owned(), "#idea".to_owned()];

        let subtitle = summary_subtitle(&NoteSummary::from_note(&note));

        assert!(subtitle.starts_with("#Work #idea"));
        assert!(subtitle.contains("Modified"));
    }
}
