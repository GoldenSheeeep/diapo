//! Claude's tools in Slides: read the slides and their elements, add,
//! change, remove and reorder them. Slides are numbered from 1; elements
//! have ids. Positions are on a 1280 × 720 canvas. Each change is one undo
//! step.

use crate::App;
use crate::deck::{self, Align, Element, Kind, Layout, VAlign};
use atelier::bridge::Request;
use atelier::mcp::tool;
use serde_json::{Value, json};

fn element_props() -> Vec<(&'static str, &'static str, &'static str, bool)> {
    vec![
        ("x", "number", "Left edge (canvas is 1280 × 720).", false),
        ("y", "number", "Top edge.", false),
        ("w", "number", "Width.", false),
        ("h", "number", "Height.", false),
        ("text", "string", "Its text; one line per line. With bullets on, each line is a bullet point.", false),
        ("size", "number", "Text size in points (titles ~36-48, body ~18-24).", false),
        ("bold", "boolean", "", false),
        ("italic", "boolean", "", false),
        ("underline", "boolean", "", false),
        ("color", "string", "Text colour \"#RRGGBB\".", false),
        ("font", "string", "\"Sans\", \"Serif\" or \"Mono\".", false),
        ("align", "string", "left, center or right.", false),
        ("valign", "string", "top, middle or bottom.", false),
        ("bullets", "boolean", "Each line a bullet point.", false),
        ("fill", "string", "Fill \"#RRGGBB\", or \"\" for none.", false),
        ("outline", "string", "Outline (or line) colour \"#RRGGBB\", or \"\" for none.", false),
        ("outline_width", "number", "Outline width.", false),
    ]
}

pub fn list() -> Value {
    let mut add = vec![
        ("slide", "integer", "Slide number (from 1).", true),
        ("kind", "string", "text, rect, ellipse or line.", true),
    ];
    add.extend(element_props());
    let mut update = vec![("slide", "integer", "Slide number (from 1).", true), ("id", "integer", "The element's id.", true)];
    update.extend(element_props());
    json!([
        tool(
            "get_presentation",
            "The presentation: every slide (or one) with its background and elements (id, kind, position, text and style), the slide the user is on and what they selected.",
            &[("slide", "integer", "Only this slide (from 1).", false)]
        ),
        tool(
            "add_slide",
            "Adds a slide laid out like Google Slides' layouts, in the deck's colours, and goes to it.",
            &[
                ("at", "integer", "Its number once added (default: after the current slide).", false),
                ("layout", "string", "title, title_and_body, section or blank.", true),
                ("title", "string", "Its title.", false),
                ("body", "string", "Body text (title_and_body: one bullet per line; title: the subtitle).", false),
            ]
        ),
        tool("delete_slide", "Deletes a slide.", &[("slide", "integer", "Slide number.", true)]),
        tool(
            "move_slide",
            "Moves a slide to another place.",
            &[("slide", "integer", "Slide number.", true), ("to", "integer", "Its new number.", true)]
        ),
        tool("duplicate_slide", "Copies a slide to right after it.", &[("slide", "integer", "Slide number.", true)]),
        tool("add_element", "Adds a text box, rectangle, ellipse or line to a slide.", &add),
        tool("update_element", "Changes an element: give only what should change.", &update),
        tool(
            "delete_element",
            "Removes an element from a slide.",
            &[("slide", "integer", "Slide number.", true), ("id", "integer", "The element's id.", true)]
        ),
        tool(
            "set_background",
            "Sets the background colour of one slide, or of all of them.",
            &[("color", "string", "\"#RRGGBB\".", true), ("slide", "integer", "Slide number; every slide if left out.", false)]
        ),
        tool("go_to_slide", "Shows the user a slide.", &[("slide", "integer", "Slide number.", true)]),
    ])
}

pub fn handle(app: &mut App, req: Request) {
    let result = run(app, &req);
    req.reply(result);
}

fn slide_index(app: &App, req: &Request, key: &str) -> Result<usize, String> {
    let n = req.int(key)?;
    let len = app.deck.slides.len();
    if n < 1 || n as usize > len {
        return Err(format!("there are {len} slides (1..{len})"));
    }
    Ok(n as usize - 1)
}

fn describe(e: &Element) -> Value {
    let mut v = json!({
        "id": e.id,
        "kind": e.kind.name(),
        "x": e.x.round(), "y": e.y.round(), "w": e.w.round(), "h": e.h.round(),
    });
    if e.has_text() {
        let s = &e.style;
        v["text"] = json!(e.text);
        v["size"] = json!(s.size.round());
        v["color"] = json!(deck::hex(s.color));
        if s.bold {
            v["bold"] = json!(true);
        }
        if s.italic {
            v["italic"] = json!(true);
        }
        if s.bullets {
            v["bullets"] = json!(true);
        }
        v["align"] = json!(match s.align {
            Align::Left => "left",
            Align::Center => "center",
            Align::Right => "right",
        });
        if s.font != "Sans" {
            v["font"] = json!(s.font);
        }
    }
    if let Some(f) = e.fill {
        v["fill"] = json!(deck::hex(f));
    }
    if let Some(s) = e.stroke {
        v["outline"] = json!(deck::hex(s));
    }
    v
}

fn color(v: &Value) -> Result<Option<Option<deck::Rgb>>, String> {
    match v.as_str() {
        None => Ok(None),
        Some("") => Ok(Some(None)),
        Some(s) => deck::parse_hex(s).map(|c| Some(Some(c))).ok_or_else(|| format!("\"{s}\" isn't #RRGGBB")),
    }
}

fn apply(e: &mut Element, a: &Value) -> Result<(), String> {
    for (k, field) in [("x", &mut e.x), ("y", &mut e.y), ("w", &mut e.w), ("h", &mut e.h)] {
        if let Some(v) = a[k].as_f64() {
            *field = v as f32;
        }
    }
    let s = &mut e.style;
    if let Some(t) = a["text"].as_str() {
        e.text = t.to_owned();
    }
    if let Some(v) = a["size"].as_f64() {
        s.size = (v as f32).clamp(4.0, 300.0);
    }
    if let Some(b) = a["bold"].as_bool() {
        s.bold = b;
    }
    if let Some(b) = a["italic"].as_bool() {
        s.italic = b;
    }
    if let Some(b) = a["underline"].as_bool() {
        s.underline = b;
    }
    if let Some(b) = a["bullets"].as_bool() {
        s.bullets = b;
    }
    if let Some(Some(c)) = color(&a["color"])? {
        s.color = c;
    }
    if let Some(f) = a["font"].as_str() {
        s.font = match f.to_lowercase().as_str() {
            "serif" => "Serif",
            "mono" | "monospace" => "Mono",
            _ => "Sans",
        }
        .to_owned();
    }
    if let Some(al) = a["align"].as_str() {
        s.align = match al {
            "center" | "centre" => Align::Center,
            "right" => Align::Right,
            _ => Align::Left,
        };
    }
    if let Some(v) = a["valign"].as_str() {
        s.valign = match v {
            "middle" | "center" => VAlign::Middle,
            "bottom" => VAlign::Bottom,
            _ => VAlign::Top,
        };
    }
    if let Some(f) = color(&a["fill"])? {
        e.fill = f;
    }
    if let Some(o) = color(&a["outline"])? {
        e.stroke = o;
    }
    if let Some(w) = a["outline_width"].as_f64() {
        e.stroke_w = w as f32;
    }
    Ok(())
}

fn run(app: &mut App, req: &Request) -> Result<String, String> {
    match req.tool.as_str() {
        "get_presentation" => {
            let only = req.opt_int("slide");
            let slides: Vec<Value> = app
                .deck
                .slides
                .iter()
                .enumerate()
                .filter(|(i, _)| only.is_none_or(|n| n as usize == i + 1))
                .map(|(i, s)| {
                    json!({
                        "slide": i + 1,
                        "background": deck::hex(s.background),
                        "elements": s.elements.iter().map(describe).collect::<Vec<_>>(),
                    })
                })
                .collect();
            Ok(json!({
                "slides_count": app.deck.slides.len(),
                "current_slide": app.current + 1,
                "selected_element": app.selected,
                "canvas": "1280 x 720",
                "slides": slides,
            })
            .to_string())
        }
        "add_slide" => {
            let at = match req.opt_int("at") {
                Some(n) => (n.max(1) as usize - 1).min(app.deck.slides.len()),
                None => app.current + 1,
            };
            let layout = Layout::parse(req.str("layout")?);
            let body: Vec<String> = req
                .opt_str("body")
                .unwrap_or_default()
                .lines()
                .map(|l| l.trim_start().trim_start_matches(['-', '*', '•']).trim_start().to_owned())
                .collect();
            app.stop_editing();
            app.checkpoint();
            let s = app.deck.new_slide(layout, req.opt_str("title").unwrap_or_default(), &body.join("\n"));
            app.deck.slides.insert(at, s);
            app.go_to(at);
            let ids: Vec<u64> = app.deck.slides[at].elements.iter().map(|e| e.id).collect();
            Ok(format!("added slide {} (element ids {ids:?})", at + 1))
        }
        "delete_slide" => {
            let i = slide_index(app, req, "slide")?;
            if app.deck.slides.len() == 1 {
                return Err("it's the only slide".to_owned());
            }
            app.checkpoint();
            app.deck.slides.remove(i);
            app.go_to(i.min(app.deck.slides.len() - 1));
            Ok("deleted".to_owned())
        }
        "move_slide" => {
            let i = slide_index(app, req, "slide")?;
            let to = (req.int("to")?.max(1) as usize - 1).min(app.deck.slides.len() - 1);
            app.checkpoint();
            let s = app.deck.slides.remove(i);
            app.deck.slides.insert(to, s);
            app.go_to(to);
            Ok(format!("now slide {}", to + 1))
        }
        "duplicate_slide" => {
            let i = slide_index(app, req, "slide")?;
            app.checkpoint();
            let mut s = app.deck.slides[i].clone();
            for e in &mut s.elements {
                e.id = app.deck.id();
            }
            app.deck.slides.insert(i + 1, s);
            app.go_to(i + 1);
            Ok(format!("copied to slide {}", i + 2))
        }
        "add_element" => {
            let i = slide_index(app, req, "slide")?;
            let kind = match req.str("kind")? {
                "text" => Kind::Text,
                "rect" | "rectangle" => Kind::Rect,
                "ellipse" | "circle" => Kind::Ellipse,
                "line" => Kind::Line,
                other => return Err(format!("unknown kind {other}")),
            };
            app.checkpoint();
            let id = app.deck.id();
            let mut e = Element::new(id, kind, 200.0, 200.0, 400.0, if kind == Kind::Line { 0.0 } else { 120.0 });
            let bg = app.deck.slides[i].background;
            e.style.color = if deck::luminance(bg) < 0.5 { [255, 255, 255] } else { [0, 0, 0] };
            apply(&mut e, &req.args)?;
            app.deck.slides[i].elements.push(e);
            app.go_to(i);
            app.selected = Some(id);
            Ok(format!("added element {id}"))
        }
        "update_element" => {
            let i = slide_index(app, req, "slide")?;
            let id = req.int("id")? as u64;
            if !app.deck.slides[i].elements.iter().any(|e| e.id == id) {
                return Err(format!("no element {id} on slide {}", i + 1));
            }
            app.stop_editing();
            app.checkpoint();
            let e = app.deck.slides[i].elements.iter_mut().find(|e| e.id == id).unwrap();
            apply(e, &req.args)?;
            Ok("updated".to_owned())
        }
        "delete_element" => {
            let i = slide_index(app, req, "slide")?;
            let id = req.int("id")? as u64;
            app.stop_editing();
            app.checkpoint();
            let before = app.deck.slides[i].elements.len();
            app.deck.slides[i].elements.retain(|e| e.id != id);
            if app.deck.slides[i].elements.len() == before {
                return Err(format!("no element {id} on slide {}", i + 1));
            }
            Ok("deleted".to_owned())
        }
        "set_background" => {
            let c = color(&req.args["color"])?.flatten().ok_or("color should be #RRGGBB")?;
            app.checkpoint();
            match req.opt_int("slide") {
                Some(_) => {
                    let i = slide_index(app, req, "slide")?;
                    app.deck.slides[i].background = c;
                }
                None => {
                    for s in &mut app.deck.slides {
                        s.background = c;
                    }
                }
            }
            Ok("done".to_owned())
        }
        "go_to_slide" => {
            let i = slide_index(app, req, "slide")?;
            app.go_to(i);
            Ok("shown".to_owned())
        }
        other => Err(format!("unknown tool {other}")),
    }
}
