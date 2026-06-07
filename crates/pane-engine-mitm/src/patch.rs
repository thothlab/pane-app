//! Response patching: walk a small dot-path against a virtual response tree
//! `{ status, headers.<Name>, body.<nested.dot.path> }` and apply set / delete
//! / append operations against it.
//!
//! Path syntax (mirrors JS/jq intuition, no library required):
//!   - `.`            — segment separator
//!   - `[i]`          — explicit array index (after a segment)
//!   - `[-]`          — append (only valid as the final segment of `set`/`append`)
//!
//! Examples:
//!   - `status`
//!   - `headers.Content-Type`
//!   - `body.user.fio`
//!   - `body.user.visible_facilities[0]`
//!   - `body.user.visible_facilities[-]`
//!
//! Path heads `status` / `headers` / `body` are special. Everything else
//! under `body.` walks the parsed JSON body in place. Unknown heads are a
//! no-op (logged at warn).

use pane_storage::PatchOp;
use serde_json::Value;

pub struct ResponseTree<'a> {
    pub status: &'a mut u16,
    pub headers: &'a mut Vec<(String, String)>,
    pub body: &'a mut Value,
}

#[derive(Debug, Clone, Copy)]
enum Step<'a> {
    Key(&'a str),
    Index(usize),
    Append,
}

fn tokenize(path: &str) -> Vec<Step<'_>> {
    let mut out = Vec::new();
    for raw_seg in path.split('.') {
        // A segment can carry trailing bracket indices: "facilities[0][1]".
        let mut buf = String::new();
        let mut chars = raw_seg.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '[' {
                if !buf.is_empty() {
                    out.push(Step::Key(leak_str(std::mem::take(&mut buf))));
                }
                let mut idx = String::new();
                for c2 in chars.by_ref() {
                    if c2 == ']' {
                        break;
                    }
                    idx.push(c2);
                }
                if idx == "-" {
                    out.push(Step::Append);
                } else if let Ok(n) = idx.parse::<usize>() {
                    out.push(Step::Index(n));
                }
            } else {
                buf.push(c);
            }
        }
        if !buf.is_empty() {
            out.push(Step::Key(leak_str(buf)));
        }
    }
    out
}

// Tokenizer leaks tiny strings to satisfy Step::Key's borrow lifetime.
// Acceptable: a rule's path is typically <100 bytes and parsed once per
// request — the storage size is a wash compared to the proxied body.
fn leak_str(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

/// Apply one operation to the response tree. Returns true on success.
///
/// Paths starting with `status` / `headers.<...>` / `body.<...>` target the
/// matching part of the response. Anything else is treated as a body path
/// (the `body.` prefix is optional), so `user.fio` is equivalent to
/// `body.user.fio`. This matches how users intuitively write paths.
pub fn apply(tree: &mut ResponseTree<'_>, op: &PatchOp) -> bool {
    let path = match op {
        PatchOp::Set { path, .. } => path.as_str(),
        PatchOp::Delete { path } => path.as_str(),
        PatchOp::Append { path, .. } => path.as_str(),
    };
    let steps = tokenize(path);
    if steps.is_empty() {
        return false;
    }
    let head = match steps[0] {
        Step::Key(k) => k,
        _ => return false,
    };
    let tail = &steps[1..];
    match head {
        "status" => apply_status(tree, op, tail),
        "headers" => apply_headers(tree, op, tail),
        "body" => apply_body(tree.body, op, tail),
        _ => apply_body(tree.body, op, &steps),
    }
}

fn apply_status(tree: &mut ResponseTree<'_>, op: &PatchOp, tail: &[Step<'_>]) -> bool {
    if !tail.is_empty() {
        return false;
    }
    if let PatchOp::Set { value, .. } = op {
        if let Some(n) = value.as_u64() {
            *tree.status = n.min(599) as u16;
            return true;
        }
        if let Some(s) = value.as_str() {
            if let Ok(n) = s.parse::<u16>() {
                *tree.status = n.min(599);
                return true;
            }
        }
    }
    false
}

fn apply_headers(tree: &mut ResponseTree<'_>, op: &PatchOp, tail: &[Step<'_>]) -> bool {
    let name = match tail.first() {
        Some(Step::Key(k)) => *k,
        _ => return false,
    };
    let find = |h: &(String, String)| h.0.eq_ignore_ascii_case(name);
    match op {
        PatchOp::Set { value, .. } => {
            let v = match value {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            if let Some(existing) = tree.headers.iter_mut().find(|h| find(h)) {
                existing.1 = v;
            } else {
                tree.headers.push((name.to_string(), v));
            }
            true
        }
        PatchOp::Delete { .. } => {
            let before = tree.headers.len();
            tree.headers.retain(|h| !find(h));
            tree.headers.len() != before
        }
        PatchOp::Append { .. } => false,
    }
}

fn apply_body(body: &mut Value, op: &PatchOp, tail: &[Step<'_>]) -> bool {
    if tail.is_empty() {
        // Replacing the whole body: only allowed for set.
        if let PatchOp::Set { value, .. } = op {
            *body = value.clone();
            return true;
        }
        return false;
    }
    let (last, parents) = tail.split_last().unwrap();
    let parent = match navigate_to_parent(body, parents) {
        Some(p) => p,
        None => return false,
    };
    match (op, *last) {
        (PatchOp::Set { value, .. }, Step::Key(k)) => set_key(parent, k, value.clone()),
        (PatchOp::Set { value, .. }, Step::Index(i)) => set_index(parent, i, value.clone()),
        (PatchOp::Set { value, .. }, Step::Append) => append(parent, value.clone()),
        (PatchOp::Append { value, .. }, _) => {
            // Whatever the final segment is, append means push to an array.
            let target = match *last {
                Step::Key(k) => match parent {
                    Value::Object(map) => map.entry(k).or_insert_with(|| Value::Array(Vec::new())),
                    _ => return false,
                },
                Step::Index(i) => match parent {
                    Value::Array(arr) => {
                        if i >= arr.len() {
                            return false;
                        }
                        &mut arr[i]
                    }
                    _ => return false,
                },
                Step::Append => parent,
            };
            if let Value::Array(arr) = target {
                arr.push(value.clone());
                true
            } else {
                false
            }
        }
        (PatchOp::Delete { .. }, Step::Key(k)) => match parent {
            Value::Object(map) => map.remove(k).is_some(),
            _ => false,
        },
        (PatchOp::Delete { .. }, Step::Index(i)) => match parent {
            Value::Array(arr) => {
                if i < arr.len() {
                    arr.remove(i);
                    true
                } else {
                    false
                }
            }
            _ => false,
        },
        (PatchOp::Delete { .. }, Step::Append) => false,
    }
}

/// Walk the path through the JSON tree, creating intermediate objects on
/// missing keys. Returns the parent of the final step so the caller can
/// apply the actual operation in place.
fn navigate_to_parent<'a>(root: &'a mut Value, steps: &[Step<'_>]) -> Option<&'a mut Value> {
    let mut cur = root;
    for step in steps {
        cur = match step {
            Step::Key(k) => {
                if !cur.is_object() {
                    *cur = Value::Object(serde_json::Map::new());
                }
                cur.as_object_mut()?.entry(*k).or_insert(Value::Null)
            }
            Step::Index(i) => {
                if !cur.is_array() {
                    return None;
                }
                let arr = cur.as_array_mut()?;
                if *i >= arr.len() {
                    return None;
                }
                &mut arr[*i]
            }
            Step::Append => return None,
        };
    }
    Some(cur)
}

fn set_key(parent: &mut Value, k: &str, value: Value) -> bool {
    if !parent.is_object() {
        *parent = Value::Object(serde_json::Map::new());
    }
    if let Some(map) = parent.as_object_mut() {
        map.insert(k.to_string(), value);
        true
    } else {
        false
    }
}

fn set_index(parent: &mut Value, i: usize, value: Value) -> bool {
    match parent {
        Value::Array(arr) => {
            if i < arr.len() {
                arr[i] = value;
                true
            } else {
                false
            }
        }
        _ => false,
    }
}

fn append(parent: &mut Value, value: Value) -> bool {
    match parent {
        Value::Array(arr) => {
            arr.push(value);
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn run(
        mut status: u16,
        mut headers: Vec<(String, String)>,
        mut body: Value,
        ops: Vec<PatchOp>,
    ) -> (u16, Vec<(String, String)>, Value) {
        let mut tree = ResponseTree {
            status: &mut status,
            headers: &mut headers,
            body: &mut body,
        };
        for op in &ops {
            apply(&mut tree, op);
        }
        (status, headers, body)
    }

    #[test]
    fn set_body_field() {
        let (_, _, b) = run(
            200,
            vec![],
            json!({"user": {"fio": "old", "uid": 1}}),
            vec![PatchOp::Set {
                path: "body.user.fio".into(),
                value: json!("new"),
            }],
        );
        assert_eq!(b["user"]["fio"], "new");
    }

    #[test]
    fn set_status() {
        let (s, _, _) = run(
            200,
            vec![],
            json!({}),
            vec![PatchOp::Set {
                path: "status".into(),
                value: json!(404),
            }],
        );
        assert_eq!(s, 404);
    }

    #[test]
    fn set_header_case_insensitive() {
        let (_, h, _) = run(
            200,
            vec![("Content-Type".into(), "application/json".into())],
            json!({}),
            vec![PatchOp::Set {
                path: "headers.content-type".into(),
                value: json!("text/plain"),
            }],
        );
        assert_eq!(h, vec![("Content-Type".into(), "text/plain".into())]);
    }

    #[test]
    fn append_to_array() {
        let (_, _, b) = run(
            200,
            vec![],
            json!({"list": [1, 2]}),
            vec![PatchOp::Append {
                path: "body.list".into(),
                value: json!(3),
            }],
        );
        assert_eq!(b["list"], json!([1, 2, 3]));
    }

    #[test]
    fn delete_field() {
        let (_, _, b) = run(
            200,
            vec![],
            json!({"a": 1, "b": 2}),
            vec![PatchOp::Delete {
                path: "body.a".into(),
            }],
        );
        assert!(b.get("a").is_none());
        assert_eq!(b["b"], 2);
    }

    #[test]
    fn delete_array_index() {
        let (_, _, b) = run(
            200,
            vec![],
            json!({"l": [10, 20, 30]}),
            vec![PatchOp::Delete {
                path: "body.l[1]".into(),
            }],
        );
        assert_eq!(b["l"], json!([10, 30]));
    }

    #[test]
    fn body_prefix_is_optional() {
        let (_, _, b) = run(
            200,
            vec![],
            json!({"user": {"fio": "old"}}),
            vec![PatchOp::Set {
                path: "user.fio".into(),
                value: json!("new"),
            }],
        );
        assert_eq!(b["user"]["fio"], "new");
    }

    #[test]
    fn creates_missing_intermediate_objects_on_set() {
        let (_, _, b) = run(
            200,
            vec![],
            json!({}),
            vec![PatchOp::Set {
                path: "body.user.profile.name".into(),
                value: json!("X"),
            }],
        );
        assert_eq!(b["user"]["profile"]["name"], "X");
    }
}
