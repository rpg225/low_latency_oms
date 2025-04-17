use std::time::{SystemTime, UNIX_EPOCH};

// Unique ID for each order
type OrderId = u64;


// Represents Buy or Sell

#[derive(Debug, Clone, PartialEq, Eq)]

pub enum Side {
    Buy,
    Sell,
}

// Our main Order structure

#[derive(Debug, Clone)]
pub struct Order {
    id: OrderId,
    side: Side,
    price: u64, // using integers for price 
    quantity: u64,
    timestamp: u128,
}

impl Order {
    // Function to create a new order
    pub fn new(id: OrderId, side: Side, price: u64, quantity: u64) -> Self {
        Order { 
            id, 
            side, 
            price, 
            quantity, 
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("Time went backwards")
                .as_nanos(),
        }
    }
}


fn main() {
    let buy_order = Order::new(1, Side::Buy, 100, 50);
    let sell_order = Order::new(2, Side::Sell, 105, 30);
    println!("Created Buy Order: {:?}", buy_order);
    println!("Created Sell Order: {:?}", sell_order);
}
