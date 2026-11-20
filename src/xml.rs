//! A small XML tree, enough to read PowerPoint's parts: elements by local
//! name (prefixes dropped), their attributes, children and text.

use quick_xml::events::Event;
use quick_xml::Reader;

#[derive(Debug, Default, Clone)]
pub struct Node {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
    pub text: String,
}

fn local(name: &str) -> String {
    name.rsplit(':').next().unwrap_or_default().to_owned()
}

#[allow(deprecated)] // normalized_value wants the XML version; these are all 1.0
fn attrs(e: &quick_xml::events::BytesStart) -> Vec<(String, String)> {
    e.attributes()
        .flatten()
        .map(|a| {
            let key = a.key.as_ref().to_owned();
            let value = a.unescape_value().map(|v| v.into_owned()).unwrap_or_default();
            (key, value)
        })
        .collect()
}

pub fn parse(xml: &str) -> Result<Node, String> {
    let mut reader = Reader::from_str(xml);
    let mut stack = vec![Node { name: "#root".into(), ..Default::default() }];
    loop {
        match reader.read_event().map_err(|e| e.to_string())? {
            Event::Start(e) => stack.push(Node { name: local(e.name().as_ref()), attrs: attrs(&e), ..Default::default() }),
            Event::Empty(e) => {
                let n = Node { name: local(e.name().as_ref()), attrs: attrs(&e), ..Default::default() };
                stack.last_mut().unwrap().children.push(n);
            }
            Event::End(_) => {
                let n = stack.pop().unwrap();
                match stack.last_mut() {
                    Some(parent) => parent.children.push(n),
                    None => return Ok(n),
                }
            }
            // Entities (&amp; and the like) come as references of their own.
            Event::Text(t) => stack.last_mut().unwrap().text.push_str(&t.xml10_content()),
            Event::CData(t) => stack.last_mut().unwrap().text.push_str(&t.into_inner()),
            Event::GeneralRef(r) => {
                let name = r.into_inner().into_owned();
                let ch = match name.as_str() {
                    "amp" => Some('&'),
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "quot" => Some('"'),
                    "apos" => Some('\''),
                    n if n.starts_with("#x") => u32::from_str_radix(&n[2..], 16).ok().and_then(char::from_u32),
                    n if n.starts_with('#') => n[1..].parse().ok().and_then(char::from_u32),
                    _ => None,
                };
                if let Some(c) = ch {
                    stack.last_mut().unwrap().text.push(c);
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    let mut root = stack.into_iter().next().unwrap();
    Ok(if root.children.len() == 1 { root.children.remove(0) } else { root })
}

impl Node {
    pub fn attr(&self, key: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == key || k.rsplit(':').next() == Some(key))
            .map(|(_, v)| v.as_str())
    }

    pub fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.name == name)
    }

    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }

    /// The first element down a path of names: `node.at(&["spPr", "xfrm", "off"])`.
    pub fn at(&self, path: &[&str]) -> Option<&Node> {
        let mut n = self;
        for p in path {
            n = n.child(p)?;
        }
        Some(n)
    }

    /// Every element of a name anywhere below.
    pub fn find_all<'a>(&'a self, name: &str, out: &mut Vec<&'a Node>) {
        for c in &self.children {
            if c.name == name {
                out.push(c);
            }
            c.find_all(name, out);
        }
    }
}

pub fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}
