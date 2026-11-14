//! Patches to the view tree (the language package's diff): what changed since the last
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
/// host asks the app for the whole frame.
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

    // The same cases the app's diff is tested against (the conformance fixtures).
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

    fn el(attrs: &[(&str, &str)], children: Vec<Node>) -> Node {
        Node::Element {
            tag: "div".into(),
            attrs: attrs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            children,
        }
    }

    fn text(s: &str) -> Node {
        Node::Text(s.into())
    }

    fn ops(json: &str) -> Vec<Op> {
        serde_json::from_str(json).unwrap()
    }

    fn tree() -> Option<Node> {
        Some(el(
            &[("class", "a"), ("id", "x")],
            vec![text("0"), el(&[], vec![text("1.0")]), text("2")],
        ))
    }

    #[test]
    fn attrs_replace_the_whole_map() {
        let mut t = tree();
        apply(
            &mut t,
            ops(r#"[{"op": "attrs", "at": [], "a": {"title": "t"}}]"#),
        )
        .unwrap();
        let Some(Node::Element { attrs, .. }) = t else {
            panic!()
        };
        // `class` and `id` are gone, not merged with.
        assert_eq!(
            attrs.into_iter().collect::<Vec<_>>(),
            [("title".into(), "t".into())]
        );
    }

    #[test]
    fn splices_delete_and_insert_in_one_step() {
        let mut t = tree();
        let splice =
            r#"[{"op": "splice", "at": [], "start": 1, "delete": 2, "c": ["x", "y", "z"]}]"#;
        apply(&mut t, ops(splice)).unwrap();
        let Some(Node::Element { children, .. }) = &t else {
            panic!()
        };
        assert_eq!(children, &[text("0"), text("x"), text("y"), text("z")]);

        // Pure insertion at the end, pure deletion at the start.
        apply(
            &mut t,
            ops(r#"[{"op": "splice", "at": [], "start": 4, "delete": 0, "c": ["end"]}]"#),
        )
        .unwrap();
        apply(
            &mut t,
            ops(r#"[{"op": "splice", "at": [], "start": 0, "delete": 1, "c": []}]"#),
        )
        .unwrap();
        let Some(Node::Element { children, .. }) = &t else {
            panic!()
        };
        assert_eq!(children, &[text("x"), text("y"), text("z"), text("end")]);
    }

    #[test]
    fn splices_reach_nested_children() {
        let mut t = tree();
        let splice = r#"[{"op": "splice", "at": [1], "start": 0, "delete": 0, "c": ["new"]}]"#;
        apply(&mut t, ops(splice)).unwrap();
        assert_eq!(
            t,
            Some(el(
                &[("class", "a"), ("id", "x")],
                vec![
                    text("0"),
                    el(&[], vec![text("new"), text("1.0")]),
                    text("2")
                ]
            ))
        );
    }

    #[test]
    fn a_splice_must_stay_inside_the_children() {
        let mut t = tree();
        let before = t.clone();
        // 3 children: 2 + 2 runs past the end; 3 + 0 is fine (append).
        let past = r#"[{"op": "splice", "at": [], "start": 2, "delete": 2, "c": []}]"#;
        let error = apply(&mut t, ops(past)).unwrap_err();
        assert_eq!(error, "no children 2..4 at []");
        assert_eq!(t, before);
        assert!(
            apply(
                &mut t,
                ops(r#"[{"op": "splice", "at": [], "start": 3, "delete": 0, "c": []}]"#)
            )
            .is_ok()
        );
        // A text node has no children to splice.
        let in_text = r#"[{"op": "splice", "at": [0], "start": 0, "delete": 0, "c": []}]"#;
        assert_eq!(
            apply(&mut t, ops(in_text)).unwrap_err(),
            "no children 0..0 at [0]"
        );
    }

    #[test]
    fn errors_say_where_the_path_ran_out() {
        let mut t = tree();
        let missing = r#"[{"op": "replace", "at": [1, 5], "node": "x"}]"#;
        assert_eq!(
            apply(&mut t, ops(missing)).unwrap_err(),
            "no node at [1, 5]"
        );
        // Through a text node.
        let through = r#"[{"op": "replace", "at": [0, 0], "node": "x"}]"#;
        assert_eq!(
            apply(&mut t, ops(through)).unwrap_err(),
            "no node at [0, 0]"
        );
        let attrs_on_text = r#"[{"op": "attrs", "at": [2], "a": {}}]"#;
        assert_eq!(
            apply(&mut t, ops(attrs_on_text)).unwrap_err(),
            "[2] is text, not an element"
        );
        assert_eq!(
            apply(
                &mut None,
                ops(r#"[{"op": "replace", "at": [0], "node": "x"}]"#)
            )
            .unwrap_err(),
            "no tree to patch"
        );
    }

    #[test]
    fn only_the_root_may_be_replaced_by_nothing() {
        let mut t = tree();
        let nothing = r#"[{"op": "replace", "at": [1], "node": null}]"#;
        assert_eq!(
            apply(&mut t, ops(nothing)).unwrap_err(),
            "no node to replace with"
        );
        apply(
            &mut t,
            ops(r#"[{"op": "replace", "at": [], "node": null}]"#),
        )
        .unwrap();
        assert_eq!(t, None);
        // And a tree can start from nothing.
        apply(
            &mut t,
            ops(r#"[{"op": "replace", "at": [], "node": "hi"}]"#),
        )
        .unwrap();
        assert_eq!(t, Some(text("hi")));
    }

    #[test]
    fn ops_apply_in_order_and_stop_at_the_first_that_fails() {
        let mut t = tree();
        let patch = r#"[
            {"op": "replace", "at": [0], "node": "A"},
            {"op": "replace", "at": [9], "node": "?"},
            {"op": "replace", "at": [2], "node": "C"}
        ]"#;
        assert!(apply(&mut t, ops(patch)).is_err());
        let Some(Node::Element { children, .. }) = &t else {
            panic!()
        };
        // The first landed, the third did not.
        assert_eq!(children[0], text("A"));
        assert_eq!(children[2], text("2"));
        // Later ops see the earlier ones' changes: paths shift after a splice.
        let mut t = tree();
        let shifted = r#"[
            {"op": "splice", "at": [], "start": 0, "delete": 0, "c": ["new"]},
            {"op": "replace", "at": [2, 0], "node": "moved"}
        ]"#;
        apply(&mut t, ops(shifted)).unwrap();
        let Some(Node::Element { children, .. }) = &t else {
            panic!()
        };
        assert_eq!(children[2], el(&[], vec![text("moved")]));
    }

    #[test]
    fn unknown_ops_do_not_parse() {
        assert!(serde_json::from_str::<Vec<Op>>(r#"[{"op": "move", "at": []}]"#).is_err());
        assert!(serde_json::from_str::<Vec<Op>>(r#"[{"op": "attrs", "a": {}}]"#).is_err());
    }
}
