//! Patches to the view tree (`Baguette.Patch`): what changed since the last
//! frame, applied in place of a whole new tree.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::view::Node;

/// One change. `at` is the path from the root, as child indices.
#[derive(Deserialize, Debug)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum Op {
    Replace {
        at: Vec<usize>,
        node: Option<Node>,
    },
    Attrs {
        at: Vec<usize>,
        a: BTreeMap<String, String>,
    },
    /// Removes `delete` children from `start` and inserts `c` there.
    Splice {
        at: Vec<usize>,
        start: usize,
        delete: usize,
        c: Vec<Node>,
    },
}

/// Applies `ops` in order. On an error the tree is part patched, and the
/// host asks Elixir for the whole frame.
pub fn apply(root: &mut Option<Node>, ops: Vec<Op>) -> Result<(), String> {
    for op in ops {
        match op {
            Op::Replace { at, node } if at.is_empty() => *root = node,
            Op::Replace { at, node } => {
                *node_at(root, &at)? = node.ok_or("no node to replace with")?;
            }
            Op::Attrs { at, a } => match node_at(root, &at)? {
                Node::Element { attrs, .. } => *attrs = a,
                Node::Text(_) => return Err(format!("{at:?} is text, not an element")),
            },
            Op::Splice {
                at,
                start,
                delete,
                c,
            } => match node_at(root, &at)? {
                Node::Element { children, .. } if start + delete <= children.len() => {
                    children.splice(start..start + delete, c);
                }
                _ => return Err(format!("no children {start}..{} at {at:?}", start + delete)),
            },
        }
    }
    Ok(())
}

fn node_at<'a>(root: &'a mut Option<Node>, at: &[usize]) -> Result<&'a mut Node, String> {
    let mut node = root.as_mut().ok_or("no tree to patch")?;
    for (depth, &i) in at.iter().enumerate() {
        node = match node {
            Node::Element { children, .. } => children.get_mut(i),
            Node::Text(_) => None,
        }
        .ok_or_else(|| format!("no node at {:?}", &at[..=depth]))?;
    }
    Ok(node)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Deserialize)]
    struct Case {
        name: String,
        old: Option<Node>,
        new: Option<Node>,
        patch: Vec<Op>,
    }

    // The same cases Elixir's diff is tested against (Baguette.PatchTest).
    #[test]
    fn rebuilds_each_new_tree_from_the_old_one() {
        let cases: Vec<Case> =
            serde_json::from_str(include_str!("../../fixtures/patches.json")).unwrap();
        for Case {
            name,
            mut old,
            new,
            patch,
        } in cases
        {
            apply(&mut old, patch).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(old, new, "{name}");
        }
    }

    #[test]
    fn reports_patches_that_dont_fit() {
        let ops = |json: &str| -> Vec<Op> { serde_json::from_str(json).unwrap() };
        let mut tree = Some(Node::Element {
            tag: "div".into(),
            attrs: BTreeMap::new(),
            children: vec![Node::Text("x".into())],
        });
        assert!(apply(&mut tree, ops(r#"[{"op":"replace","at":[3],"node":"y"}]"#)).is_err());
        assert!(apply(&mut tree, ops(r#"[{"op":"attrs","at":[0],"a":{}}]"#)).is_err());
        let splice = r#"[{"op":"splice","at":[],"start":1,"delete":1,"c":[]}]"#;
        assert!(apply(&mut tree, ops(splice)).is_err());
        assert!(apply(&mut None, ops(r#"[{"op":"attrs","at":[],"a":{}}]"#)).is_err());
    }
}
