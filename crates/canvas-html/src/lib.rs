//! SignalRGB-format HTML effect runtime. Spine stub: the HTML area replaces the bodies, never
//! the signatures.

use canvas_proto::{Canvas, Effect, Inputs, PropertySchema, Props};

#[derive(Debug, thiserror::Error)]
pub enum HtmlError {
    #[error("parse: {0}")]
    Parse(String),
    #[error("script: {0}")]
    Script(String),
}

/// One HTML effect instance (its own JS context and 2D canvas).
pub struct HtmlEffect {
    schema: PropertySchema,
}

impl HtmlEffect {
    /// `id` becomes `schema.id`; name/description/properties come from the file's
    /// `<title>` and `<meta>` tags. `default_overrides` replace the file's declared defaults
    /// for the keys given (keys are the file's `property` names, e.g. `speedRaw`).
    pub fn from_source(id: &str, html: &str, default_overrides: &Props) -> Result<Self, HtmlError> {
        let _ = (id, html, default_overrides);
        todo!("canvas-html")
    }
}

impl Effect for HtmlEffect {
    fn schema(&self) -> PropertySchema {
        self.schema.clone()
    }
    fn init(&mut self, props: &Props) {
        let _ = props;
        todo!()
    }
    fn update(&mut self, dt: f32, canvas: &mut Canvas, inputs: &Inputs) {
        let _ = (dt, canvas, inputs);
        todo!()
    }
}
