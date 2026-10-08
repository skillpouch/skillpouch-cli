//! Canonical JSON, byte-identical to `canonicalJson` in `@skillpouch/contracts`:
//! object keys sorted by UTF-16 code units, no whitespace.

use serde_json::Value;

pub fn canonical_json(value: &Value) -> String {
    let mut out = String::new();
    write(value, &mut out);
    out
}

fn write(value: &Value, out: &mut String) {
    match value {
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write(item, out);
            }
            out.push(']');
        }
        Value::Object(map) => {
            let mut entries: Vec<_> = map.iter().collect();
            entries.sort_by(|(a, _), (b, _)| a.encode_utf16().cmp(b.encode_utf16()));
            out.push('{');
            for (index, (key, item)) in entries.into_iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String(key.clone()).to_string());
                out.push(':');
                write(item, out);
            }
            out.push('}');
        }
        scalar => out.push_str(&scalar.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sorts_keys_recursively_without_whitespace() {
        let value = json!({ "y": [1, { "b": true, "a": null }], "x": "é\"" });
        assert_eq!(
            canonical_json(&value),
            r#"{"x":"é\"","y":[1,{"a":null,"b":true}]}"#
        );
    }

    #[test]
    fn orders_keys_by_utf16_like_javascript() {
        // U+1F600 sorts after U+FF61 in UTF-8 but before it in UTF-16.
        let value = json!({ "\u{FF61}": 2, "\u{1F600}": 1 });
        assert_eq!(canonical_json(&value), "{\"\u{1F600}\":1,\"\u{FF61}\":2}");
    }
}
