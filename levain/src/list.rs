//! Virtualized lists (`<list rows phx-rows>`): the rows the host has of
//! each, and which to ask Elixir for as they scroll into view.

use std::collections::BTreeMap;
use std::ops::Range;

use serde::Deserialize;
use tauri_plugin_gpui::gpui::UniformListScrollHandle;

use crate::view::Node;

/// Rows of one list from Elixir, starting at index `start`.
#[derive(Deserialize, Default)]
pub struct Segment {
    /// The list's id; frames key segments by it instead.
    #[serde(default)]
    pub id: String,
    pub start: usize,
    pub rows: Vec<Node>,
}

/// A request for rows `start..end`, and the range on screen (with a
/// screen's margin either side) that Elixir keeps fresh on each render.
#[derive(Debug, PartialEq)]
pub struct Want {
    pub rows: Range<usize>,
    pub visible: Range<usize>,
}

pub struct ListState {
    pub rows: BTreeMap<usize, Node>,
    pub count: usize,
    pub event: Option<String>,
    /// The `<list>`'s element path; rows are `<path>.rows.<index>`.
    pub path: String,
    /// How tall a row is drawn before it arrives.
    pub row_height: f32,
    pub scroll: UniformListScrollHandle,
    pending: Option<Range<usize>>,
}

impl Default for ListState {
    fn default() -> Self {
        Self {
            rows: BTreeMap::new(),
            count: 0,
            event: None,
            path: String::new(),
            row_height: 24.,
            scroll: UniformListScrollHandle::new(),
            pending: None,
        }
    }
}

impl ListState {
    /// What to ask for to show `visible`: nothing when every row in it is
    /// here or already asked for, else the missing rows within a screen
    /// either side of it.
    pub fn wanted(&mut self, visible: Range<usize>) -> Option<Want> {
        let visible = visible.start.min(self.count)..visible.end.min(self.count);
        let lacks = |state: &Self, i: &usize| {
            !state.rows.contains_key(i) && !state.pending.as_ref().is_some_and(|p| p.contains(i))
        };
        if !visible.clone().any(|i| lacks(self, &i)) {
            return None;
        }
        let margin = visible.len();
        let around = visible.start.saturating_sub(margin)..(visible.end + margin).min(self.count);
        let first = around.clone().find(|i| lacks(self, i))?;
        let last = around.clone().rev().find(|i| lacks(self, i))?;
        let rows = first..last + 1;
        self.pending = Some(rows.clone());
        Some(Want {
            rows,
            visible: around,
        })
    }

    /// Rows Elixir sent, in reply to a request.
    pub fn insert(&mut self, segment: Segment) {
        let end = segment.start + segment.rows.len();
        if self
            .pending
            .as_ref()
            .is_some_and(|p| p.start < end && segment.start < p.end)
        {
            self.pending = None;
        }
        self.rows.extend(
            (segment.start..)
                .zip(segment.rows)
                .filter(|(i, _)| *i < self.count),
        );
    }

    /// A new frame: its rows replace all the others, which are stale.
    pub fn reset(&mut self, segment: Option<Segment>) {
        self.rows.clear();
        self.pending = None;
        if let Some(segment) = segment {
            self.insert(segment);
        }
    }

    /// The row GPUI measures every row's height from: the first one here.
    pub fn measured(&self) -> usize {
        self.rows.keys().next().copied().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(count: usize) -> ListState {
        ListState {
            count,
            ..ListState::default()
        }
    }

    fn rows(start: usize, n: usize) -> Segment {
        Segment {
            id: "jobs".into(),
            start,
            rows: (start..start + n)
                .map(|i| Node::Text(i.to_string()))
                .collect(),
        }
    }

    #[test]
    fn asks_for_missing_rows_with_a_screen_either_side() {
        let mut list = list(1000);
        assert_eq!(
            list.wanted(100..110),
            Some(Want {
                rows: 90..120,
                visible: 90..120
            })
        );
        // Already asked for.
        assert_eq!(list.wanted(100..110), None);
    }

    #[test]
    fn asks_only_for_what_it_lacks() {
        let mut list = list(1000);
        list.insert(rows(90, 30));
        assert_eq!(list.wanted(95..105), None);
        assert_eq!(
            list.wanted(115..125),
            Some(Want {
                rows: 120..135,
                visible: 105..135
            })
        );
    }

    #[test]
    fn stays_within_the_list() {
        let mut list = list(12);
        assert_eq!(
            list.wanted(5..20),
            Some(Want {
                rows: 0..12,
                visible: 0..12
            })
        );
        list.insert(rows(10, 5));
        assert!(!list.rows.contains_key(&12));
    }

    #[test]
    fn a_reply_clears_the_request() {
        let mut list = list(100);
        list.wanted(0..10);
        list.insert(rows(0, 20));
        assert_eq!(list.wanted(0..10), None);
        list.reset(None);
        assert!(list.wanted(0..10).is_some());
    }

    #[test]
    fn a_frame_replaces_the_rows() {
        let mut list = list(100);
        list.insert(rows(0, 50));
        list.reset(Some(rows(40, 5)));
        assert_eq!(
            list.rows.keys().copied().collect::<Vec<_>>(),
            [40, 41, 42, 43, 44]
        );
        assert_eq!(list.measured(), 40);
    }

    #[test]
    fn an_empty_list_wants_nothing() {
        assert_eq!(list(0).wanted(0..10), None);
        assert_eq!(list(0).measured(), 0);
    }
}
