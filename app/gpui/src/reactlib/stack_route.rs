//! The stack builder's route, after `app/web/src/lib/stackRoute.ts`.
//!
//! React keeps the builder's name in the URL (`/effects/?stack=<name>`) and
//! the page reads it back with `useStackParam`. The GPUI editor has no URL
//! bar, so the same route lives in the page's state: the name that is open,
//! and the way in and out of it. The URL React would show is still produced
//! by [`StackRoute::url`], so a capture names the same place React does.

#![allow(dead_code)]

/// The builder's route: the `?stack=` value, and the way in and out.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StackRoute {
    /// The stack that is open: its name, or `Some("new")` for a stack that has
    /// never been saved.
    name: Option<String>,
}

/// The name a new stack opens under (`?stack=new`).
pub const NEW_STACK: &str = "new";

impl StackRoute {
    /// The closed route: the effects library, with no builder.
    pub fn new() -> Self {
        Self { name: None }
    }

    /// The route that opens a stack by name.
    pub fn open(name: &str) -> Self {
        Self { name: Some(name.to_string()) }
    }

    /// The stack that is open, if any.
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Whether the builder is on screen.
    pub fn is_open(&self) -> bool {
        self.name.is_some()
    }

    /// Whether the open stack is a new one (`?stack=new`).
    pub fn is_new(&self) -> bool {
        self.name.as_deref() == Some(NEW_STACK)
    }

    /// Opens a stack (React's `go(name)`).
    pub fn go(&mut self, name: &str) {
        self.name = Some(name.to_string());
    }

    /// Closes the builder (React's `go(null)`).
    pub fn close(&mut self) {
        self.name = None;
    }

    /// The URL React would show for this route: `/effects/` with no builder,
    /// `/effects/?stack=<name>` with one.
    pub fn url(&self) -> String {
        match &self.name {
            None => "/effects/".to_string(),
            Some(name) => format!("/effects/?stack={}", encode_component(name)),
        }
    }
}

/// `encodeURIComponent` for a path query (`stackRoute.ts`).
fn encode_component(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.as_bytes() {
        let c = *byte as char;
        if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '!' | '~' | '*' | '\'' | '(' | ')') {
            out.push(c);
        } else if c == ' ' {
            out.push_str("%20");
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_closed_route_is_the_effects_page() {
        let route = StackRoute::new();
        assert!(!route.is_open());
        assert_eq!(route.name(), None);
        assert_eq!(route.url(), "/effects/");
    }

    #[test]
    fn a_named_stack_opens_the_builder() {
        let route = StackRoute::open("Sunset glow");
        assert!(route.is_open());
        assert_eq!(route.name(), Some("Sunset glow"));
        assert!(!route.is_new());
        assert_eq!(route.url(), "/effects/?stack=Sunset%20glow");
    }

    #[test]
    fn the_new_stack_is_named_new() {
        let route = StackRoute::open(NEW_STACK);
        assert!(route.is_new());
        assert_eq!(route.url(), "/effects/?stack=new");
    }

    #[test]
    fn going_in_and_out_keeps_the_one_name() {
        let mut route = StackRoute::new();
        route.go("Sunset glow");
        assert_eq!(route.name(), Some("Sunset glow"));
        route.go(NEW_STACK);
        assert_eq!(route.name(), Some("new"));
        route.close();
        assert!(!route.is_open());
        assert_eq!(route.url(), "/effects/");
    }

    #[test]
    fn a_name_with_odd_characters_is_encoded_the_same_way() {
        let route = StackRoute::open("a b&c");
        assert_eq!(route.url(), "/effects/?stack=a%20b%26c");
    }
}
