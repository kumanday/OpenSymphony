fn top_level() -> usize {
    1
}

pub struct Widget {
    id: usize,
}

enum Mode {
    Fast,
    Slow,
}

trait Runnable {
    fn run(&self);
}

impl Widget {
    pub fn new(id: usize) -> Self {
        Self { id }
    }

    fn run(&self) -> usize {
        self.id
    }
}

#[test]
/// Keeps test attribute detection working when comments sit next to attributes.
fn exercises_widget() {
    let widget = Widget::new(1);
    assert_eq!(widget.run(), 1);
}
