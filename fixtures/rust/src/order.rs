use crate::pricing::discount;

#[derive(Default)]
pub struct Order {
    pub items: Vec<u32>,
}

impl Order {
    pub fn total(&self) -> u32 {
        discount(self.subtotal())
    }

    fn subtotal(&self) -> u32 {
        self.items.iter().sum()
    }
}
