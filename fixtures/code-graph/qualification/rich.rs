pub trait Renderable {
    fn render(&self) -> String;
}

pub struct Widget {
    pub value: i32,
}

pub struct Receipt {
    pub id: WidgetId,
}

pub enum Mode {
    Fast,
    Safe,
}

pub type WidgetId = u64;
pub const DEFAULT_WIDGET: WidgetId = 1;

macro_rules! widget {
    () => { Widget { value: 1 } };
}

impl Renderable for Widget {
    fn render(&self) -> String {
        target();
        target();
        String::new()
    }
}

pub fn target() {}

pub fn receipt() -> Receipt {
    Receipt { id: DEFAULT_WIDGET }
}

#[test]
fn target_is_reachable() {
    target();
}
