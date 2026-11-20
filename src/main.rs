//! Slides: presentations. Opens and saves PowerPoint files (.pptx). Claude,
//! in the panel on the right, reads and edits the open presentation
//! through tools.rs.

mod canvas;
mod deck;
mod pptx;
mod tools;
mod xml;

use deck::{Align, Deck, Element, Kind, Layout};
use atelier::bridge::Bridge;
use atelier::claude::Claude;
use atelier::closing::{Choice, CloseGuard};
use atelier::egui::{self, Key, RichText, TextureHandle, Ui};
use atelier::icons::{self, Icon};
use atelier::{files, fonts, theme, titlebar, widgets};
use std::collections::HashMap;
use std::path::PathBuf;

pub struct App {
    pub deck: Deck,
    pub current: usize,
    pub selected: Option<u64>,
    /// The text box being typed in.
    pub editing: Option<u64>,
    pub focus_text: bool,
    /// A drawing tool picked: the next drag on the slide draws one.
    pub insert: Option<Kind>,
    pub drag: Option<canvas::Drag>,
    pub moved: bool,
    undo: Vec<(Deck, usize)>,
    redo: Vec<(Deck, usize)>,
    pub dirty: bool,
    path: Option<PathBuf>,
    pub textures: HashMap<String, TextureHandle>,
    presenting: bool,
    pub claude: Claude,
    bridge: Bridge,
    close: CloseGuard,
    status: Option<(String, f64)>,
    clip: Option<Element>,
}

fn rgb32(c: [u8; 3]) -> egui::Color32 {
    egui::Color32::from_rgb(c[0], c[1], c[2])
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> App {
        let bridge = Bridge::start("slides", cc.egui_ctx.clone());
        let claude = Claude::new(
            "Slides",
            bridge.path.clone(),
            "presentation",
            &["Make a 5-slide outline about this topic", "Tighten the wording on this slide", "Give every slide the same title style"],
        );
        let mut app = App {
            deck: Deck::default(),
            current: 0,
            selected: None,
            editing: None,
            focus_text: false,
            insert: None,
            drag: None,
            moved: false,
            undo: Vec::new(),
            redo: Vec::new(),
            dirty: false,
            path: None,
            textures: HashMap::new(),
            presenting: false,
            claude,
            bridge,
            close: CloseGuard::default(),
            status: None,
            clip: None,
        };
        if let Some(p) = atelier::file_arg() {
            app.open_path(p);
        }
        app
    }

    fn name(&self) -> String {
        self.path.as_deref().map(files::stem).unwrap_or_else(|| "Untitled presentation".to_owned())
    }

    fn say(&mut self, msg: impl Into<String>, ctx: &egui::Context) {
        self.status = Some((msg.into(), ctx.input(|i| i.time) + 4.0));
    }

    pub fn checkpoint(&mut self) {
        self.undo.push((self.deck.clone(), self.current));
        if self.undo.len() > 200 {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.dirty = true;
    }

    fn undo(&mut self) {
        self.stop_editing();
        if let Some((d, c)) = self.undo.pop() {
            self.redo.push((std::mem::replace(&mut self.deck, d), self.current));
            self.current = c.min(self.deck.slides.len() - 1);
            self.dirty = true;
        }
    }

    fn redo(&mut self) {
        self.stop_editing();
        if let Some((d, c)) = self.redo.pop() {
            self.undo.push((std::mem::replace(&mut self.deck, d), self.current));
            self.current = c.min(self.deck.slides.len() - 1);
            self.dirty = true;
        }
    }

    pub fn go_to(&mut self, i: usize) {
        self.stop_editing();
        self.current = i.min(self.deck.slides.len() - 1);
        self.selected = None;
    }

    pub fn selected_element(&self) -> Option<&Element> {
        let id = self.selected?;
        self.deck.slides[self.current].elements.iter().find(|e| e.id == id)
    }

    pub fn element_mut(&mut self, id: u64) -> Option<&mut Element> {
        self.deck.slides[self.current].elements.iter_mut().find(|e| e.id == id)
    }

    pub fn add_element(&mut self, kind: Kind, x: f32, y: f32, w: f32, h: f32) -> u64 {
        self.checkpoint();
        let id = self.deck.id();
        let mut e = Element::new(id, kind, x, y, w, h);
        // Text in the colour that reads on this slide.
        let bg = self.deck.slides[self.current].background;
        e.style.color = if deck::luminance(bg) < 0.5 { [255, 255, 255] } else { [0, 0, 0] };
        if kind == Kind::Line && deck::luminance(bg) >= 0.5 {
            e.stroke = Some([0, 0, 0]);
        }
        self.deck.slides[self.current].elements.push(e);
        self.selected = Some(id);
        id
    }

    pub fn start_editing(&mut self) {
        if self.claude.busy() {
            return;
        }
        if let Some(id) = self.selected {
            self.checkpoint();
            self.editing = Some(id);
            self.focus_text = true;
        }
    }

    pub fn stop_editing(&mut self) {
        // A text box left empty goes.
        if let Some(id) = self.editing.take() {
            let slide = &mut self.deck.slides[self.current];
            slide.elements.retain(|e| !(e.id == id && e.kind == Kind::Text && e.text.trim().is_empty() && e.fill.is_none()));
            if self.selected.is_some_and(|s| !slide.elements.iter().any(|e| e.id == s)) {
                self.selected = None;
            }
        }
    }

    /// Changes the selected element (one undo step per click).
    fn change(&mut self, f: impl FnOnce(&mut Element)) {
        let Some(id) = self.selected else { return };
        if self.editing.is_none() {
            self.checkpoint();
        }
        if let Some(e) = self.element_mut(id) {
            f(e);
        }
        self.dirty = true;
    }

    fn new_slide(&mut self, layout: Layout) {
        self.stop_editing();
        self.checkpoint();
        let s = self.deck.new_slide(layout, "", "");
        self.deck.slides.insert(self.current + 1, s);
        self.go_to(self.current + 1);
    }

    fn delete_selected(&mut self) {
        if let Some(id) = self.selected.take() {
            self.checkpoint();
            self.deck.slides[self.current].elements.retain(|e| e.id != id);
        }
    }

    fn duplicate_selected(&mut self) {
        if let Some(mut e) = self.selected_element().cloned() {
            self.checkpoint();
            e.id = self.deck.id();
            e.x += 20.0;
            e.y += 20.0;
            self.selected = Some(e.id);
            self.deck.slides[self.current].elements.push(e);
        }
    }

    fn arrange(&mut self, front: bool) {
        let Some(id) = self.selected else { return };
        self.checkpoint();
        let els = &mut self.deck.slides[self.current].elements;
        if let Some(i) = els.iter().position(|e| e.id == id) {
            let e = els.remove(i);
            if front {
                els.push(e);
            } else {
                els.insert(0, e);
            }
        }
    }

    fn insert_image(&mut self) {
        let Some(p) = files::open("Pictures", &["png", "jpg", "jpeg", "gif", "webp"]) else { return };
        let Ok(bytes) = std::fs::read(&p) else { return };
        let Ok(img) = image::load_from_memory(&bytes) else { return };
        let ext = p.extension().and_then(|e| e.to_str()).unwrap_or_default().to_lowercase();
        let key = match ext.as_str() {
            "png" => self.deck.add_image(bytes, "png"),
            "jpg" | "jpeg" => self.deck.add_image(bytes, "jpeg"),
            _ => {
                let mut png = Vec::new();
                if img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).is_err() {
                    return;
                }
                self.deck.add_image(png, "png")
            }
        };
        // At its own size, made to fit within most of the slide.
        let (w, h) = (img.width() as f32, img.height() as f32);
        let k = (deck::W * 0.7 / w).min(deck::H * 0.7 / h).min(1.0);
        let (w, h) = (w * k, h * k);
        let id = self.add_element(Kind::Image, (deck::W - w) / 2.0, (deck::H - h) / 2.0, w, h);
        if let Some(e) = self.element_mut(id) {
            e.image = Some(key);
            e.fill = None;
        }
    }

    /// Pictures as textures, loaded once.
    fn load_textures(&mut self, ctx: &egui::Context) {
        for (key, bytes) in &self.deck.images {
            if self.textures.contains_key(key) {
                continue;
            }
            if let Ok(img) = image::load_from_memory(bytes) {
                let rgba = img.to_rgba8();
                let size = [rgba.width() as usize, rgba.height() as usize];
                let color = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
                let tex = ctx.load_texture(key, color, egui::TextureOptions::LINEAR);
                self.textures.insert(key.clone(), tex);
            }
        }
    }

    fn open_path(&mut self, p: PathBuf) {
        match std::fs::read(&p).map_err(|e| e.to_string()).and_then(pptx::load) {
            Ok(d) => {
                self.deck = d;
                self.path = Some(p);
                self.current = 0;
                self.selected = None;
                self.undo.clear();
                self.redo.clear();
                self.dirty = false;
                self.textures.clear();
            }
            Err(e) => self.status = Some((format!("Couldn't open {}: {e}", p.display()), f64::MAX)),
        }
    }

    fn open(&mut self) {
        if let Some(p) = files::open("Presentations", &["pptx"]) {
            self.open_path(p);
        }
    }

    fn save(&mut self, ctx: &egui::Context) -> bool {
        self.stop_editing();
        let path = match &self.path {
            Some(p) => p.clone(),
            None => match files::save("PowerPoint presentation", &["pptx"], &format!("{}.pptx", self.name()), None) {
                Some(p) => p,
                None => return false,
            },
        };
        self.save_to(path, ctx)
    }

    fn save_as(&mut self, ctx: &egui::Context) {
        let dir = self.path.as_ref().and_then(|p| p.parent().map(|d| d.to_path_buf()));
        if let Some(p) = files::save("PowerPoint presentation", &["pptx"], &format!("{}.pptx", self.name()), dir.as_deref()) {
            self.save_to(p, ctx);
        }
    }

    fn save_to(&mut self, path: PathBuf, ctx: &egui::Context) -> bool {
        match pptx::save(&self.deck, &path) {
            Ok(()) => {
                self.path = Some(path);
                self.dirty = false;
                self.say("Saved", ctx);
                true
            }
            Err(e) => {
                self.say(format!("Couldn't save: {e}"), ctx);
                false
            }
        }
    }

    fn start_presenting(&mut self, ctx: &egui::Context, from_start: bool) {
        self.stop_editing();
        if from_start {
            self.current = 0;
        }
        self.presenting = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(true));
    }

    pub fn stop_presenting(&mut self, ctx: &egui::Context) {
        self.presenting = false;
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
    }

    fn keys(&mut self, ctx: &egui::Context) {
        let typing = ctx.memory(|m| m.focused().is_some());
        let busy = self.claude.busy();
        let events = ctx.input(|i| i.events.clone());
        for ev in events {
            match ev {
                egui::Event::Key { key, pressed: true, modifiers: m, .. } => {
                    let cmd = m.command;
                    match key {
                        Key::S if cmd && m.shift => self.save_as(ctx),
                        Key::S if cmd => {
                            self.save(ctx);
                        }
                        Key::O if cmd => self.open(),
                        Key::N if cmd => {
                            std::process::Command::new(std::env::current_exe().unwrap()).spawn().ok();
                        }
                        Key::J if cmd => self.claude.toggle(),
                        Key::F5 => self.start_presenting(ctx, !m.shift),
                        Key::Enter if cmd => self.start_presenting(ctx, false),
                        _ => {}
                    }
                    if typing || busy {
                        continue;
                    }
                    let step = if m.shift { 10.0 } else { 1.0 };
                    match key {
                        Key::Z if cmd && m.shift => self.redo(),
                        Key::Z if cmd => self.undo(),
                        Key::Y if cmd => self.redo(),
                        Key::D if cmd => self.duplicate_selected(),
                        Key::M if cmd => self.new_slide(Layout::TitleBody),
                        Key::Delete | Key::Backspace if self.selected.is_some() => self.delete_selected(),
                        Key::Enter if self.selected.is_some() => self.start_editing(),
                        Key::Escape => {
                            self.selected = None;
                            self.insert = None;
                        }
                        Key::ArrowLeft | Key::ArrowRight | Key::ArrowUp | Key::ArrowDown if self.selected.is_some() => {
                            let (dx, dy) = match key {
                                Key::ArrowLeft => (-step, 0.0),
                                Key::ArrowRight => (step, 0.0),
                                Key::ArrowUp => (0.0, -step),
                                _ => (0.0, step),
                            };
                            self.change(|e| {
                                e.x += dx;
                                e.y += dy;
                            });
                        }
                        Key::ArrowUp | Key::PageUp => self.go_to(self.current.saturating_sub(1)),
                        Key::ArrowDown | Key::PageDown => self.go_to(self.current + 1),
                        _ => {}
                    }
                }
                egui::Event::Copy if !typing => {
                    if let Some(e) = self.selected_element().cloned() {
                        ctx.copy_text(e.text.clone());
                        self.clip = Some(e);
                    }
                }
                egui::Event::Cut if !typing && !busy => {
                    if let Some(e) = self.selected_element().cloned() {
                        ctx.copy_text(e.text.clone());
                        self.clip = Some(e);
                        self.delete_selected();
                    }
                }
                egui::Event::Paste(text) if !typing && !busy => {
                    match self.clip.clone().filter(|e| e.text == text) {
                        Some(mut e) => {
                            self.checkpoint();
                            e.id = self.deck.id();
                            e.x += 20.0;
                            e.y += 20.0;
                            self.selected = Some(e.id);
                            self.clip = Some(e.clone());
                            self.deck.slides[self.current].elements.push(e);
                        }
                        None if !text.trim().is_empty() => {
                            let id = self.add_element(Kind::Text, 200.0, 280.0, 880.0, 160.0);
                            if let Some(e) = self.element_mut(id) {
                                e.text = text;
                            }
                        }
                        None => {}
                    }
                }
                _ => {}
            }
        }
    }

    fn menus(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        ui.menu_button("File", |ui| {
            if ui.button("New").clicked() {
                std::process::Command::new(std::env::current_exe().unwrap()).spawn().ok();
            }
            if ui.button("Open…").clicked() {
                self.open();
            }
            ui.separator();
            if ui.button("Save").clicked() {
                self.save(&ctx);
            }
            if ui.button("Save as…").clicked() {
                self.save_as(&ctx);
            }
        });
        ui.menu_button("Slide", |ui| {
            ui.add_enabled_ui(!self.claude.busy(), |ui| {
                for (name, l) in [("Title slide", Layout::Title), ("Title and text", Layout::TitleBody), ("Section", Layout::Section), ("Blank", Layout::Blank)] {
                    if ui.button(format!("New: {name}")).clicked() {
                        self.new_slide(l);
                    }
                }
            });
        });
        ui.menu_button("Present", |ui| {
            if ui.button("From the start (F5)").clicked() {
                self.start_presenting(&ctx, true);
            }
            if ui.button("From this slide (Shift+F5)").clicked() {
                self.start_presenting(&ctx, false);
            }
        });
    }

    fn toolbar(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        ui.add_enabled_ui(!self.claude.busy(), |ui| {
            if icons::button(ui, Icon::Undo, false, "Undo (Ctrl+Z)").clicked() {
                self.undo();
            }
            if icons::button(ui, Icon::Redo, false, "Redo (Ctrl+Y)").clicked() {
                self.redo();
            }
            widgets::separator(ui);
            if icons::button(ui, Icon::Plus, false, "New slide (Ctrl+M)").clicked() {
                self.new_slide(Layout::TitleBody);
            }
            widgets::separator(ui);
            for (icon, kind, tip) in [
                (Icon::TextBox, Kind::Text, "Text box"),
                (Icon::Rect, Kind::Rect, "Rectangle"),
                (Icon::Ellipse, Kind::Ellipse, "Ellipse"),
                (Icon::Line, Kind::Line, "Line"),
            ] {
                let on = self.insert == Some(kind);
                if icons::button(ui, icon, on, &format!("{tip}: drag on the slide")).clicked() {
                    self.stop_editing();
                    self.insert = if on { None } else { Some(kind) };
                }
            }
            if icons::button(ui, Icon::Image, false, "Picture").clicked() {
                self.insert_image();
            }
            widgets::separator(ui);
            let Some(e) = self.selected_element().cloned() else {
                // Nothing selected: the slide's background.
                let bg = self.deck.slides[self.current].background;
                if let Some(Some(c)) = widgets::color_button(ui, Icon::Fill, Some(rgb32(bg)), "Background") {
                    self.checkpoint();
                    self.deck.slides[self.current].background = [c.r(), c.g(), c.b()];
                }
                ui.label(RichText::new("Background").color(theme::MUTED));
                return;
            };
            if e.has_text() {
                if let Some(i) = widgets::choice(ui, "font", &e.style.font, 64.0, &fonts::FAMILIES) {
                    self.change(|x| x.style.font = fonts::FAMILIES[i].to_owned());
                }
                if icons::button(ui, Icon::Minus, false, "Smaller text").clicked() {
                    self.change(|x| x.style.size = (x.style.size - 2.0).max(6.0));
                }
                ui.label(RichText::new(format!("{}", e.style.size.round())).color(theme::MUTED));
                if icons::button(ui, Icon::Plus, false, "Larger text").clicked() {
                    self.change(|x| x.style.size = (x.style.size + 2.0).min(200.0));
                }
                let bold = egui::FontId::new(14.0, fonts::family("Sans", true));
                if icons::text_button(ui, RichText::new("B").font(bold), e.style.bold, "Bold").clicked() {
                    self.change(|x| x.style.bold = !x.style.bold);
                }
                if icons::text_button(ui, RichText::new("I").italics(), e.style.italic, "Italic").clicked() {
                    self.change(|x| x.style.italic = !x.style.italic);
                }
                if icons::text_button(ui, RichText::new("U").underline(), e.style.underline, "Underline").clicked() {
                    self.change(|x| x.style.underline = !x.style.underline);
                }
                if let Some(Some(c)) = widgets::color_button(ui, Icon::TextColor, Some(rgb32(e.style.color)), "Text colour") {
                    self.change(|x| x.style.color = [c.r(), c.g(), c.b()]);
                }
                for (icon, al, tip) in [(Icon::AlignLeft, Align::Left, "Left"), (Icon::AlignCenter, Align::Center, "Centre"), (Icon::AlignRight, Align::Right, "Right")] {
                    if icons::button(ui, icon, e.style.align == al, tip).clicked() {
                        self.change(|x| x.style.align = al);
                    }
                }
                if icons::button(ui, Icon::Bullets, e.style.bullets, "Bullet points").clicked() {
                    self.change(|x| x.style.bullets = !x.style.bullets);
                }
                widgets::separator(ui);
            }
            if e.kind != Kind::Line && e.kind != Kind::Image {
                if let Some(c) = widgets::color_button(ui, Icon::Fill, e.fill.map(rgb32), "Fill (Automatic: none)") {
                    self.change(|x| x.fill = c.map(|c| [c.r(), c.g(), c.b()]));
                }
            }
            if e.kind != Kind::Image {
                if let Some(c) = widgets::color_button(ui, Icon::Line, e.stroke.map(rgb32), "Outline (Automatic: none)") {
                    self.change(|x| x.stroke = c.map(|c| [c.r(), c.g(), c.b()]));
                }
            }
            widgets::separator(ui);
            if icons::button(ui, Icon::Up, false, "Bring to front").clicked() {
                self.arrange(true);
            }
            if icons::button(ui, Icon::Down, false, "Send to back").clicked() {
                self.arrange(false);
            }
            if icons::button(ui, Icon::Duplicate, false, "Duplicate (Ctrl+D)").clicked() {
                self.duplicate_selected();
            }
            if icons::button(ui, Icon::Trash, false, "Delete").clicked() {
                self.delete_selected();
            }
            let _ = &ctx;
        });
    }

    fn context(&self) -> String {
        let mut s = format!(
            "Presentation \"{}\", {} slides, on slide {}",
            self.name(),
            self.deck.slides.len(),
            self.current + 1
        );
        if let Some(e) = self.selected_element() {
            s.push_str(&format!(", element {} ({}) selected", e.id, e.kind.name()));
        }
        s
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        for req in self.bridge.pending() {
            tools::handle(self, req);
        }
        self.close.check(ctx, self.dirty);
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.load_textures(&ctx);
        if self.presenting {
            canvas::present(self, ui);
            return;
        }
        let title = format!("{}{}", self.name(), if self.dirty { " •" } else { "" });
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!("{} – Slides", self.name())));
        let (mut toggle, mut play, open) = (false, false, self.claude.open);
        titlebar::show(
            ui,
            &title,
            |ui| self.menus(ui),
            |ui| {
                toggle = widgets::claude_toggle(ui, open);
                let r = ui.add(egui::Button::new(RichText::new("     Present").size(13.0)).fill(theme::BUTTON).corner_radius(14.0).min_size(egui::vec2(0.0, 28.0)));
                icons::paint(ui.painter(), Icon::Play, r.rect.left_center() + egui::vec2(16.0, 0.0), theme::TEXT);
                play = r.on_hover_text("Present from this slide (F5 from the start)").clicked();
            },
        );
        if toggle {
            self.claude.toggle();
        }
        if play {
            self.start_presenting(&ctx, false);
        }
        widgets::toolbar(ui, |ui| self.toolbar(ui));
        egui::Panel::bottom("status")
            .exact_size(26.0)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(theme::BG).inner_margin(egui::Margin::symmetric(14, 4)))
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    let now = ui.input(|i| i.time);
                    if let Some((msg, until)) = &self.status {
                        if now < *until {
                            ui.label(RichText::new(msg).size(12.0).color(theme::MUTED));
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(format!("Slide {} of {}", self.current + 1, self.deck.slides.len())).size(12.0).color(theme::MUTED));
                    });
                });
            });
        egui::Panel::left("slides_list")
            .exact_size(210.0)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(theme::PANEL).stroke(egui::Stroke::new(1.0, theme::BORDER)).inner_margin(egui::Margin::symmetric(8, 6)))
            .show(ui, |ui| canvas::thumbnails(self, ui));
        let context = self.context();
        self.claude.panel(ui, || context);
        egui::CentralPanel::default().frame(egui::Frame::new().fill(theme::BG)).show(ui, |ui| canvas::editor(self, ui));
        self.keys(&ctx);
        if let Some(Choice::Save) = self.close.dialog(&ctx, &self.name()) {
            if self.save(&ctx) {
                self.close.close_now(&ctx);
            }
        }
    }
}

fn main() -> eframe::Result {
    atelier::run("slides", "Slides", tools::list, App::new)
}
