use serde::Serialize;

use crate::{error::PromptTemplateError, PromptContext};

pub(crate) fn render_prompt<T: Serialize>(
    template_source: &str,
    issue: &T,
    attempt: Option<u32>,
) -> Result<String, PromptTemplateError> {
    let parser = liquid::ParserBuilder::with_stdlib()
        .build()
        .map_err(|source| PromptTemplateError::Parse { source })?;

    let template = parser
        .parse(template_source)
        .map_err(|source| PromptTemplateError::Parse { source })?;
    let globals = liquid::to_object(&PromptContext { issue, attempt })
        .map_err(|source| PromptTemplateError::Context { source })?;

    template
        .render(&globals)
        .map_err(|source| PromptTemplateError::Render { source })
}
