use std::path::Path;

use opensymphony::opensymphony_memory::{parse_okf_concept, render_okf_concept};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let markdown = r#"---
type: topic-doc
area: openhands-runtime
visibility: public
docs_sync:
  status: pending
---

# Runtime

See [COE-123](/issues/COE-123.md).
"#;

    let concept = parse_okf_concept(Path::new("."), Path::new("./areas/./runtime.md"), markdown)?;
    println!("id={}", concept.id);
    println!("path={}", concept.path.as_path().display());
    println!("derived_opensymphony={}", concept.derived_opensymphony);
    println!("{}", render_okf_concept(&concept)?);
    Ok(())
}
