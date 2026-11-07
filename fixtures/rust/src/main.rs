use shop::Order;

fn main() {
    let order = Order::default();
    println!("{}", order.total());
}
