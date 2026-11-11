//! Conversion minimale texte brut <-> ADF (Atlassian Document Format).
//!
//! - Texte -> ADF : un `paragraph` par bloc séparé par une (ou plusieurs) ligne(s) vide(s), `hardBreak`
//!   pour les retours à la ligne simples. Texte vide (ou blanc) -> `None` (envoyer `null` à Jira).
//! - ADF -> texte : extraction tolérante. Paragraphes séparés par une ligne vide ; titres (`# `),
//!   listes (`- ` / `1. `), citations (`> `), blocs de code (clôtures ```), tableaux (`a | b`) ;
//!   mentions, emoji, statuts via `attrs.text` ; liens (`inlineCard`…) via `attrs.url` ; médias
//!   remplacés par un marqueur ; nœuds inconnus parcourus récursivement.
//! - [`is_lossless`] indique si réécrire le document depuis son texte ne perdrait rien.

use serde_json::{Value, json};

/// Convertit du texte brut en document ADF. `None` si le texte est vide.
pub fn text_to_adf(text: &str) -> Option<Value> {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut paragraphs: Vec<Vec<&str>> = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    for line in text.split('\n') {
        if line.trim().is_empty() {
            if !current.is_empty() {
                paragraphs.push(std::mem::take(&mut current));
            }
        } else {
            current.push(line);
        }
    }
    if !current.is_empty() {
        paragraphs.push(current);
    }
    if paragraphs.is_empty() {
        return None;
    }
    let content: Vec<Value> = paragraphs
        .into_iter()
        .map(|lines| {
            let mut inline = Vec::with_capacity(lines.len() * 2);
            for (i, line) in lines.into_iter().enumerate() {
                if i > 0 {
                    inline.push(json!({ "type": "hardBreak" }));
                }
                inline.push(json!({ "type": "text", "text": line }));
            }
            json!({ "type": "paragraph", "content": inline })
        })
        .collect();
    Some(json!({ "type": "doc", "version": 1, "content": content }))
}

/// Extrait le texte d'un document ADF (ou de tout nœud ADF). `null` -> chaîne vide.
pub fn adf_to_text(doc: &Value) -> String {
    render(doc).trim_end_matches('\n').to_owned()
}

/// `true` si le document ne contient que des paragraphes de texte sans mise en forme (et des
/// `hardBreak`) et que `text_to_adf(adf_to_text(doc))` le reproduit : l'éditer en texte brut est sans perte.
/// `null` est sans perte.
pub fn is_lossless(doc: &Value) -> bool {
    if doc.is_null() {
        return true;
    }
    if !is_simple(doc) {
        return false;
    }
    let text = adf_to_text(doc);
    let back = text_to_adf(&text);
    let back_text = back.as_ref().map(adf_to_text).unwrap_or_default();
    // Mêmes blocs (pas de paragraphe vide perdu, pas d'espaces de bord supprimés) et même texte.
    back_text == text && block_count(doc) == back.as_ref().map(block_count).unwrap_or(0)
}

fn block_count(doc: &Value) -> usize {
    children(doc).len()
}

fn is_simple(doc: &Value) -> bool {
    if node_type(doc) != "doc" {
        return false;
    }
    children(doc).iter().all(|p| {
        node_type(p) == "paragraph"
            && !has_marks_or_attrs(p, &["localId"])
            && children(p).iter().all(|n| match node_type(n) {
                "text" => {
                    !has_marks_or_attrs(n, &[])
                        && n.get("text")
                            .and_then(Value::as_str)
                            .is_some_and(|t| !t.contains('\n') && !t.contains('\r'))
                }
                "hardBreak" => true,
                _ => false,
            })
    })
}

/// Vrai si le nœud porte des `marks` ou des `attrs` autres que ceux tolérés.
fn has_marks_or_attrs(node: &Value, tolerated_attrs: &[&str]) -> bool {
    let marks = node
        .get("marks")
        .and_then(Value::as_array)
        .is_some_and(|m| !m.is_empty());
    let attrs = node
        .get("attrs")
        .and_then(Value::as_object)
        .is_some_and(|a| {
            a.iter()
                .any(|(k, v)| !v.is_null() && !tolerated_attrs.contains(&k.as_str()))
        });
    marks || attrs
}

fn node_type(node: &Value) -> &str {
    node.get("type").and_then(Value::as_str).unwrap_or("")
}

fn children(node: &Value) -> &[Value] {
    node.get("content")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn attr<'a>(node: &'a Value, name: &str) -> Option<&'a str> {
    node.get("attrs")?.get(name)?.as_str()
}

fn is_inline(node: &Value) -> bool {
    match node_type(node) {
        "text" | "hardBreak" | "mention" | "emoji" | "inlineCard" | "status" | "date"
        | "placeholder" | "inlineExtension" | "mediaInline" => true,
        // Nœud inconnu : inline s'il porte du texte et pas de contenu.
        _ => node.get("text").is_some() && node.get("content").is_none(),
    }
}

fn render_inline_seq(nodes: &[Value]) -> String {
    nodes.iter().map(render).collect()
}

fn render_blocks(nodes: &[Value], sep: &str) -> String {
    nodes
        .iter()
        .map(render)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(sep)
}

fn prefix_lines(text: &str, first: &str, rest: &str) -> String {
    text.split('\n')
        .enumerate()
        .map(|(i, l)| {
            let p = if i == 0 { first } else { rest };
            if l.is_empty() && i > 0 {
                String::new()
            } else {
                format!("{p}{l}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn render(node: &Value) -> String {
    if node.is_null() {
        return String::new();
    }
    if let Some(s) = node.as_str() {
        return s.to_owned();
    }
    if let Some(arr) = node.as_array() {
        return render_blocks(arr, "\n\n");
    }
    let kids = children(node);
    match node_type(node) {
        "doc" => render_blocks(kids, "\n\n"),
        "paragraph" => render_inline_seq(kids),
        "text" => node
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
        "hardBreak" => "\n".to_owned(),
        "heading" => {
            let level = node
                .get("attrs")
                .and_then(|a| a.get("level"))
                .and_then(Value::as_u64)
                .unwrap_or(1)
                .clamp(1, 6) as usize;
            format!("{} {}", "#".repeat(level), render_inline_seq(kids))
        }
        "bulletList" | "taskList" => kids
            .iter()
            .map(|item| {
                let marker = match (node_type(item), attr(item, "state")) {
                    ("taskItem", Some("DONE")) => "- [x] ",
                    ("taskItem", _) => "- [ ] ",
                    _ => "- ",
                };
                prefix_lines(&render_list_item(item), marker, "  ")
            })
            .collect::<Vec<_>>()
            .join("\n"),
        "orderedList" => {
            let start = node
                .get("attrs")
                .and_then(|a| a.get("order"))
                .and_then(Value::as_u64)
                .unwrap_or(1);
            kids.iter()
                .enumerate()
                .map(|(i, item)| {
                    let marker = format!("{}. ", start + i as u64);
                    let indent = " ".repeat(marker.len());
                    prefix_lines(&render_list_item(item), &marker, &indent)
                })
                .collect::<Vec<_>>()
                .join("\n")
        }
        "listItem" | "taskItem" => render_list_item(node),
        "codeBlock" => format!("```\n{}\n```", render_inline_seq(kids)),
        "blockquote" => prefix_lines(&render_blocks(kids, "\n\n"), "> ", "> "),
        "rule" => "---".to_owned(),
        "panel" | "expand" | "nestedExpand" | "layoutSection" | "layoutColumn" | "tableCell"
        | "tableHeader" | "bodiedExtension" => {
            let title = attr(node, "title")
                .map(|t| format!("{t}\n"))
                .unwrap_or_default();
            format!("{title}{}", render_blocks(kids, "\n\n"))
        }
        "table" => kids.iter().map(render).collect::<Vec<_>>().join("\n"),
        "tableRow" => kids
            .iter()
            .map(|c| render(c).replace('\n', " "))
            .collect::<Vec<_>>()
            .join(" | "),
        "mention" => attr(node, "text")
            .map(str::to_owned)
            .or_else(|| attr(node, "id").map(|id| format!("@{id}")))
            .unwrap_or_else(|| "@?".to_owned()),
        "emoji" => attr(node, "text")
            .or_else(|| attr(node, "shortName"))
            .unwrap_or("")
            .to_owned(),
        "status" => attr(node, "text")
            .map(|t| format!("[{t}]"))
            .unwrap_or_default(),
        "date" => attr(node, "timestamp").unwrap_or("").to_owned(),
        "inlineCard" | "blockCard" | "embedCard" => attr(node, "url")
            .map(str::to_owned)
            .or_else(|| {
                node.get("attrs")
                    .and_then(|a| a.get("data"))
                    .and_then(|d| d.get("url"))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .unwrap_or_default(),
        "mediaSingle" | "mediaGroup" | "media" | "mediaInline" => "[média]".to_owned(),
        _ => {
            // Nœud inconnu : texte direct éventuel, sinon contenu parcouru.
            if let Some(t) = node.get("text").and_then(Value::as_str) {
                t.to_owned()
            } else if kids.iter().all(is_inline) {
                render_inline_seq(kids)
            } else {
                render_blocks(kids, "\n\n")
            }
        }
    }
}

fn render_list_item(item: &Value) -> String {
    let kids = children(item);
    if kids.iter().all(is_inline) {
        // taskItem : contenu inline direct.
        render_inline_seq(kids)
    } else {
        render_blocks(kids, "\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text_is_none() {
        assert_eq!(text_to_adf(""), None);
        assert_eq!(text_to_adf("  \n \n"), None);
    }

    #[test]
    fn paragraphs_and_hard_breaks() {
        let doc = text_to_adf("Ligne 1\nLigne 2\n\n\nParagraphe 2\r\n").unwrap();
        assert_eq!(
            doc,
            json!({"type":"doc","version":1,"content":[
                {"type":"paragraph","content":[
                    {"type":"text","text":"Ligne 1"},{"type":"hardBreak"},{"type":"text","text":"Ligne 2"}]},
                {"type":"paragraph","content":[{"type":"text","text":"Paragraphe 2"}]}
            ]})
        );
        assert_eq!(adf_to_text(&doc), "Ligne 1\nLigne 2\n\nParagraphe 2");
        assert!(is_lossless(&doc));
    }

    #[test]
    fn fixture_paragraph_roundtrip() {
        let doc: Value =
            serde_json::from_str(include_str!("../tests/fixtures/issues/adf_paragraph.json"))
                .unwrap();
        let text = adf_to_text(&doc);
        assert_eq!(
            text,
            "En tant qu'utilisateur, je veux me connecter avec un jeton API.\n\n\
             Critere : le jeton est stocke hors du depot."
        );
        assert!(is_lossless(&doc));
        assert_eq!(text_to_adf(&text).unwrap(), doc);
    }

    #[test]
    fn null_is_lossless_and_empty() {
        assert!(is_lossless(&Value::Null));
        assert_eq!(adf_to_text(&Value::Null), "");
    }

    #[test]
    fn rich_content_is_extracted_but_lossy() {
        let doc = json!({"type":"doc","version":1,"content":[
            {"type":"heading","attrs":{"level":2},"content":[{"type":"text","text":"Titre"}]},
            {"type":"paragraph","content":[
                {"type":"text","text":"Voir "},
                {"type":"mention","attrs":{"id":"712020:a","text":"@Alice"}},
                {"type":"text","text":" "},
                {"type":"emoji","attrs":{"shortName":":smile:","text":"😄"}},
                {"type":"text","text":" et "},
                {"type":"inlineCard","attrs":{"url":"https://example.test/x"}},
                {"type":"text","text":" gras","marks":[{"type":"strong"}]}
            ]},
            {"type":"bulletList","content":[
                {"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"un"}]},
                    {"type":"bulletList","content":[{"type":"listItem","content":[
                        {"type":"paragraph","content":[{"type":"text","text":"sous"}]}]}]}]},
                {"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"deux"}]}]}
            ]},
            {"type":"orderedList","attrs":{"order":3},"content":[
                {"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"trois"}]}]}
            ]},
            {"type":"codeBlock","attrs":{"language":"rust"},"content":[{"type":"text","text":"fn main() {}"}]},
            {"type":"futureNode","content":[{"type":"paragraph","content":[{"type":"text","text":"inconnu"}]}]},
            {"type":"mediaSingle","content":[{"type":"media","attrs":{"id":"x","type":"file"}}]}
        ]});
        let text = adf_to_text(&doc);
        assert_eq!(
            text,
            "## Titre\n\n\
             Voir @Alice 😄 et https://example.test/x gras\n\n\
             - un\n  - sous\n- deux\n\n\
             3. trois\n\n\
             ```\nfn main() {}\n```\n\n\
             inconnu\n\n\
             [média]"
        );
        assert!(!is_lossless(&doc));
    }

    #[test]
    fn marks_or_empty_paragraphs_are_lossy() {
        let bold = json!({"type":"doc","version":1,"content":[
            {"type":"paragraph","content":[{"type":"text","text":"x","marks":[{"type":"em"}]}]}]});
        assert!(!is_lossless(&bold));
        let empty_para = json!({"type":"doc","version":1,"content":[
            {"type":"paragraph","content":[{"type":"text","text":"a"}]},
            {"type":"paragraph"},
            {"type":"paragraph","content":[{"type":"text","text":"b"}]}]});
        assert!(!is_lossless(&empty_para));
        let trailing_space = json!({"type":"doc","version":1,"content":[
            {"type":"paragraph","content":[{"type":"text","text":"a "}]}]});
        assert!(is_lossless(&trailing_space), "les espaces sont conservés");
        let blank_para = json!({"type":"doc","version":1,"content":[
            {"type":"paragraph","content":[{"type":"text","text":"a"}]},
            {"type":"paragraph","content":[{"type":"text","text":"  "}]}]});
        assert!(!is_lossless(&blank_para));
        let leading_break = json!({"type":"doc","version":1,"content":[
            {"type":"paragraph","content":[{"type":"text","text":"a"}]},
            {"type":"paragraph","content":[{"type":"hardBreak"},{"type":"text","text":"b"}]}]});
        assert!(!is_lossless(&leading_break));
        let local_id = json!({"type":"doc","version":1,"content":[
            {"type":"paragraph","attrs":{"localId":"abc"},"content":[{"type":"text","text":"a"}]}]});
        assert!(is_lossless(&local_id));
    }
}
