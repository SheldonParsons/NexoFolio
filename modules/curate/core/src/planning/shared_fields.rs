//! Reversible batch-local dictionaries. Facts and freshness hashes remain unchanged.
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Default)]
struct Pool {
    values: Vec<Value>,
    ids: BTreeMap<String, usize>,
}
impl Pool {
    fn intern(&mut self, value: Value) -> usize {
        let key = value.to_string();
        if let Some(id) = self.ids.get(&key) {
            return *id;
        }
        let id = self.values.len();
        self.ids.insert(key, id);
        self.values.push(value);
        id
    }
}
#[derive(Default)]
struct Encoder {
    sources: Pool,
    texts: Pool,
    siblings: Pool,
}
impl Encoder {
    fn source(&mut self, value: &mut Value) {
        let Some(mut base) = value.as_object().cloned() else {
            return;
        };
        let pointer = base.remove("pointer").unwrap_or(Value::Null);
        *value = json!({"source_ref":self.sources.intern(Value::Object(base)),"pointer":pointer});
    }
}

pub(crate) fn shared_fields(mut fields: Vec<Value>) -> Value {
    let mut encoder = Encoder::default();
    for field in &mut fields {
        let siblings = field["siblings"].take();
        field["siblings"] = json!({"siblings_ref":encoder.siblings.intern(siblings)});
        if let Some(clues) = field["clues"].as_array_mut() {
            for clue in clues {
                let text = clue[1].take();
                clue[1] = json!({"text_ref":encoder.texts.intern(text)});
                encoder.source(&mut clue[3]);
            }
        }
        if let Some(values) = field["representative_values"].as_array_mut() {
            for value in values {
                if let Some(source) = value.get_mut("source") {
                    encoder.source(source);
                }
            }
        }
    }
    json!({"fields":fields,"shared":{"sources":encoder.sources.values,"texts":encoder.texts.values,"siblings":encoder.siblings.values}})
}

/// The description pass shares the same reversible representation without changing its hash input.
pub(crate) fn shared_endpoint_input(mut input: Value) -> Value {
    let mut encoder = Encoder::default();
    if let Some(fields) = input["fields"].as_array_mut() {
        for field in fields {
            let siblings = field["siblings"].take();
            field["siblings"] = json!({"siblings_ref":encoder.siblings.intern(siblings)});
            if let Some(evidence) = field["evidence"].as_array_mut() {
                for clue in evidence {
                    let text = clue["text"].take();
                    clue["text"] = json!({"text_ref":encoder.texts.intern(text)});
                    if let Some(source) = clue.get_mut("source") {
                        encoder.source(source);
                    }
                }
            }
            if let Some(values) = field
                .get_mut("value_examples")
                .and_then(Value::as_array_mut)
            {
                for value in values {
                    if let Some(source) = value.get_mut("source") {
                        encoder.source(source);
                    }
                }
            }
        }
    }
    input["shared"] = json!({"sources":encoder.sources.values,"texts":encoder.texts.values,"siblings":encoder.siblings.values});
    input
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_sources_keep_scope_pointers_types_and_distinct_texts() {
        let sources = [
            json!({"endpoint":"one","environment":"qa","pointer":"/a","complete":false}),
            json!({"endpoint":"one","environment":"qa","pointer":"/b","complete":false}),
            json!({"endpoint":"two","environment":"prod","pointer":"/a","complete":true}),
        ];
        let fields:Vec<_> = sources.iter().enumerate().map(|(i,s)| json!({"key":i.to_string(),"siblings":["a","b"],"clues":[["observation","observed",[],s]],"representative_values":[{"value_json":"{\"id\":900719925474099312345}","source":s}]})).collect();
        let packed = shared_fields(fields.clone());
        assert_eq!(packed["shared"]["sources"].as_array().unwrap().len(), 2);
        for (i, original) in fields.iter().enumerate() {
            let mut decoded = packed["fields"][i].clone();
            let id = decoded["siblings"]["siblings_ref"].as_u64().unwrap() as usize;
            decoded["siblings"] = packed["shared"]["siblings"][id].clone();
            let id = decoded["clues"][0][1]["text_ref"].as_u64().unwrap() as usize;
            decoded["clues"][0][1] = packed["shared"]["texts"][id].clone();
            for pointer in ["/clues/0/3", "/representative_values/0/source"] {
                let target = decoded.pointer_mut(pointer).unwrap();
                let id = target["source_ref"].as_u64().unwrap() as usize;
                let mut source = packed["shared"]["sources"][id].clone();
                source["pointer"] = target["pointer"].clone();
                *target = source;
            }
            assert_eq!(&decoded, original);
        }
    }
}

#[cfg(test)]
mod endpoint_tests {
    use super::*;
    #[test]
    fn description_encoding_retains_all_field_evidence_and_non_field_context() {
        let original = json!({"method":"POST","path":"/fixture","authentication":[{"scheme":"fixture"}],"request_material":[{"body_state":"partial"}],"fields":[{"key":"request_body:id","siblings":["id","name"],"evidence":[{"text":"fixture label","source":{"endpoint":"a","environment":"qa","pointer":"/id"},"options":[{"value":"1","label":"one"}]}],"value_examples":[{"value_json":"null","source":{"endpoint":"a","environment":"qa","pointer":"/id"}}]}]});
        let encoded = shared_endpoint_input(original.clone());
        let mut decoded = encoded.clone();
        let shared = decoded.as_object_mut().unwrap().remove("shared").unwrap();
        let field = &mut decoded["fields"][0];
        let index = field["siblings"]["siblings_ref"].as_u64().unwrap() as usize;
        field["siblings"] = shared["siblings"][index].clone();
        let index = field["evidence"][0]["text"]["text_ref"].as_u64().unwrap() as usize;
        field["evidence"][0]["text"] = shared["texts"][index].clone();
        for path in ["/evidence/0/source", "/value_examples/0/source"] {
            let source = field.pointer_mut(path).unwrap();
            let index = source["source_ref"].as_u64().unwrap() as usize;
            let mut restored = shared["sources"][index].clone();
            restored["pointer"] = source["pointer"].clone();
            *source = restored;
        }
        assert_eq!(decoded, original);
    }
}
