/*
 * This library helps convert an XML String into a serde_json::Value which can be
 * used to generate JSON
 */

#[cfg(test)]
#[macro_use]
extern crate serde_json;

use log::*;
use quick_xml::events::Event;
use quick_xml::Reader;
use serde_json::{to_value, Map, Value};
use std::io::BufRead;
use std::mem::take;

#[derive(Debug)]
pub struct Error {}

trait AttrMap {
    fn insert_text(&mut self, value: &Value) -> Option<Value>;
    fn insert_text_node(&mut self, value: Value);
}

impl AttrMap for Map<String, Value> {
    fn insert_text(&mut self, value: &Value) -> Option<Value> {
        if !self.is_empty() {
            if value.is_string() {
                self.insert_text_node(value.clone());
            }
            if let Ok(attrs) = to_value(take(self)) {
                return Some(attrs);
            }
        }
        None
    }

    fn insert_text_node(&mut self, value: Value) {
        self.insert("#text".to_string(), value);
    }
}

struct NodeValues {
    node: Map<String, Value>,
    nodes: Vec<Map<String, Value>>,
    nodes_are_map: Vec<bool>,
    values: Vec<Value>,
}

impl NodeValues {
    fn new() -> Self {
        Self {
            values: Vec::new(),
            node: Map::new(),
            nodes: Vec::new(),
            nodes_are_map: Vec::new(),
        }
    }

    fn insert(&mut self, key: String, value: Value) {
        self.node.insert(key, value);
    }

    fn insert_cdata(&mut self, value: &str) {
        let key = "#cdata".to_string();
        let new_value = match self.node.get(&key) {
            Some(existing) => {
                let mut old_value = existing.as_str().unwrap().to_string();
                old_value.push_str(value);
                old_value
            }
            None => value.to_string(),
        };
        self.node.insert(key, Value::String(new_value));
    }

    fn insert_text(&mut self, text: &str) {
        if !self.node.is_empty() {
            self.nodes.push(take(&mut self.node));
            self.nodes_are_map.push(true);
        }

        self.values.push(Value::String(text.to_string()));
        self.nodes_are_map.push(false);
    }

    fn remove_entry(&mut self, key: &String) -> Option<Value> {
        if self.node.contains_key(key) {
            debug!("Node contains `{}` already, need to convert to array", key);
            if let Some((_, existing)) = self.node.remove_entry(key) {
                return Some(existing);
            }
        }
        None
    }

    fn get_value(&mut self) -> Value {
        debug!("values to return: {:?}", self.values);
        if !self.node.is_empty() {
            self.nodes.push(take(&mut self.node));
            self.nodes_are_map.push(true);
        }

        if !self.nodes.is_empty() {
            // If we had collected some text along the way, that needs to be inserted
            // so we don't lose it

            if self.nodes.len() == 1 && self.values.len() <= 1 {
                if self.values.len() == 1 {
                    self.nodes[0].insert_text_node(self.values.remove(0));
                }
                debug!("returning node instead: {:?}", self.nodes[0]);
                return to_value(&self.nodes[0]).expect("Failed to #to_value() a node!");
            }
            for (index, node_is_map) in self.nodes_are_map.iter().enumerate() {
                if *node_is_map {
                    self.values
                        .insert(index, Value::Object(self.nodes.remove(0)));
                }
            }
        }

        match self.values.len() {
            0 => Value::Null,
            1 => self.values.pop().unwrap(),
            _ => Value::Array(take(&mut self.values)),
        }
    }
}

pub fn read<R: BufRead>(reader: &mut Reader<R>, depth: u64) -> Value {
    let mut buf = Vec::new();
    let mut nodes = NodeValues::new();
    debug!("Parsing at depth: {}", depth);

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                if let Ok(name) = String::from_utf8(e.name().into_inner().to_vec()) {
                    let mut child = read(reader, depth + 1);
                    let mut attrs = Map::new();
                    debug!("{} children: {:?}", name, child);

                    let _ = e
                        .attributes()
                        .map(|a| {
                            if let Ok(attr) = a {
                                let key = String::from_utf8(attr.key.into_inner().to_vec());
                                let value = String::from_utf8(attr.value.to_vec());

                                // Only bother adding the attribute if both key and value are valid utf8
                                if let (Ok(key), Ok(value)) = (key, value) {
                                    let key = format!("@{}", key);
                                    let value = Value::String(value);

                                    // If the child is already an object, that's where the insert
                                    // should happen
                                    if child.is_object() {
                                        child.as_object_mut().unwrap().insert(key, value);
                                    } else {
                                        attrs.insert(key, value);
                                    }
                                }
                            }
                        })
                        .collect::<Vec<_>>();

                    if let Some(mut existing) = nodes.remove_entry(&name) {
                        let mut entries: Vec<Value> = vec![];

                        if existing.is_array() {
                            let existing = existing.as_array_mut().unwrap();
                            while !existing.is_empty() {
                                entries.push(existing.remove(0));
                            }
                        } else {
                            entries.push(existing);
                        }

                        /*
                         * nodes with attributes need to be handled special
                         */
                        if let Some(attrs) = attrs.insert_text(&child) {
                            entries.push(attrs);
                        } else {
                            entries.push(child);
                        }

                        nodes.insert(name, Value::Array(entries));
                    /*
                     * nodes with attributes need to be handled special
                     */
                    } else if let Some(attrs) = attrs.insert_text(&child) {
                        nodes.insert(name, attrs);
                    } else {
                        nodes.insert(name, child);
                    }
                }
            }
            Ok(Event::Text(ref e)) => {
                if let Ok(decoded) = e.unescape() {
                    nodes.insert_text(&decoded);
                }
            }
            Ok(Event::CData(ref e)) => {
                if let Ok(decoded) = e.clone().escape() {
                    if let Ok(decoded_bt) = decoded.unescape() {
                        nodes.insert_cdata(&decoded_bt);
                    }
                }
            }
            Ok(Event::End(ref _e)) => break,
            Ok(Event::Eof) => break,
            _ => (),
        }
    }
    nodes.get_value()
}

/**
 * to_json() will take an input string and attempt to convert it into a form
 * of JSON
 */
pub fn to_json(xml: &str) -> Result<Value, Error> {
    let mut reader = Reader::from_str(xml);
    let config = reader.config_mut();
    config.expand_empty_elements = true;
    config.trim_text(true);

    Ok(read(&mut reader, 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json_eq(left: Value, right: Result<Value, Error>) {
        assert!(right.is_ok());
        assert_eq!(left, right.unwrap());
    }

    #[test]
    fn single_node() {
        json_eq(json!({ "e": null }), to_json("<e></e>"));
    }

    #[test]
    fn node_with_text() {
        json_eq(json!({"e" : "foo"}), to_json("<e>foo</e>"));
    }

    #[test]
    fn node_with_attr() {
        json_eq(
            json!({"e" : {"@name":"value"}}),
            to_json("<e name=\"value\"></e>"),
        );
    }

    #[test]
    fn node_with_attr_and_text() {
        json_eq(
            json!({"e": {"@name":"value", "#text" : "text"}}),
            to_json(r#"<e name="value">text</e>"#),
        );
    }

    #[test]
    fn node_with_children() {
        json_eq(
            json!(
            {
            "e":{
                "a":"text1",
                "b":"text2"
            }
            }),
            to_json(r#"<e> <a>text1</a> <b>text2</b> </e>"#),
        );
    }

    #[test]
    fn node_with_multiple_identical_children() {
        json_eq(
            json!({
            "e":{"a":[
                "text",
                "text"
                ]}
            }),
            to_json(r#"<e><a>text</a><a>text</a></e>"#),
        );
    }

    #[test]
    fn node_with_n_identical_children() {
        json_eq(
            json!({
            "e":{"a":[
                "text1",
                "text2",
                "text3"
                ]}
            }),
            to_json(r#"<e><a>text1</a><a>text2</a><a>text3</a></e>"#),
        );
    }

    #[test]
    fn node_with_text_and_child() {
        json_eq(
            json!(
            {
            "e":{
                "#text":"lol",
                "a":"text"
            }
            }),
            to_json(r#"<e> lol <a>text</a></e>"#),
        );
    }

    #[test]
    fn node_with_just_text() {
        json_eq(
            json!(
            {
            "a":"hello"
            }),
            to_json(r#"<a>hello</a>"#),
        );
    }

    #[test]
    fn node_with_attrs_and_text() {
        json_eq(
            json!(
            {
                "a":{
                    "@x":"y",
                    "#text":"hello"
                }
            }),
            to_json(r#"<a x="y">hello</a>"#),
        );
    }

    #[test]
    fn nested_nodes_with_attrs() {
        json_eq(
            json!(
            {
                "a":{
                    "@id":"a",
                    "b":{
                        "@id":"b",
                        "#text":"hey!"
                    }
                }
            }),
            to_json(r#"<a id="a"><b id="b">hey!</b></a>"#),
        );
    }

    #[test]
    fn node_with_nested_text() {
        json_eq(
            json!(
            {
                "a":["x",{"c":null},"y"]
            }),
            to_json(r#"<a>x<c/>y</a>"#),
        );
    }

    #[test]
    fn node_with_empty_attrs() {
        json_eq(
            json!(
            {
            "x":{"@u":""}
            }),
            to_json(r#"<x u=""/>"#),
        );
    }

    #[test]
    fn some_basic_html() {
        json_eq(
            json!(
            {
            "html":{
                "head":{
                "title":"Xml/Json",
                "meta":{
                    "@name":"x",
                    "@content":"y"
                }
                },
                "body":null
            }
            }),
            to_json(
                r#"<html><head><title>Xml/Json</title><meta name="x" content="y"/></head><body/></html>"#,
            ),
        );
    }

    #[test]
    fn more_complex_html() {
        json_eq(
            json!(
            {
                "ol":{
                    "@class":"xoxo",
                    "li":[
                    {
                        "#text":"Subject 1",
                        "ol":{"li":[
                            "subpoint a",
                            "subpoint b"
                        ]}
                    },
                    {
                        "span":"Subject 2",
                        "ol":{
                        "@compact":"compact",
                        "li":[
                            "subpoint c",
                            "subpoint d"
                        ]
                        }
                    }
                    ]
                }
            }),
            to_json(
                r#"<ol class="xoxo"><li>Subject 1     <ol><li>subpoint a</li><li>subpoint b</li></ol></li><li><span>Subject 2</span><ol compact="compact"><li>subpoint c</li><li>subpoint d</li></ol></li></ol>"#,
            ),
        );
    }

    #[test]
    fn node_with_cdata() {
        json_eq(
            json!(
            {
            "e":{"#cdata":" .. some data .. "}
            }),
            to_json(r#"<e><![CDATA[ .. some data .. ]]></e>"#),
        );
    }

    #[test]
    fn node_with_cdata_and_siblings() {
        json_eq(
            json!(
            {
            "e":{
                "a":null,
                "#cdata":" .. some data .. ",
                "b":null
            }
            }),
            to_json(r#"<e><a/><![CDATA[ .. some data .. ]]><b/></e>"#),
        );
    }

    #[test]
    fn node_with_cdata_inside_text() {
        json_eq(
            json!(
            {
            "e":["some text",{"#cdata":" .. some data .. "}, "more text"]
            }),
            to_json(r#"<e>  some text  <![CDATA[ .. some data .. ]]>  more text</e>"#),
        );
    }

    #[test]
    fn node_with_child_cdata_and_text() {
        json_eq(
            json!(
            {
            "e":{
                "#text":"some text",
                "#cdata":" .. some data .. ",
                "a":null
            }
            }),
            to_json(r#"<e>  some text  <![CDATA[ .. some data .. ]]><a/></e>"#),
        );
    }

    #[test]
    fn node_with_duplicate_cdata() {
        json_eq(
            json!(
            {
            "e":{
                "#cdata":" .. some data ..  .. more data .. ",
            }
            }),
            to_json(r#"<e><![CDATA[ .. some data .. ]]><![CDATA[ .. more data .. ]]></e>"#),
        );
    }

    #[test]
    fn node_empty() {
        json_eq(json!(null), to_json(""));
    }

    #[test]
    fn node_with_duplicate_text() {
        json_eq(
            json!({"e": {"a": ["x", "y"]}}),
            to_json("<e><a>x</a><a>y</a></e>"),
        );
    }

    #[test]
    fn node_with_duplicate_attrs_and_text() {
        json_eq(
            json!({"e": {"a": [{"#text": "x", "@u": "x"}, {"#text": "y", "@u": "y"}]}}),
            to_json(r#"<e><a u="x">x</a><a u="y">y</a></e>"#),
        );
    }

    #[test]
    fn node_with_text_and_siblings() {
        json_eq(
            json!({"e":["x", {"a": {"@u": "y"}}, "z"]}),
            to_json(r#"<e>x <a u="y"/> z</e>"#),
        );
    }

    #[test]
    fn node_with_text_and_siblings_mixed() {
        json_eq(
            json!({"e":["a", {"x": "b"}, "c", {"x": "d"}]}),
            to_json(r#"<e>a <x>b</x> c <x>d</x></e>"#),
        );
    }

    #[test]
    fn node_with_cdata_only() {
        json_eq(
            json!(
            {
            "#cdata":" .. some data .. "
            }),
            to_json(r#"<![CDATA[ .. some data .. ]]>"#),
        );
    }
}
