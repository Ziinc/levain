//! Keymaps from Elixir (`keymap` in `Baguette.View`) as GPUI key bindings,
//! and the one action type they all dispatch.

use std::collections::BTreeMap;
use std::rc::Rc;

use serde::Deserialize;
use tauri_plugin_gpui::gpui::{
    self, Action, Global, KeyBinding, KeyBindingContextPredicate, PlatformKeyboardMapper,
    SharedString,
};

/// A binding as a frame carries it.
#[derive(Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Binding {
    pub keys: String,
    pub action: String,
    #[serde(default)]
    pub context: Option<String>,
}

/// An action of the view's, named as Elixir names it. GPUI dispatches it
/// like any other; the host sends its name back.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct ElixirAction {
    pub name: SharedString,
}

impl Action for ElixirAction {
    fn boxed_clone(&self) -> Box<dyn Action> {
        Box::new(self.clone())
    }

    fn partial_eq(&self, action: &dyn Action) -> bool {
        (action as &dyn std::any::Any)
            .downcast_ref::<Self>()
            .is_some_and(|other| other == self)
    }

    fn name(&self) -> &'static str {
        Self::name_for_type()
    }

    fn name_for_type() -> &'static str {
        "baguette::ElixirAction"
    }

    fn build(value: serde_json::Value) -> gpui::Result<Box<dyn Action>> {
        Ok(Box::new(serde_json::from_value::<Self>(value)?))
    }
}

/// The key context of window `window`'s content, which scopes its view's
/// bindings to that window.
pub fn window_context(window: u32) -> String {
    format!("baguette-window-{window}")
}

/// Every window's bindings. GPUI's keymap is the app's, so each is scoped
/// to its window and the whole set is bound again when one changes.
#[derive(Default)]
struct Keymaps(BTreeMap<u32, Vec<Binding>>);

impl Global for Keymaps {}

/// Makes `bindings` window `window`'s, replacing the ones it had.
pub fn set(window: u32, bindings: &[Binding], cx: &mut gpui::App) {
    let keymaps = cx.default_global::<Keymaps>();
    if keymaps.0.get(&window).map(Vec::as_slice).unwrap_or(&[]) == bindings {
        return;
    }
    keymaps.0.insert(window, bindings.to_vec());
    let all = keymaps.0.clone();
    let mapper = cx.keyboard_mapper().clone();
    let ours: Vec<KeyBinding> = all
        .iter()
        .flat_map(|(window, bindings)| bindings.iter().map(|b| scoped(*window, b)))
        .filter_map(|binding| {
            key_binding(&binding, mapper.as_ref())
                .map_err(|error| crate::log_line(&format!("baguette: keymap: {error}")))
                .ok()
        })
        .collect();
    let others: Vec<KeyBinding> = cx
        .key_bindings()
        .borrow()
        .bindings()
        .filter(|key| key.action().name() != ElixirAction::name_for_type())
        .cloned()
        .collect();
    cx.clear_key_bindings();
    cx.bind_keys(others.into_iter().chain(ours));
}

/// `binding`, firing only inside window `window`.
fn scoped(window: u32, binding: &Binding) -> Binding {
    let window = window_context(window);
    Binding {
        context: Some(match &binding.context {
            Some(context) => format!("{window} > ({context})"),
            None => window,
        }),
        ..binding.clone()
    }
}

/// `binding` as a GPUI key binding, or why it can't be one.
pub fn key_binding(
    binding: &Binding,
    mapper: &dyn PlatformKeyboardMapper,
) -> Result<KeyBinding, String> {
    if binding.keys.trim().is_empty() {
        return Err(format!("`{}` has no keys", binding.action));
    }
    let context = match &binding.context {
        Some(context) => Some(Rc::new(
            KeyBindingContextPredicate::parse(context).map_err(|e| e.to_string())?,
        )),
        None => None,
    };
    let action = ElixirAction {
        name: binding.action.clone().into(),
    };
    KeyBinding::load(
        &binding.keys,
        Box::new(action),
        context,
        false,
        None,
        mapper,
    )
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri_plugin_gpui::gpui::DummyKeyboardMapper;

    fn binding(keys: &str, context: Option<&str>) -> Binding {
        Binding {
            keys: keys.into(),
            action: "open_palette".into(),
            context: context.map(Into::into),
        }
    }

    #[test]
    fn binds_keys_to_named_actions() {
        let key = key_binding(&binding("secondary-k", Some("Jobs")), &DummyKeyboardMapper).unwrap();
        let expected = ElixirAction {
            name: "open_palette".into(),
        };
        assert!(key.action().partial_eq(&expected));
        assert!(key.predicate().is_some());
        assert!(key_binding(&binding("escape", None), &DummyKeyboardMapper).is_ok());
    }

    #[test]
    fn rejects_what_gpui_cant_parse() {
        assert!(key_binding(&binding("", None), &DummyKeyboardMapper).is_err());
        assert!(key_binding(&binding("k", Some("Jobs &&")), &DummyKeyboardMapper).is_err());
    }

    #[test]
    fn scopes_bindings_to_their_window() {
        assert_eq!(
            scoped(2, &binding("escape", None)).context.as_deref(),
            Some("baguette-window-2")
        );
        let jobs = scoped(0, &binding("secondary-k", Some("Jobs && !Editing")));
        assert_eq!(
            jobs.context.as_deref(),
            Some("baguette-window-0 > (Jobs && !Editing)")
        );
        assert!(key_binding(&jobs, &DummyKeyboardMapper).is_ok());
    }

    #[test]
    fn actions_compare_by_name() {
        let a = ElixirAction { name: "a".into() };
        assert!(a.partial_eq(&ElixirAction { name: "a".into() }));
        assert!(!a.partial_eq(&ElixirAction { name: "b".into() }));
        let built = ElixirAction::build(serde_json::json!({"name": "a"})).unwrap();
        assert!(built.partial_eq(&a));
    }
}
