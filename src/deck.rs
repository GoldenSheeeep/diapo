//! A presentation: slides of elements (text boxes, shapes, lines,
//! pictures) placed on a 1280 × 720 canvas, the 16:9 size PowerPoint and
//! Google Slides use.

use std::collections::HashMap;
use std::sync::Arc;

pub const W: f32 = 1280.0;
pub const H: f32 = 720.0;

pub type Rgb = [u8; 3];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Text,
    Rect,
    Ellipse,
    Line,
    Image,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Text => "text",
            Kind::Rect => "rect",
            Kind::Ellipse => "ellipse",
            Kind::Line => "line",
            Kind::Image => "image",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum VAlign {
    #[default]
    Top,
    Middle,
    Bottom,
}

#[derive(Clone, PartialEq, Debug)]
pub struct TextStyle {
    /// Points.
    pub size: f32,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub color: Rgb,
    pub align: Align,
    pub valign: VAlign,
    /// "Sans", "Serif" or "Mono".
    pub font: String,
    /// Each line a bullet point.
    pub bullets: bool,
}

impl Default for TextStyle {
    fn default() -> Self {
        TextStyle {
            size: 18.0,
            bold: false,
            italic: false,
            underline: false,
            color: [255, 255, 255],
            align: Align::Left,
            valign: VAlign::Top,
            font: "Sans".to_owned(),
            bullets: false,
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct Element {
    pub id: u64,
    pub kind: Kind,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub fill: Option<Rgb>,
    pub stroke: Option<Rgb>,
    pub stroke_w: f32,
    /// Lines of text (text boxes, and text inside shapes).
    pub text: String,
    pub style: TextStyle,
    /// A key into the deck's pictures.
    pub image: Option<String>,
    /// Lines only: drawn from bottom left to top right instead.
    pub flip: bool,
}

impl Element {
    pub fn new(id: u64, kind: Kind, x: f32, y: f32, w: f32, h: f32) -> Element {
        let shape = matches!(kind, Kind::Rect | Kind::Ellipse);
        Element {
            id,
            kind,
            x,
            y,
            w,
            h,
            fill: shape.then_some([0x40, 0x40, 0x40]),
            stroke: (kind == Kind::Line).then_some([255, 255, 255]),
            stroke_w: 2.0,
            text: String::new(),
            style: TextStyle {
                align: if shape { Align::Center } else { Align::Left },
                valign: if shape { VAlign::Middle } else { VAlign::Top },
                ..Default::default()
            },
            image: None,
            flip: false,
        }
    }

    pub fn has_text(&self) -> bool {
        matches!(self.kind, Kind::Text | Kind::Rect | Kind::Ellipse)
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct Slide {
    pub background: Rgb,
    pub elements: Vec<Element>,
    pub notes: String,
}

#[derive(Clone, Debug)]
pub struct Deck {
    pub slides: Vec<Slide>,
    /// Pictures, by key ("image3.png"), shared between undo steps.
    pub images: HashMap<String, Arc<Vec<u8>>>,
    next_id: u64,
}

impl Default for Deck {
    fn default() -> Deck {
        let mut d = Deck { slides: Vec::new(), images: HashMap::new(), next_id: 1 };
        let s = d.new_slide(Layout::Title, "Click to add a title", "");
        d.slides.push(s);
        d
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    Title,
    TitleBody,
    Section,
    Blank,
}

impl Layout {
    pub fn parse(s: &str) -> Layout {
        match s.to_lowercase().replace([' ', '_', '-'], "").as_str() {
            "title" | "titleslide" => Layout::Title,
            "section" | "sectionheader" => Layout::Section,
            "blank" => Layout::Blank,
            _ => Layout::TitleBody,
        }
    }
}

impl Deck {
    pub fn id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    /// Used ids go past ones read from a file.
    pub fn bump_ids(&mut self) {
        let max = self.slides.iter().flat_map(|s| s.elements.iter().map(|e| e.id)).max().unwrap_or(0);
        self.next_id = self.next_id.max(max);
    }

    /// A slide laid out like Google Slides' layouts, in the deck's colours
    /// (those of its first slide's background, black for a new deck).
    pub fn new_slide(&mut self, layout: Layout, title: &str, body: &str) -> Slide {
        let background = self.slides.first().map(|s| s.background).unwrap_or([0, 0, 0]);
        let fg = if luminance(background) < 0.5 { [255, 255, 255] } else { [0, 0, 0] };
        let mut els = Vec::new();
        let mut text = |deck: &mut Deck, x, y, w, h, t: &str, size, bold, align, valign, bullets| {
            let mut e = Element::new(deck.id(), Kind::Text, x, y, w, h);
            e.text = t.to_owned();
            e.style = TextStyle { size, bold, align, valign, bullets, color: fg, ..Default::default() };
            els.push(e);
        };
        match layout {
            Layout::Title => {
                text(self, 100.0, 220.0, 1080.0, 160.0, title, 48.0, true, Align::Center, VAlign::Bottom, false);
                text(self, 100.0, 400.0, 1080.0, 90.0, body, 24.0, false, Align::Center, VAlign::Top, false);
            }
            Layout::Section => {
                text(self, 100.0, 260.0, 1080.0, 200.0, title, 44.0, true, Align::Left, VAlign::Middle, false);
            }
            Layout::TitleBody => {
                text(self, 80.0, 50.0, 1120.0, 110.0, title, 36.0, true, Align::Left, VAlign::Middle, false);
                text(self, 80.0, 180.0, 1120.0, 480.0, body, 22.0, false, Align::Left, VAlign::Top, true);
            }
            Layout::Blank => {}
        }
        Slide { background, elements: els, notes: String::new() }
    }

    pub fn add_image(&mut self, bytes: Vec<u8>, ext: &str) -> String {
        let key = format!("image{}.{ext}", self.id());
        self.images.insert(key.clone(), Arc::new(bytes));
        key
    }
}

pub fn luminance(c: Rgb) -> f32 {
    (0.2126 * c[0] as f32 + 0.7152 * c[1] as f32 + 0.0722 * c[2] as f32) / 255.0
}

pub fn hex(c: Rgb) -> String {
    format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2])
}

pub fn parse_hex(s: &str) -> Option<Rgb> {
    atelier::theme::parse_hex(s).map(|c| [c.r(), c.g(), c.b()])
}
