//! PowerPoint files. Saving writes a plain, valid .pptx: one blank layout,
//! each slide's background, and its elements as text boxes, shapes, lines
//! and pictures with their own colours, so it looks the same in
//! PowerPoint, Keynote and Google Slides. Opening reads other people's
//! decks too: placeholders positioned by their layout or master, theme
//! colours, groups, pictures; tables come in as text, charts are left out.

use crate::deck::{Align, Deck, Element, Kind, Rgb, Slide, TextStyle, VAlign, H, W};
use crate::xml::{self, Node, esc};
use std::collections::HashMap;
use std::io::{Read, Write};
use zip::write::SimpleFileOptions;

const EMU: f32 = 9525.0; // per canvas pixel (12192000 / 1280)
const NS: &str = r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main""#;
const HEAD: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#;
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

fn emu(px: f32) -> i64 {
    (px * EMU).round() as i64
}

fn rgb(c: Rgb) -> String {
    format!("{:02X}{:02X}{:02X}", c[0], c[1], c[2])
}

fn font_name(f: &str) -> &'static str {
    match f {
        "Serif" => "Georgia",
        "Mono" => "Courier New",
        _ => "Arial",
    }
}

const GROUP_HEAD: &str = r#"<p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr>"#;

fn theme_xml() -> String {
    let colors = [
        ("dk1", "000000"),
        ("lt1", "FFFFFF"),
        ("dk2", "1F1F1F"),
        ("lt2", "E7E6E6"),
        ("accent1", "595959"),
        ("accent2", "7F7F7F"),
        ("accent3", "A5A5A5"),
        ("accent4", "404040"),
        ("accent5", "262626"),
        ("accent6", "BFBFBF"),
        ("hlink", "0563C1"),
        ("folHlink", "954F72"),
    ];
    let clr: String = colors.iter().map(|(n, c)| format!("<a:{n}><a:srgbClr val=\"{c}\"/></a:{n}>")).collect();
    let solid = r#"<a:solidFill><a:schemeClr val="phClr"/></a:solidFill>"#;
    let ln = r#"<a:ln w="9525"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln>"#;
    let eff = "<a:effectStyle><a:effectLst/></a:effectStyle>";
    format!(
        r#"{HEAD}<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="Office"><a:themeElements><a:clrScheme name="Office">{clr}</a:clrScheme><a:fontScheme name="Office"><a:majorFont><a:latin typeface="Arial"/><a:ea typeface=""/><a:cs typeface=""/></a:majorFont><a:minorFont><a:latin typeface="Arial"/><a:ea typeface=""/><a:cs typeface=""/></a:minorFont></a:fontScheme><a:fmtScheme name="Office"><a:fillStyleLst>{solid}{solid}{solid}</a:fillStyleLst><a:lnStyleLst>{ln}{ln}{ln}</a:lnStyleLst><a:effectStyleLst>{eff}{eff}{eff}</a:effectStyleLst><a:bgFillStyleLst>{solid}{solid}{solid}</a:bgFillStyleLst></a:fmtScheme></a:themeElements><a:objectDefaults/><a:extraClrSchemeLst/></a:theme>"#
    )
}

fn text_body(e: &Element) -> String {
    let s = &e.style;
    let anchor = match s.valign {
        VAlign::Top => "t",
        VAlign::Middle => "ctr",
        VAlign::Bottom => "b",
    };
    let algn = match s.align {
        Align::Left => "l",
        Align::Center => "ctr",
        Align::Right => "r",
    };
    let size = (s.size * 100.0).round() as i64;
    let mut rpr = format!(r#"lang="en-US" sz="{size}""#);
    if s.bold {
        rpr.push_str(r#" b="1""#);
    }
    if s.italic {
        rpr.push_str(r#" i="1""#);
    }
    if s.underline {
        rpr.push_str(r#" u="sng""#);
    }
    let font = font_name(&s.font);
    let props = format!(
        r#"<a:rPr {rpr} dirty="0"><a:solidFill><a:srgbClr val="{}"/></a:solidFill><a:latin typeface="{font}"/></a:rPr>"#,
        rgb(s.color)
    );
    let ppr = if s.bullets {
        format!(r#"<a:pPr marL="342900" indent="-342900" algn="{algn}"><a:buFont typeface="Arial"/><a:buChar char="•"/></a:pPr>"#)
    } else {
        format!(r#"<a:pPr algn="{algn}"><a:buNone/></a:pPr>"#)
    };
    let mut paras = String::new();
    for line in e.text.split('\n') {
        paras.push_str("<a:p>");
        paras.push_str(&ppr);
        if !line.is_empty() {
            paras.push_str(&format!("<a:r>{props}<a:t>{}</a:t></a:r>", esc(line)));
        }
        paras.push_str(&format!(r#"<a:endParaRPr {rpr} dirty="0"/></a:p>"#));
    }
    format!(
        r#"<p:txBody><a:bodyPr wrap="square" lIns="91440" tIns="45720" rIns="91440" bIns="45720" rtlCol="0" anchor="{anchor}"><a:noAutofit/></a:bodyPr><a:lstStyle/>{paras}</p:txBody>"#
    )
}

fn slide_xml(slide: &Slide, image_rels: &HashMap<String, String>) -> String {
    let mut shapes = String::new();
    for (i, e) in slide.elements.iter().enumerate() {
        let id = i + 2;
        let xfrm = |flip: &str| {
            format!(
                r#"<a:xfrm{flip}><a:off x="{}" y="{}"/><a:ext cx="{}" cy="{}"/></a:xfrm>"#,
                emu(e.x),
                emu(e.y),
                emu(e.w.max(1.0)),
                emu(e.h.max(1.0))
            )
        };
        let fill = match e.fill {
            Some(c) => format!(r#"<a:solidFill><a:srgbClr val="{}"/></a:solidFill>"#, rgb(c)),
            None => "<a:noFill/>".to_owned(),
        };
        let line = match e.stroke {
            Some(c) => format!(r#"<a:ln w="{}"><a:solidFill><a:srgbClr val="{}"/></a:solidFill></a:ln>"#, emu(e.stroke_w), rgb(c)),
            None => "<a:ln><a:noFill/></a:ln>".to_owned(),
        };
        match e.kind {
            Kind::Text | Kind::Rect | Kind::Ellipse => {
                let (geom, name, tx) = match e.kind {
                    Kind::Text => ("rect", "TextBox", r#" txBox="1""#),
                    Kind::Rect => ("rect", "Rectangle", ""),
                    _ => ("ellipse", "Oval", ""),
                };
                shapes.push_str(&format!(
                    r#"<p:sp><p:nvSpPr><p:cNvPr id="{id}" name="{name} {id}"/><p:cNvSpPr{tx}/><p:nvPr/></p:nvSpPr><p:spPr>{}<a:prstGeom prst="{geom}"><a:avLst/></a:prstGeom>{fill}{line}</p:spPr>{}</p:sp>"#,
                    xfrm(""),
                    text_body(e)
                ));
            }
            Kind::Line => {
                let flip = if e.flip { r#" flipV="1""# } else { "" };
                shapes.push_str(&format!(
                    r#"<p:cxnSp><p:nvCxnSpPr><p:cNvPr id="{id}" name="Line {id}"/><p:cNvCxnSpPr/><p:nvPr/></p:nvCxnSpPr><p:spPr>{}<a:prstGeom prst="line"><a:avLst/></a:prstGeom>{line}</p:spPr></p:cxnSp>"#,
                    xfrm(flip)
                ));
            }
            Kind::Image => {
                let Some(rid) = e.image.as_ref().and_then(|k| image_rels.get(k)) else { continue };
                shapes.push_str(&format!(
                    r#"<p:pic><p:nvPicPr><p:cNvPr id="{id}" name="Picture {id}"/><p:cNvPicPr><a:picLocks noChangeAspect="1"/></p:cNvPicPr><p:nvPr/></p:nvPicPr><p:blipFill><a:blip r:embed="{rid}"/><a:stretch><a:fillRect/></a:stretch></p:blipFill><p:spPr>{}<a:prstGeom prst="rect"><a:avLst/></a:prstGeom></p:spPr></p:pic>"#,
                    xfrm("")
                ));
            }
        }
    }
    format!(
        r#"{HEAD}<p:sld {NS}><p:cSld><p:bg><p:bgPr><a:solidFill><a:srgbClr val="{}"/></a:solidFill><a:effectLst/></p:bgPr></p:bg><p:spTree>{GROUP_HEAD}{shapes}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>"#,
        rgb(slide.background)
    )
}

fn rels(items: &[(String, &str, String)]) -> String {
    let body: String = items
        .iter()
        .map(|(id, ty, target)| format!(r#"<Relationship Id="{id}" Type="{ty}" Target="{target}"/>"#))
        .collect();
    format!(r#"{HEAD}<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{body}</Relationships>"#)
}

pub fn save(deck: &Deck, path: &std::path::Path) -> Result<(), String> {
    let tmp = path.with_extension("pptx.saving");
    let file = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipWriter::new(file);
    let opt = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let mut put = |name: &str, data: &[u8]| -> Result<(), String> {
        zip.start_file(name, opt).map_err(|e| e.to_string())?;
        zip.write_all(data).map_err(|e| e.to_string())
    };
    let n = deck.slides.len();
    let mut types = String::from(
        r#"<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="png" ContentType="image/png"/><Default Extension="jpeg" ContentType="image/jpeg"/><Default Extension="jpg" ContentType="image/jpeg"/><Default Extension="gif" ContentType="image/gif"/><Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/><Override PartName="/ppt/slideMasters/slideMaster1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml"/><Override PartName="/ppt/slideLayouts/slideLayout1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml"/><Override PartName="/ppt/theme/theme1.xml" ContentType="application/vnd.openxmlformats-officedocument.theme+xml"/><Override PartName="/ppt/presProps.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presProps+xml"/><Override PartName="/ppt/viewProps.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.viewProps+xml"/><Override PartName="/ppt/tableStyles.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.tableStyles+xml"/><Override PartName="/docProps/core.xml" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/><Override PartName="/docProps/app.xml" ContentType="application/vnd.openxmlformats-officedocument.extended-properties+xml"/>"#,
    );
    for i in 1..=n {
        types.push_str(&format!(
            r#"<Override PartName="/ppt/slides/slide{i}.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/>"#
        ));
    }
    put("[Content_Types].xml", format!(r#"{HEAD}<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">{types}</Types>"#).as_bytes())?;
    put(
        "_rels/.rels",
        rels(&[
            ("rId1".into(), "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument", "ppt/presentation.xml".into()),
            ("rId2".into(), "http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties", "docProps/core.xml".into()),
            ("rId3".into(), "http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties", "docProps/app.xml".into()),
        ])
        .as_bytes(),
    )?;
    put(
        "docProps/core.xml",
        format!(r#"{HEAD}<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"><dc:title>Presentation</dc:title></cp:coreProperties>"#).as_bytes(),
    )?;
    put(
        "docProps/app.xml",
        format!(r#"{HEAD}<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties"><Application>Slides</Application><Slides>{n}</Slides></Properties>"#).as_bytes(),
    )?;

    let mut pres_rels = vec![(
        "rId1".to_owned(),
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster",
        "slideMasters/slideMaster1.xml".to_owned(),
    )];
    let mut ids = String::new();
    for i in 1..=n {
        pres_rels.push((format!("rId{}", i + 1), "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide", format!("slides/slide{i}.xml")));
        ids.push_str(&format!(r#"<p:sldId id="{}" r:id="rId{}"/>"#, 255 + i, i + 1));
    }
    for (k, ty, target) in [
        ("theme", "theme", "theme/theme1.xml"),
        ("presProps", "presProps", "presProps.xml"),
        ("viewProps", "viewProps", "viewProps.xml"),
        ("tableStyles", "tableStyles", "tableStyles.xml"),
    ] {
        let _ = k;
        pres_rels.push((format!("rId{}", pres_rels.len() + 1), Box::leak(format!("{REL}/{ty}").into_boxed_str()), target.to_owned()));
    }
    put("ppt/_rels/presentation.xml.rels", rels(&pres_rels).as_bytes())?;
    put(
        "ppt/presentation.xml",
        format!(
            r#"{HEAD}<p:presentation {NS} saveSubsetFonts="1"><p:sldMasterIdLst><p:sldMasterId id="2147483648" r:id="rId1"/></p:sldMasterIdLst><p:sldIdLst>{ids}</p:sldIdLst><p:sldSz cx="12192000" cy="6858000"/><p:notesSz cx="6858000" cy="9144000"/></p:presentation>"#
        )
        .as_bytes(),
    )?;
    put("ppt/presProps.xml", format!(r#"{HEAD}<p:presentationPr {NS}/>"#).as_bytes())?;
    put("ppt/viewProps.xml", format!(r#"{HEAD}<p:viewPr {NS}/>"#).as_bytes())?;
    put(
        "ppt/tableStyles.xml",
        format!(r#"{HEAD}<a:tblStyleLst xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" def="{{5C22544A-7EE6-4342-B048-85BDC9FD1C3A}}"/>"#).as_bytes(),
    )?;
    put("ppt/theme/theme1.xml", theme_xml().as_bytes())?;
    put(
        "ppt/slideMasters/slideMaster1.xml",
        format!(
            r#"{HEAD}<p:sldMaster {NS}><p:cSld><p:bg><p:bgRef idx="1001"><a:schemeClr val="bg1"/></p:bgRef></p:bg><p:spTree>{GROUP_HEAD}</p:spTree></p:cSld><p:clrMap bg1="lt1" tx1="dk1" bg2="lt2" tx2="dk2" accent1="accent1" accent2="accent2" accent3="accent3" accent4="accent4" accent5="accent5" accent6="accent6" hlink="hlink" folHlink="folHlink"/><p:sldLayoutIdLst><p:sldLayoutId id="2147483649" r:id="rId1"/></p:sldLayoutIdLst><p:txStyles><p:titleStyle/><p:bodyStyle/><p:otherStyle/></p:txStyles></p:sldMaster>"#
        )
        .as_bytes(),
    )?;
    put(
        "ppt/slideMasters/_rels/slideMaster1.xml.rels",
        rels(&[
            ("rId1".into(), "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout", "../slideLayouts/slideLayout1.xml".into()),
            ("rId2".into(), "http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme", "../theme/theme1.xml".into()),
        ])
        .as_bytes(),
    )?;
    put(
        "ppt/slideLayouts/slideLayout1.xml",
        format!(r#"{HEAD}<p:sldLayout {NS} type="blank" preserve="1"><p:cSld name="Blank"><p:spTree>{GROUP_HEAD}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>"#).as_bytes(),
    )?;
    put(
        "ppt/slideLayouts/_rels/slideLayout1.xml.rels",
        rels(&[("rId1".into(), "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster", "../slideMasters/slideMaster1.xml".into())]).as_bytes(),
    )?;

    // Pictures, each stored once, named as in the deck.
    let mut media_done = std::collections::HashSet::new();
    for (i, slide) in deck.slides.iter().enumerate() {
        let mut r = vec![("rId1".to_owned(), "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout", "../slideLayouts/slideLayout1.xml".to_owned())];
        let mut image_rels = HashMap::new();
        for e in &slide.elements {
            if let Some(key) = &e.image {
                if image_rels.contains_key(key) || !deck.images.contains_key(key) {
                    continue;
                }
                let rid = format!("rId{}", r.len() + 1);
                r.push((rid.clone(), "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image", format!("../media/{key}")));
                image_rels.insert(key.clone(), rid);
                if media_done.insert(key.clone()) {
                    put(&format!("ppt/media/{key}"), &deck.images[key])?;
                }
            }
        }
        put(&format!("ppt/slides/_rels/slide{}.xml.rels", i + 1), rels(&r).as_bytes())?;
        put(&format!("ppt/slides/slide{}.xml", i + 1), slide_xml(slide, &image_rels).as_bytes())?;
    }
    zip.finish().map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

// ---- Opening ---------------------------------------------------------------

struct Pkg {
    zip: zip::ZipArchive<std::io::Cursor<Vec<u8>>>,
}

impl Pkg {
    fn bytes(&mut self, name: &str) -> Option<Vec<u8>> {
        let mut f = self.zip.by_name(name.trim_start_matches('/')).ok()?;
        let mut v = Vec::new();
        f.read_to_end(&mut v).ok()?;
        Some(v)
    }

    fn xml(&mut self, name: &str) -> Option<Node> {
        let b = self.bytes(name)?;
        xml::parse(&String::from_utf8_lossy(&b)).ok()
    }

    /// A part's relationships, by id: (type's last word, absolute target).
    fn rels(&mut self, part: &str) -> HashMap<String, (String, String)> {
        let (dir, file) = part.rsplit_once('/').unwrap_or(("", part));
        let mut out = HashMap::new();
        if let Some(r) = self.xml(&format!("{dir}/_rels/{file}.rels")) {
            for rel in r.children_named("Relationship") {
                let id = rel.attr("Id").unwrap_or_default().to_owned();
                let ty = rel.attr("Type").unwrap_or_default().rsplit('/').next().unwrap_or_default().to_owned();
                let target = rel.attr("Target").unwrap_or_default();
                out.insert(id, (ty, resolve(dir, target)));
            }
        }
        out
    }
}

/// A relationship target relative to its part's folder, as a path in the zip.
fn resolve(dir: &str, target: &str) -> String {
    if let Some(abs) = target.strip_prefix('/') {
        return abs.to_owned();
    }
    let mut parts: Vec<&str> = dir.split('/').filter(|s| !s.is_empty()).collect();
    for seg in target.split('/') {
        match seg {
            ".." => {
                parts.pop();
            }
            "." | "" => {}
            s => parts.push(s),
        }
    }
    parts.join("/")
}

/// Theme colours by name (dk1, lt1, accent1...) and the master's mapping
/// of tx1/bg1... onto them.
struct Colors {
    scheme: HashMap<String, Rgb>,
    map: HashMap<String, String>,
}

impl Colors {
    fn get(&self, node: &Node) -> Option<Rgb> {
        let (base, mods) = if let Some(c) = node.child("srgbClr") {
            (crate::deck::parse_hex(c.attr("val")?), c)
        } else if let Some(c) = node.child("schemeClr") {
            let name = c.attr("val")?;
            let name = self.map.get(name).map(|s| s.as_str()).unwrap_or(name);
            (self.scheme.get(name).copied(), c)
        } else if let Some(c) = node.child("sysClr") {
            (crate::deck::parse_hex(c.attr("lastClr").unwrap_or("000000")), c)
        } else if let Some(c) = node.child("prstClr") {
            (Some(if c.attr("val") == Some("white") { [255, 255, 255] } else { [0, 0, 0] }), c)
        } else {
            return None;
        };
        let mut c = base?.map(|v| v as f32);
        let val = |n: &str| mods.child(n).and_then(|m| m.attr("val")).and_then(|v| v.parse::<f32>().ok()).map(|v| v / 100000.0);
        if let Some(m) = val("lumMod") {
            c = c.map(|v| v * m);
        }
        if let Some(o) = val("lumOff") {
            c = c.map(|v| v + 255.0 * o);
        }
        if let Some(t) = val("tint") {
            c = c.map(|v| v + (255.0 - v) * (1.0 - t));
        }
        if let Some(s) = val("shade") {
            c = c.map(|v| v * s);
        }
        Some(c.map(|v| v.clamp(0.0, 255.0).round() as u8))
    }
}

/// What a placeholder on a layout or master says about position and text.
#[derive(Clone, Default)]
struct PhInfo {
    rect: Option<(f32, f32, f32, f32)>,
    size: Option<f32>,
    anchor: Option<VAlign>,
    align: Option<Align>,
}

struct Ctx<'a> {
    colors: &'a Colors,
    /// Layout's placeholders, then master's.
    phs: Vec<HashMap<String, PhInfo>>,
    master_styles: HashMap<&'static str, f32>,
    master_align: HashMap<&'static str, Align>,
    scale: f32,
    dx: f32,
    dy: f32,
}

fn ph_key(ph: &Node) -> (Option<String>, String) {
    let ty = ph.attr("type").unwrap_or("body").to_owned();
    let ty = match ty.as_str() {
        "ctrTitle" => "title".to_owned(),
        "subTitle" | "obj" => "body".to_owned(),
        t => t.to_owned(),
    };
    (ph.attr("idx").map(|s| s.to_owned()), ty)
}

fn xfrm(node: &Node) -> Option<(f32, f32, f32, f32)> {
    let x = node.at(&["spPr", "xfrm"]).or_else(|| node.at(&["xfrm"]))?;
    let off = x.child("off")?;
    let ext = x.child("ext")?;
    let n = |n: &Node, k: &str| n.attr(k).and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0);
    Some((n(off, "x"), n(off, "y"), n(ext, "cx"), n(ext, "cy")))
}

fn first_size(body: &Node) -> Option<f32> {
    let mut rprs = Vec::new();
    body.find_all("rPr", &mut rprs);
    body.find_all("defRPr", &mut rprs);
    rprs.iter().find_map(|r| r.attr("sz")).and_then(|s| s.parse::<f32>().ok()).map(|s| s / 100.0)
}

fn placeholders(tree: &Node) -> HashMap<String, PhInfo> {
    let mut out = HashMap::new();
    let mut sps = Vec::new();
    tree.find_all("sp", &mut sps);
    for sp in sps {
        let Some(ph) = sp.at(&["nvSpPr", "nvPr", "ph"]) else { continue };
        let (idx, ty) = ph_key(ph);
        let body = sp.child("txBody");
        let info = PhInfo {
            rect: xfrm(sp),
            size: body.and_then(|b| b.child("lstStyle")).and_then(first_size),
            anchor: body.and_then(|b| b.child("bodyPr")).and_then(|b| b.attr("anchor")).map(anchor),
            align: body
                .and_then(|b| b.at(&["lstStyle", "lvl1pPr"]))
                .and_then(|p| p.attr("algn"))
                .map(align),
        };
        if let Some(i) = idx {
            out.insert(format!("idx:{i}"), info.clone());
        }
        out.entry(format!("type:{ty}")).or_insert(info);
    }
    out
}

fn anchor(a: &str) -> VAlign {
    match a {
        "ctr" => VAlign::Middle,
        "b" => VAlign::Bottom,
        _ => VAlign::Top,
    }
}

fn align(a: &str) -> Align {
    match a {
        "ctr" => Align::Center,
        "r" => Align::Right,
        _ => Align::Left,
    }
}

/// A group's mapping of its children's coordinates onto the slide.
#[derive(Clone, Copy)]
struct Transform {
    ox: f32,
    oy: f32,
    sx: f32,
    sy: f32,
}

impl Transform {
    fn apply(&self, (x, y, w, h): (f32, f32, f32, f32)) -> (f32, f32, f32, f32) {
        (self.ox + x * self.sx, self.oy + y * self.sy, w * self.sx, h * self.sy)
    }
}

fn read_tree(tree: &Node, cx: &Ctx, t: Transform, pkg: &mut Pkg, rels: &HashMap<String, (String, String)>, deck: &mut Deck, out: &mut Vec<Element>) {
    for node in &tree.children {
        match node.name.as_str() {
            "grpSp" => {
                let Some(x) = node.at(&["grpSpPr", "xfrm"]) else { continue };
                let g = |n: &str, k: &str| x.child(n).and_then(|c| c.attr(k)).and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0);
                let (ox, oy, ex, ey) = (g("off", "x"), g("off", "y"), g("ext", "cx"), g("ext", "cy"));
                let (cox, coy, cex, cey) = (g("chOff", "x"), g("chOff", "y"), g("chExt", "cx"), g("chExt", "cy"));
                let sx = if cex > 0.0 { ex / cex } else { 1.0 };
                let sy = if cey > 0.0 { ey / cey } else { 1.0 };
                let inner = Transform { ox: t.ox + (ox - cox * sx) * t.sx, oy: t.oy + (oy - coy * sy) * t.sy, sx: t.sx * sx, sy: t.sy * sy };
                read_tree(node, cx, inner, pkg, rels, deck, out);
            }
            "sp" => {
                if let Some(e) = read_shape(node, cx, t, deck) {
                    out.push(e);
                }
            }
            "cxnSp" => {
                let Some(r) = xfrm(node) else { continue };
                let (x, y, w, h) = place(cx, t.apply(r));
                let mut e = Element::new(deck.id(), Kind::Line, x, y, w, h);
                let xf = node.at(&["spPr", "xfrm"]);
                e.flip = xf.is_some_and(|x| (x.attr("flipV") == Some("1")) != (x.attr("flipH") == Some("1")));
                let (stroke, width) = line_of(node, cx);
                e.stroke = stroke.or(Some([255, 255, 255]));
                e.stroke_w = width.unwrap_or(2.0);
                out.push(e);
            }
            "pic" => {
                let Some(r) = xfrm(node) else { continue };
                let Some(rid) = node.at(&["blipFill", "blip"]).and_then(|b| b.attr("embed")) else { continue };
                let Some((_, target)) = rels.get(rid) else { continue };
                let Some(bytes) = pkg.bytes(target) else { continue };
                let Some(key) = picture(deck, bytes, target) else { continue };
                let (x, y, w, h) = place(cx, t.apply(r));
                let mut e = Element::new(deck.id(), Kind::Image, x, y, w, h);
                e.image = Some(key);
                out.push(e);
            }
            "graphicFrame" => {
                // Tables come in as text, a line per row.
                let mut tbls = Vec::new();
                node.find_all("tbl", &mut tbls);
                let Some(tbl) = tbls.first() else { continue };
                let Some(r) = xfrm(node) else { continue };
                let mut lines = Vec::new();
                for tr in tbl.children_named("tr") {
                    let cells: Vec<String> = tr.children_named("tc").map(|tc| paragraphs_text(tc).join(" ")).collect();
                    lines.push(cells.join("  |  "));
                }
                let (x, y, w, h) = place(cx, t.apply(r));
                let mut e = Element::new(deck.id(), Kind::Text, x, y, w, h);
                e.text = lines.join("\n");
                e.style.size = 14.0;
                e.style.color = default_text(cx);
                out.push(e);
            }
            _ => {}
        }
    }
}

/// Pictures in a format the screen and PowerPoint both take (PNG or JPEG);
/// others are converted to PNG, and ones that can't be read are left out.
fn picture(deck: &mut Deck, bytes: Vec<u8>, name: &str) -> Option<String> {
    let ext = name.rsplit('.').next().unwrap_or_default().to_lowercase();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" => Some(deck.add_image(bytes, if ext == "png" { "png" } else { "jpeg" })),
        _ => {
            let img = image::load_from_memory(&bytes).ok()?;
            let mut png = Vec::new();
            img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
            Some(deck.add_image(png, "png"))
        }
    }
}

fn place(cx: &Ctx, (x, y, w, h): (f32, f32, f32, f32)) -> (f32, f32, f32, f32) {
    (x * cx.scale + cx.dx, y * cx.scale + cx.dy, w * cx.scale, h * cx.scale)
}

fn default_text(cx: &Ctx) -> Rgb {
    let tx = Node { name: "c".into(), children: vec![Node { name: "schemeClr".into(), attrs: vec![("val".into(), "tx1".into())], ..Default::default() }], ..Default::default() };
    cx.colors.get(&tx).unwrap_or([0, 0, 0])
}

fn line_of(node: &Node, cx: &Ctx) -> (Option<Rgb>, Option<f32>) {
    let ln = node.at(&["spPr", "ln"]);
    let width = ln.and_then(|l| l.attr("w")).and_then(|w| w.parse::<f32>().ok()).map(|w| (w * cx.scale).max(1.0));
    if ln.is_some_and(|l| l.child("noFill").is_some()) {
        return (None, width);
    }
    let color = ln
        .and_then(|l| l.child("solidFill"))
        .and_then(|f| cx.colors.get(f))
        .or_else(|| node.at(&["style", "lnRef"]).and_then(|r| cx.colors.get(r)));
    (color, width)
}

fn paragraphs_text(body: &Node) -> Vec<String> {
    let mut ps = Vec::new();
    body.find_all("p", &mut ps);
    ps.iter()
        .map(|p| {
            let mut s = String::new();
            for c in &p.children {
                match c.name.as_str() {
                    "r" | "fld" => s.push_str(&c.child("t").map(|t| t.text.as_str()).unwrap_or_default()),
                    "br" => s.push(' '),
                    _ => {}
                }
            }
            s
        })
        .collect()
}

fn read_shape(sp: &Node, cx: &Ctx, t: Transform, deck: &mut Deck) -> Option<Element> {
    let ph = sp.at(&["nvSpPr", "nvPr", "ph"]);
    let inherited: Vec<PhInfo> = ph
        .map(|ph| {
            let (idx, ty) = ph_key(ph);
            cx.phs
                .iter()
                .filter_map(|m| idx.as_ref().and_then(|i| m.get(&format!("idx:{i}"))).or_else(|| m.get(&format!("type:{ty}"))).cloned())
                .collect()
        })
        .unwrap_or_default();
    let rect = xfrm(sp).map(|r| t.apply(r)).or_else(|| inherited.iter().find_map(|i| i.rect))?;
    let (x, y, w, h) = place(cx, rect);
    let geom = sp.at(&["spPr", "prstGeom"]).and_then(|g| g.attr("prst")).unwrap_or("rect");
    let body = sp.child("txBody");
    let lines = body.map(paragraphs_text).unwrap_or_default();
    let has_text = lines.iter().any(|l| !l.trim().is_empty());
    let spfill = sp.child("spPr");
    let fill = if spfill.and_then(|s| s.child("noFill")).is_some() {
        None
    } else {
        spfill
            .and_then(|s| s.child("solidFill"))
            .and_then(|f| cx.colors.get(f))
            .or_else(|| {
                // Shapes drawn in PowerPoint take their fill from the theme.
                let r = sp.at(&["style", "fillRef"])?;
                (r.attr("idx") != Some("0")).then(|| cx.colors.get(r)).flatten()
            })
    };
    let (stroke, width) = line_of(sp, cx);
    let kind = match geom {
        "ellipse" => Kind::Ellipse,
        _ if sp.at(&["nvSpPr", "cNvSpPr"]).and_then(|c| c.attr("txBox")) == Some("1") || ph.is_some() => Kind::Text,
        _ if fill.is_none() && stroke.is_none() => Kind::Text,
        _ => Kind::Rect,
    };
    if kind == Kind::Text && !has_text {
        // Empty placeholders show nothing in a presentation.
        return None;
    }
    let mut e = Element::new(deck.id(), kind, x, y, w, h);
    e.fill = if kind == Kind::Text { fill } else { fill.or(None) };
    e.stroke = stroke;
    if let Some(wd) = width {
        e.stroke_w = wd;
    }
    e.text = lines.join("\n");

    // The text's look: its first run, else what its placeholder inherits.
    let ph_type = ph.map(|p| ph_key(p).1).unwrap_or_default();
    let mut style = TextStyle::default();
    let rpr = body.and_then(|b| {
        let mut v = Vec::new();
        b.find_all("rPr", &mut v);
        v.into_iter().next().cloned()
    });
    let size = rpr
        .as_ref()
        .and_then(|r| r.attr("sz"))
        .and_then(|s| s.parse::<f32>().ok())
        .map(|s| s / 100.0)
        .or_else(|| body.and_then(|b| b.child("lstStyle")).and_then(first_size))
        .or_else(|| inherited.iter().find_map(|i| i.size))
        .or_else(|| cx.master_styles.get(if ph_type == "title" { "title" } else if ph.is_some() { "body" } else { "other" }).copied())
        .unwrap_or(18.0);
    style.size = size * cx.scale * 12700.0 * 0.75;
    if let Some(r) = &rpr {
        style.bold = r.attr("b") == Some("1");
        style.italic = r.attr("i") == Some("1");
        style.underline = r.attr("u").is_some_and(|u| u != "none");
        let face = r.child("latin").and_then(|l| l.attr("typeface")).unwrap_or_default().to_lowercase();
        if face.contains("mono") || face.contains("courier") || face.contains("consol") {
            style.font = "Mono".into();
        } else if face.contains("georgia") || face.contains("times") || face.contains("garamond") || (face.contains("serif") && !face.contains("sans")) {
            style.font = "Serif".into();
        }
    }
    style.color = rpr
        .as_ref()
        .and_then(|r| r.child("solidFill"))
        .and_then(|f| cx.colors.get(f))
        .or_else(|| sp.at(&["style", "fontRef"]).and_then(|f| cx.colors.get(f)))
        .unwrap_or_else(|| default_text(cx));
    let ppr = body.and_then(|b| {
        let mut v = Vec::new();
        b.find_all("pPr", &mut v);
        v.into_iter().next().cloned()
    });
    style.align = ppr
        .as_ref()
        .and_then(|p| p.attr("algn"))
        .map(align)
        .or_else(|| inherited.iter().find_map(|i| i.align))
        .or_else(|| ph.and_then(|_| cx.master_align.get(if ph_type == "title" { "title" } else { "body" }).copied()))
        .unwrap_or(if kind == Kind::Text { Align::Left } else { Align::Center });
    style.valign = body
        .and_then(|b| b.child("bodyPr"))
        .and_then(|b| b.attr("anchor"))
        .map(anchor)
        .or_else(|| inherited.iter().find_map(|i| i.anchor))
        .unwrap_or(if kind == Kind::Text { VAlign::Top } else { VAlign::Middle });
    let bu_none = ppr.as_ref().is_some_and(|p| p.child("buNone").is_some());
    let bu_char = ppr.as_ref().is_some_and(|p| p.child("buChar").is_some() || p.child("buAutoNum").is_some());
    style.bullets = bu_char || (ph.is_some() && ph_type == "body" && ph.and_then(|p| p.attr("type")).is_none_or(|t| t == "body" || t == "obj") && !bu_none && lines.len() > 1);
    e.style = style;
    Some(e)
}

fn background(node: &Node, cx: &Colors) -> Option<Rgb> {
    let bg = node.at(&["cSld", "bg"])?;
    if let Some(p) = bg.child("bgPr") {
        return p.child("solidFill").and_then(|f| cx.get(f)).or_else(|| {
            // A gradient: its first colour.
            let mut stops = Vec::new();
            p.find_all("gs", &mut stops);
            stops.first().and_then(|s| cx.get(s))
        });
    }
    bg.child("bgRef").and_then(|r| cx.get(r))
}

pub fn load(bytes: Vec<u8>) -> Result<Deck, String> {
    let zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let mut pkg = Pkg { zip };
    let pres = pkg.xml("ppt/presentation.xml").ok_or("not a PowerPoint file")?;
    let pres_rels = pkg.rels("ppt/presentation.xml");
    let size = pres.child("sldSz");
    let num = |k: &str, d: f32| size.and_then(|s| s.attr(k)).and_then(|v| v.parse::<f32>().ok()).unwrap_or(d);
    let (cw, ch) = (num("cx", 12192000.0), num("cy", 6858000.0));
    let scale = (W / cw).min(H / ch);
    let (dx, dy) = ((W - cw * scale) / 2.0, (H - ch * scale) / 2.0);

    let mut deck = Deck::default();
    deck.slides.clear();
    let slide_ids: Vec<String> = pres
        .at(&["sldIdLst"])
        .map(|l| l.children_named("sldId").filter_map(|s| s.attr("id").and(s.attr("r:id")).map(|s| s.to_owned())).collect())
        .unwrap_or_default();
    for rid in slide_ids {
        let Some((_, path)) = pres_rels.get(&rid).cloned() else { continue };
        let Some(slide) = pkg.xml(&path) else { continue };
        let rels = pkg.rels(&path);
        let layout_path = rels.values().find(|(t, _)| t == "slideLayout").map(|(_, p)| p.clone());
        let layout = layout_path.as_ref().and_then(|p| pkg.xml(p));
        let master_path = layout_path.as_ref().and_then(|p| pkg.rels(p).values().find(|(t, _)| t == "slideMaster").map(|(_, p)| p.clone()));
        let master = master_path.as_ref().and_then(|p| pkg.xml(p));
        let theme = master_path.as_ref().and_then(|p| pkg.rels(p).values().find(|(t, _)| t == "theme").map(|(_, p)| p.clone())).and_then(|p| pkg.xml(&p));

        let mut scheme = HashMap::new();
        if let Some(cs) = theme.as_ref().and_then(|t| t.at(&["themeElements", "clrScheme"])) {
            for c in &cs.children {
                let wrapped = Node { name: "c".into(), children: c.children.clone(), ..Default::default() };
                let tmp = Colors { scheme: HashMap::new(), map: HashMap::new() };
                if let Some(rgb) = tmp.get(&wrapped) {
                    scheme.insert(c.name.clone(), rgb);
                }
            }
        }
        let mut map = HashMap::new();
        if let Some(m) = master.as_ref().and_then(|m| m.child("clrMap")) {
            for (k, v) in &m.attrs {
                map.insert(k.clone(), v.clone());
            }
        }
        for (k, v) in [("bg1", "lt1"), ("tx1", "dk1"), ("bg2", "lt2"), ("tx2", "dk2")] {
            map.entry(k.to_owned()).or_insert_with(|| v.to_owned());
        }
        let colors = Colors { scheme, map };
        let (mut master_styles, mut master_align) = (HashMap::new(), HashMap::new());
        if let Some(ts) = master.as_ref().and_then(|m| m.child("txStyles")) {
            for (k, n) in [("title", "titleStyle"), ("body", "bodyStyle"), ("other", "otherStyle")] {
                if let Some(sz) = ts.child(n).and_then(first_size) {
                    master_styles.insert(k, sz);
                }
                if let Some(a) = ts.child(n).and_then(|s| s.child("lvl1pPr")).and_then(|p| p.attr("algn")) {
                    master_align.insert(k, align(a));
                }
            }
        }
        let phs = [layout.as_ref(), master.as_ref()]
            .into_iter()
            .flatten()
            .map(|n| n.child("cSld").and_then(|c| c.child("spTree")).map(placeholders).unwrap_or_default())
            .collect();
        let cx = Ctx { colors: &colors, phs, master_styles, master_align, scale, dx, dy };
        let bg = background(&slide, &colors)
            .or_else(|| layout.as_ref().and_then(|l| background(l, &colors)))
            .or_else(|| master.as_ref().and_then(|m| background(m, &colors)))
            .unwrap_or([255, 255, 255]);
        let mut elements = Vec::new();
        if let Some(tree) = slide.at(&["cSld", "spTree"]) {
            read_tree(tree, &cx, Transform { ox: 0.0, oy: 0.0, sx: 1.0, sy: 1.0 }, &mut pkg, &rels, &mut deck, &mut elements);
        }
        deck.slides.push(Slide { background: bg, elements, notes: String::new() });
    }
    if deck.slides.is_empty() {
        return Err("no slides in it".to_owned());
    }
    deck.bump_ids();
    Ok(deck)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deck::Layout;

    #[test]
    fn round_trip() {
        let mut deck = Deck::default();
        let s = deck.new_slide(Layout::TitleBody, "Agenda", "First point\nSecond & <third>");
        deck.slides.push(s);
        let mut r = Element::new(deck.id(), Kind::Rect, 100.0, 100.0, 200.0, 100.0);
        r.text = "Box".into();
        r.fill = Some([0x12, 0x34, 0x56]);
        deck.slides[1].elements.push(r);
        let mut l = Element::new(deck.id(), Kind::Line, 10.0, 600.0, 300.0, 50.0);
        l.flip = true;
        deck.slides[1].elements.push(l);
        let dir = std::env::temp_dir().join("slides-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.pptx");
        save(&deck, &path).unwrap();
        let back = load(std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(back.slides.len(), 2);
        let s1 = &back.slides[1];
        assert_eq!(s1.background, [0, 0, 0]);
        let texts: Vec<&str> = s1.elements.iter().map(|e| e.text.as_str()).collect();
        assert_eq!(texts, vec!["Agenda", "First point\nSecond & <third>", "Box", ""]);
        let body = &s1.elements[1];
        assert!(body.style.bullets);
        assert_eq!((body.x, body.y, body.w, body.h), (80.0, 180.0, 1120.0, 480.0));
        assert!((body.style.size - 22.0).abs() < 0.01);
        assert_eq!(s1.elements[2].kind, Kind::Rect);
        assert_eq!(s1.elements[2].fill, Some([0x12, 0x34, 0x56]));
        assert!(s1.elements[3].flip && s1.elements[3].kind == Kind::Line);
        assert_eq!(s1.elements[0].style.color, [255, 255, 255]);
    }
}

#[cfg(test)]
mod dump {
    /// SLIDES_DUMP=file.pptx cargo test dump -- --nocapture: what a deck reads as.
    #[test]
    fn dump() {
        let Ok(path) = std::env::var("SLIDES_DUMP") else { return };
        let deck = super::load(std::fs::read(path).unwrap()).unwrap();
        for (i, s) in deck.slides.iter().enumerate() {
            println!("slide {i} bg {:?}", s.background);
            for e in &s.elements {
                println!(
                    "  {:?} ({:.0},{:.0} {:.0}x{:.0}) size {:.1} color {:?} align {:?}/{:?} bullets {} bold {}: {:?}",
                    e.kind, e.x, e.y, e.w, e.h, e.style.size, e.style.color, e.style.align, e.style.valign, e.style.bullets, e.style.bold, e.text
                );
            }
        }
    }
}
