/*
 * This library helps convert an XML String into a serde_json::Value which can be
 * used to generate JSON
 */

#[cfg(test)]
#[macro_use]
extern crate serde_json;

use log::*;
use quick_xml::Reader;
use quick_xml::events::Event;
use serde_json::{Map, Value};

#[derive(Debug)]
pub struct Error {
}


fn read<'a>(mut reader: &mut Reader<&'a [u8]>) -> Value {
    let mut buf = Vec::new();
    let mut values = Vec::new();
    let mut node = Map::new();

    loop {
        match reader.read_event(&mut buf) {
            Ok(Event::Start(ref e)) => {
                let mut attrs = Map::new();

                if let Ok(name) = String::from_utf8(e.name().to_vec()) {
                    let child = read(&mut reader);
                    let mut has_attrs = false;

                    e.attributes().map(|a| {
                        if let Ok(attr) = a {
                            let key = String::from_utf8(attr.key.to_vec());
                            let value = String::from_utf8(attr.value.to_vec());

                            // Only bother adding the attribute if both key and value are valid utf8
                            if key.is_ok() && value.is_ok() {
                                has_attrs = true;
                                attrs.insert(format!("@{}", key.unwrap()), Value::String(value.unwrap()));
                            }
                        }
                    }).collect::<Vec<_>>();

                    /* 
                     * nodes with attributes need to be handled special
                     */
                    if attrs.len() > 0 {
                        if child.is_string() {
                            attrs.insert("#text".to_string(), child);
                        }
                        if let Ok(attrs) = serde_json::to_value(attrs) {
                            node.insert(name, attrs);
                        }
                    }
                    else {
                        node.insert(name, child);
                    }
                }

                if let Ok(node_value) = serde_json::to_value(&node) {
                    values.push(node_value);
                }
            },
            Ok(Event::Text(e)) => {
                if let Ok(decoded) = e.unescape_and_decode(&reader) {
                    values.push(Value::String(decoded));
                }
            },
            Ok(Event::End(ref _e)) => break,
            Ok(Event::Eof) => break,
            _ => (),
        }
    }

    info!("node: {:?}", node);

    match values.len() {
        0 => Value::Null,
        1 => values.pop().unwrap(),
        _ => {
            Value::Array(values)
        }
    }
}

/**
 * to_json() will take an input string and attempt to convert it into a form
 * of JSON
 */
pub fn to_json(xml: &str) -> Result<Value, Error> {
    let mut reader = Reader::from_str(xml);
    reader.trim_text(true);

    Ok(read(&mut reader))
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
        json_eq(
            json!({"e" : null}),
            to_json("<e></e>")
        );
    }

    #[test]
    fn node_with_text() {
        json_eq(
            json!({"e" : "foo"}),
            to_json("<e>foo</e>")
        );
    }

    #[test]
    fn node_with_attr() {
        json_eq(
            json!({"e" : {"@name":"value"}}),
            to_json("<e name=\"value\"></e>")
        );
    }

    #[test]
    fn node_with_attr_and_text() {
        json_eq(
            json!({"e": {"@name":"value", "#text" : "text"}}),
            to_json(r#"<e name="value">text</e>"#)
        );
    }

    #[test]
    fn node_with_children() {
        let _ = pretty_env_logger::try_init();
        json_eq(
            json!(
                {
                "e":{
                    "a":"text1",
                    "b":"text2"
                }
                }),
            to_json(r#"<e> <a>text1</a> <b>text2</b> </e>"#)
        );
    }
}
