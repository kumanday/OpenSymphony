use std::collections::HashMap;

fn build() -> HashMap<String, usize> {
    HashMap::new()
}

#[test]
fn builds_map() {
    let map = build();
    assert!(map.is_empty());
}
