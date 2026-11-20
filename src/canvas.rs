//! Drawing slides (the big one being edited, the thumbnails, the one being
//! presented) and editing on the big one: selecting, moving and resizing
//! elements, drawing new ones, typing into text boxes.

use crate::App;
use crate::deck::{Align, Element, Kind, Slide, VAlign, H, W};
use atelier::egui::{
    self, Color32, FontId, Pos2, Rect, Sense, Stroke, TextureHandle, Ui, pos2,
    text::{LayoutJob, TextFormat},
    vec2,
};
use atelier::{fonts, theme};
use std::collections::HashMap;

pub const PAD: f32 = 10.0;

#[derive(Clone, Copy)]
pub enum Drag {
    Move { id: u64, from: Pos2, orig: (f32, f32) },
    Resize { id: u64, handle: usize, orig: (f32, f32, f32, f32), flip: bool },
    Create { kind: Kind, from: Pos2, id: Option<u64> },
}

fn c32(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

/// A slide's text laid out for drawing at a scale.
pub fn text_job(e: &Element, scale: f32) -> LayoutJob {
    let s = &e.style;
    let size = s.size * 4.0 / 3.0 * scale;
    let mut f = TextFormat {
        font_id: FontId::new(size.max(1.0), fonts::family(&s.font, s.bold)),
        color: c32(s.color),
        italics: s.italic,
        line_height: Some(size * 1.2),
        ..Default::default()
    };
    if s.underline {
        f.underline = Stroke::new((size / 14.0).max(1.0), c32(s.color));
    }
    let text = if s.bullets {
        e.text.lines().map(|l| if l.trim().is_empty() { l.to_owned() } else { format!("•  {l}") }).collect::<Vec<_>>().join("\n")
    } else {
        e.text.clone()
    };
    let mut job = LayoutJob::single_section(text, f);
    job.wrap.max_width = ((e.w - 2.0 * PAD) * scale).max(1.0);
    job.halign = match s.align {
        Align::Left => egui::Align::Min,
        Align::Center => egui::Align::Center,
        Align::Right => egui::Align::Max,
    };
    job
}

/// Paints a slide into `rect` (any size: the 16:9 canvas scaled to it).
pub fn paint_slide(ui: &Ui, rect: Rect, slide: &Slide, textures: &HashMap<String, TextureHandle>, hide_text_of: Option<u64>) {
    let painter = ui.painter_at(rect);
    let k = rect.width() / W;
    let to = |x: f32, y: f32| pos2(rect.left() + x * k, rect.top() + y * k);
    painter.rect_filled(rect, 0.0, c32(slide.background));
    for e in &slide.elements {
        let r = Rect::from_min_max(to(e.x, e.y), to(e.x + e.w, e.y + e.h));
        let stroke = e.stroke.map(|c| Stroke::new((e.stroke_w * k).max(0.5), c32(c))).unwrap_or(Stroke::NONE);
        match e.kind {
            Kind::Rect => {
                painter.rect(r, 0.0, e.fill.map(c32).unwrap_or(Color32::TRANSPARENT), stroke, egui::StrokeKind::Middle);
            }
            Kind::Ellipse => {
                painter.add(egui::Shape::ellipse_filled(r.center(), r.size() / 2.0, e.fill.map(c32).unwrap_or(Color32::TRANSPARENT)));
                if stroke != Stroke::NONE {
                    painter.add(egui::Shape::ellipse_stroke(r.center(), r.size() / 2.0, stroke));
                }
            }
            Kind::Line => {
                let (a, b) = if e.flip { (r.left_bottom(), r.right_top()) } else { (r.left_top(), r.right_bottom()) };
                painter.line_segment([a, b], stroke);
            }
            Kind::Image => match e.image.as_ref().and_then(|key| textures.get(key)) {
                Some(t) => {
                    painter.image(t.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                }
                None => {
                    painter.rect_stroke(r, 0.0, Stroke::new(1.0, theme::FAINT), egui::StrokeKind::Middle);
                }
            },
            Kind::Text => {
                if let Some(f) = e.fill {
                    painter.rect_filled(r, 0.0, c32(f));
                }
                if stroke != Stroke::NONE {
                    painter.rect_stroke(r, 0.0, stroke, egui::StrokeKind::Middle);
                }
            }
        }
        if e.has_text() && !e.text.is_empty() && hide_text_of != Some(e.id) {
            let galley = ui.fonts_mut(|f| f.layout_job(text_job(e, k)));
            let inner = r.shrink(PAD * k);
            let y = match e.style.valign {
                VAlign::Top => inner.top(),
                VAlign::Middle => inner.center().y - galley.size().y / 2.0,
                VAlign::Bottom => inner.bottom() - galley.size().y,
            };
            let x = match e.style.align {
                Align::Left => inner.left(),
                Align::Center => inner.center().x,
                Align::Right => inner.right(),
            };
            painter.galley(pos2(x, y), galley, c32(e.style.color));
        }
    }
}

fn handles(r: Rect) -> [Pos2; 8] {
    [
        r.left_top(),
        r.center_top(),
        r.right_top(),
        r.right_center(),
        r.right_bottom(),
        r.center_bottom(),
        r.left_bottom(),
        r.left_center(),
    ]
}

fn dist_to_segment(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_sq().max(1e-6)).clamp(0.0, 1.0);
    (a + ab * t - p).length()
}

/// The topmost element under a point (canvas coordinates).
fn hit(slide: &Slide, p: Pos2, k: f32) -> Option<u64> {
    let slack = 6.0 / k;
    slide.elements.iter().rev().find(|e| {
        let r = Rect::from_min_size(pos2(e.x, e.y), vec2(e.w, e.h));
        if e.kind == Kind::Line {
            let (a, b) = if e.flip { (r.left_bottom(), r.right_top()) } else { (r.left_top(), r.right_bottom()) };
            dist_to_segment(p, a, b) < slack
        } else {
            r.expand(slack).contains(p)
        }
    }).map(|e| e.id)
}

/// The big slide being edited.
pub fn editor(app: &mut App, ui: &mut Ui) {
    let avail = ui.available_rect_before_wrap();
    let k = ((avail.width() - 48.0) / W).min((avail.height() - 48.0) / H).max(0.05);
    let rect = Rect::from_center_size(avail.center(), vec2(W * k, H * k));
    let resp = ui.allocate_rect(avail, Sense::click_and_drag());
    let to_canvas = |p: Pos2| pos2((p.x - rect.left()) / k, (p.y - rect.top()) / k);
    let to_screen = |x: f32, y: f32| pos2(rect.left() + x * k, rect.top() + y * k);
    let busy = app.claude.busy();

    let editing = app.editing;
    paint_slide(ui, rect, &app.deck.slides[app.current], &app.textures, editing);
    ui.painter().rect_stroke(rect, 0.0, Stroke::new(1.0, theme::BORDER), egui::StrokeKind::Outside);

    // Mouse.
    let pointer = ui.input(|i| i.pointer.interact_pos());
    let (pressed, released, double) = ui.input(|i| (i.pointer.primary_pressed(), i.pointer.primary_released(), i.pointer.button_double_clicked(egui::PointerButton::Primary)));
    let shift = ui.input(|i| i.modifiers.shift);
    if let Some(p) = pointer.filter(|_| resp.contains_pointer() || app.drag.is_some()) {
        let cp = to_canvas(p);
        let under = hit(&app.deck.slides[app.current], cp, k);
        let sel_rect = app.selected_element().map(|e| Rect::from_min_size(to_screen(e.x, e.y), vec2(e.w * k, e.h * k)));
        let on_handle = sel_rect.and_then(|r| handles(r).iter().position(|h| (*h - p).length() < 8.0));
        if resp.hovered() {
            if on_handle.is_some() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
            } else if app.insert.is_some() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
            } else if under.is_some() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Move);
            }
        }
        if pressed && !busy && resp.contains_pointer() {
            if app.editing.is_some() && sel_rect.is_some_and(|r| r.contains(p)) {
                // Clicks inside the text box being typed in are the text editor's.
            } else {
                app.stop_editing();
                if let (Some(h), Some(id)) = (on_handle, app.selected) {
                    let e = app.selected_element().unwrap();
                    app.drag = Some(Drag::Resize { id, handle: h, orig: (e.x, e.y, e.w, e.h), flip: e.flip });
                    app.checkpoint();
                } else if let Some(kind) = app.insert {
                    app.drag = Some(Drag::Create { kind, from: cp, id: None });
                } else if let Some(id) = under {
                    app.selected = Some(id);
                    let e = app.selected_element().unwrap();
                    app.drag = Some(Drag::Move { id, from: cp, orig: (e.x, e.y) });
                } else {
                    app.selected = None;
                }
            }
        }
        if double && !busy {
            if let Some(id) = hit(&app.deck.slides[app.current], cp, k) {
                app.selected = Some(id);
                if app.selected_element().is_some_and(|e| e.has_text()) {
                    app.start_editing();
                }
                app.drag = None;
            }
        }
        match app.drag {
            Some(Drag::Move { id, from, orig }) => {
                let d = cp - from;
                if d.length() * k > 3.0 {
                    if !app.moved {
                        app.checkpoint();
                        app.moved = true;
                    }
                    if let Some(e) = app.element_mut(id) {
                        e.x = (orig.0 + d.x).round();
                        e.y = (orig.1 + d.y).round();
                    }
                }
            }
            Some(Drag::Resize { id, handle, orig: (x, y, w, h), flip }) => {
                let (mut l, mut t, mut r, mut b) = (x, y, x + w, y + h);
                match handle {
                    0 => (l, t) = (cp.x, cp.y),
                    1 => t = cp.y,
                    2 => (r, t) = (cp.x, cp.y),
                    3 => r = cp.x,
                    4 => (r, b) = (cp.x, cp.y),
                    5 => b = cp.y,
                    6 => (l, b) = (cp.x, cp.y),
                    _ => l = cp.x,
                }
                let is_image = app.element_mut(id).is_some_and(|e| e.kind == Kind::Image);
                // Pictures keep their shape from the corners (Shift lets go).
                if is_image != shift && matches!(handle, 0 | 2 | 4 | 6) && w > 0.0 && h > 0.0 {
                    let ratio = w / h;
                    let nw = (r - l).abs();
                    let nh = nw / ratio;
                    if matches!(handle, 0 | 2) {
                        t = b - nh;
                    } else {
                        b = t + nh;
                    }
                }
                if let Some(e) = app.element_mut(id) {
                    if e.kind == Kind::Line {
                        // A line dragged past itself turns over.
                        e.flip = flip != ((r < l) != (b < t));
                    }
                    e.x = l.min(r).round();
                    e.y = t.min(b).round();
                    e.w = (r - l).abs().max(4.0).round();
                    e.h = (b - t).abs().max(if e.kind == Kind::Line { 0.0 } else { 4.0 }).round();
                }
            }
            Some(Drag::Create { kind, from, mut id }) => {
                let d = cp - from;
                if id.is_none() && d.length() * k > 4.0 {
                    id = Some(app.add_element(kind, from.x, from.y, 1.0, 1.0));
                    app.drag = Some(Drag::Create { kind, from, id });
                }
                if let Some(id) = id {
                    if let Some(e) = app.element_mut(id) {
                        e.x = from.x.min(cp.x).round();
                        e.y = from.y.min(cp.y).round();
                        e.w = (cp.x - from.x).abs().max(4.0).round();
                        e.h = (cp.y - from.y).abs().round().max(if kind == Kind::Line { 0.0 } else { 4.0 });
                        if kind == Kind::Line {
                            e.flip = (cp.x > from.x) != (cp.y > from.y);
                        }
                    }
                }
            }
            None => {}
        }
        if released {
            if let Some(Drag::Create { kind, from, id: None }) = app.drag {
                // A plain click with a tool: an element of a usual size there.
                let (w, h) = match kind {
                    Kind::Text => (480.0, 70.0),
                    Kind::Line => (300.0, 0.0),
                    _ => (240.0, 160.0),
                };
                let id = app.add_element(kind, from.x, from.y, w, h);
                if kind == Kind::Text {
                    app.selected = Some(id);
                    app.start_editing();
                }
            } else if let Some(Drag::Create { kind: Kind::Text, id: Some(id), .. }) = app.drag {
                app.selected = Some(id);
                app.start_editing();
            }
            if matches!(app.drag, Some(Drag::Create { .. })) {
                app.insert = None;
            }
            app.drag = None;
            app.moved = false;
        }
    }

    // The selection: an outline and its handles.
    if let Some(e) = app.selected_element() {
        let r = Rect::from_min_size(to_screen(e.x, e.y), vec2(e.w * k, e.h * k));
        let p = ui.painter();
        p.rect_stroke(r, 0.0, Stroke::new(1.0, theme::SELECT), egui::StrokeKind::Outside);
        if app.editing.is_none() {
            for h in handles(r) {
                p.rect_filled(Rect::from_center_size(h, vec2(8.0, 8.0)), 2.0, theme::TEXT);
                p.rect_stroke(Rect::from_center_size(h, vec2(8.0, 8.0)), 2.0, Stroke::new(1.0, theme::BG), egui::StrokeKind::Outside);
            }
        }
    }

    text_editor(app, ui, rect, k);
}

/// Typing into a text box (or a shape's text): a text field over it, in
/// its font, size and colour.
fn text_editor(app: &mut App, ui: &mut Ui, rect: Rect, k: f32) {
    let Some(id) = app.editing else { return };
    let Some(e) = app.element_mut(id) else {
        app.editing = None;
        return;
    };
    let r = Rect::from_min_size(pos2(rect.left() + e.x * k, rect.top() + e.y * k), vec2(e.w * k, e.h * k)).shrink(PAD * k);
    let s = e.style.clone();
    let size = s.size * 4.0 / 3.0 * k;
    let field_id = egui::Id::unique(("slide_text", id));
    let align = match s.align {
        Align::Left => egui::Align::Min,
        Align::Center => egui::Align::Center,
        Align::Right => egui::Align::Max,
    };
    let mut text = e.text.clone();
    let font = FontId::new(size.max(1.0), fonts::family(&s.font, s.bold));
    let edit = egui::TextEdit::multiline(&mut text)
        .id(field_id)
        .frame(egui::Frame::NONE)
        .font(font)
        .text_color(c32(s.color))
        .align(egui::Align2([align, egui::Align::Min]))
        .desired_width(r.width())
        .margin(egui::Margin::ZERO);
    let resp = ui.put(r, edit);
    if app.focus_text {
        resp.request_focus();
        app.focus_text = false;
    }
    if text != app.element_mut(id).map(|e| e.text.clone()).unwrap_or_default() {
        if let Some(e) = app.element_mut(id) {
            e.text = text;
        }
        app.dirty = true;
    }
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.stop_editing();
    }
}

/// The slide list down the left.
pub fn thumbnails(app: &mut App, ui: &mut Ui) {
    egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
        let w = ui.available_width() - 30.0;
        let mut action = None;
        for i in 0..app.deck.slides.len() {
            ui.add_space(6.0);
            let (row, resp) = ui.allocate_exact_size(vec2(ui.available_width(), w * H / W + 4.0), Sense::click());
            let rect = Rect::from_min_size(pos2(row.left() + 26.0, row.top() + 2.0), vec2(w, w * H / W));
            ui.painter().text(pos2(row.left() + 12.0, rect.top() + 2.0), egui::Align2::CENTER_TOP, (i + 1).to_string(), FontId::proportional(12.0), theme::MUTED);
            paint_slide(ui, rect, &app.deck.slides[i], &app.textures, None);
            let current = i == app.current;
            ui.painter().rect_stroke(
                rect,
                3.0,
                Stroke::new(if current { 2.0 } else { 1.0 }, if current { theme::TEXT } else { theme::BORDER }),
                egui::StrokeKind::Outside,
            );
            if resp.clicked() {
                app.go_to(i);
            }
            resp.context_menu(|ui| {
                ui.add_enabled_ui(!app.claude.busy(), |ui| {
                    if ui.button("Duplicate").clicked() {
                        action = Some(("dup", i));
                    }
                    if ui.add_enabled(i > 0, egui::Button::new("Move up")).clicked() {
                        action = Some(("up", i));
                    }
                    if ui.add_enabled(i + 1 < app.deck.slides.len(), egui::Button::new("Move down")).clicked() {
                        action = Some(("down", i));
                    }
                    if ui.add_enabled(app.deck.slides.len() > 1, egui::Button::new("Delete")).clicked() {
                        action = Some(("del", i));
                    }
                });
            });
        }
        ui.add_space(8.0);
        if let Some((what, i)) = action {
            app.checkpoint();
            match what {
                "dup" => {
                    let mut s = app.deck.slides[i].clone();
                    for e in &mut s.elements {
                        e.id = app.deck.id();
                    }
                    app.deck.slides.insert(i + 1, s);
                    app.go_to(i + 1);
                }
                "up" => {
                    app.deck.slides.swap(i, i - 1);
                    app.go_to(i - 1);
                }
                "down" => {
                    app.deck.slides.swap(i, i + 1);
                    app.go_to(i + 1);
                }
                _ => {
                    app.deck.slides.remove(i);
                    app.go_to(i.min(app.deck.slides.len() - 1));
                }
            }
        }
    });
}

/// Presenting: the slide alone, as large as the screen allows, on black.
pub fn present(app: &mut App, ui: &mut Ui) {
    let avail = ui.max_rect();
    ui.painter().rect_filled(avail, 0.0, Color32::BLACK);
    let k = (avail.width() / W).min(avail.height() / H);
    let rect = Rect::from_center_size(avail.center(), vec2(W * k, H * k));
    paint_slide(ui, rect, &app.deck.slides[app.current], &app.textures, None);
    let resp = ui.allocate_rect(avail, Sense::click());
    let n = app.deck.slides.len();
    let (next, prev, esc) = ui.input(|i| {
        use egui::Key::*;
        (
            [ArrowRight, ArrowDown, Space, PageDown, Enter].iter().any(|k| i.key_pressed(*k)),
            [ArrowLeft, ArrowUp, PageUp, Backspace].iter().any(|k| i.key_pressed(*k)),
            i.key_pressed(Escape),
        )
    });
    if (next || resp.clicked()) && app.current + 1 < n {
        app.current += 1;
    } else if (next || resp.clicked()) && app.current + 1 >= n {
        app.stop_presenting(ui.ctx());
    }
    if prev && app.current > 0 {
        app.current -= 1;
    }
    if esc || resp.secondary_clicked() {
        app.stop_presenting(ui.ctx());
    }
    ui.ctx().set_cursor_icon(egui::CursorIcon::None);
}
