use std::time::{SystemTime, UNIX_EPOCH};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use std::collections::VecDeque;

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

// Order Book Structure
#[derive(Debug, Default)]
pub struct OrderBook {
    bids: VecDeque<Order>,
    asks: VecDeque<Order>,
}

impl OrderBook {
    pub fn new() -> Self {
        OrderBook {
            bids: VecDeque::new(),
            asks: VecDeque::new(),
        }
    }

    
    fn try_match(&mut self) {
        println!("Attempting match...");
        // Check if there are orders on both sides
        if let (Some(best_bid), Some(best_ask)) = (self.bids.front_mut(), self.asks.front_mut()) {
             println!("Best Bid: P={}, Q={}. Best Ask: P={}, Q={}", best_bid.price, best_bid.quantity, best_ask.price, best_ask.quantity);

            // Match condition: highest bid price >= lowest ask price?
            if best_bid.price >= best_ask.price {
                println!("MATCH FOUND!");

                let matched_quantity = std::cmp::min(best_bid.quantity, best_ask.quantity);
                println!("Matched Quantity: {}", matched_quantity);

                best_bid.quantity -= matched_quantity;
                best_ask.quantity -= matched_quantity;

                // Store IDs before potentially removing the orders
                let bid_id = best_bid.id;
                let ask_id = best_ask.id;

                // Remove filled orders (quantity is 0)
                if best_bid.quantity == 0 {
                    self.bids.pop_front(); // Remove the filled bid
                    println!("Bid order {} fully filled.", bid_id);
                }
                if best_ask.quantity == 0 {
                    self.asks.pop_front(); // Remove the filled ask
                    println!("Ask order {} fully filled.", ask_id);
                }

                 // Potentially more matches possible, try again
                 if !self.bids.is_empty() && !self.asks.is_empty() {
                      println!("Checking for further matches...");
                      self.try_match(); // Recursive call is okay here
                 }

            } else {
                println!("No match possible (bid price < ask price)");
            }
        } else {
             println!("No match possible (one side is empty)");
        }
    }

    
    pub fn add_order(&mut self, order: Order) {
        let order_id = order.id; // Store the ID
        let side = order.side.clone(); // Clone the side

        match side { // Use the 'side' variable
            Side::Buy => {
                self.bids.push_back(order); // Move the original 'order' here
            }
            Side::Sell => {
                self.asks.push_back(order); // Move the original 'order' here
            }
        }

       
        println!(
            "Added order {}. Book state before match attempt: {:?}",
            order_id, // Use the saved ID here!
            self
        );
        self.try_match(); // Call matching logic
        println!("Book state after match attempt: {:?}", self);
        
    } 

}

fn main() {
    let mut book = OrderBook::new();

    // Scenario 1: Simple match
    println!("--- Scenario 1 ---");
    book.add_order(Order::new(1, Side::Buy, 100, 10)); // Buy 10 @ 100
    book.add_order(Order::new(2, Side::Sell, 100, 5)); // Sell 5 @ 100 (Partial fill for Buy order 1)
    // Expected: Match 5 shares. Bid order 1 has 5 remaining. Ask order 2 is gone.

    println!("\n--- Scenario 2 ---");
    book.add_order(Order::new(3, Side::Sell, 99, 5)); // Sell 5 @ 99 (Should match remaining 5 of order 1)
    // Expected: Match 5 shares. Bid order 1 is gone. Ask order 3 is gone.

    println!("\n--- Scenario 3 ---");
    book.add_order(Order::new(4, Side::Buy, 102, 20));  // Buy 20 @ 102
    book.add_order(Order::new(5, Side::Sell, 101, 10)); // Sell 10 @ 101
    book.add_order(Order::new(6, Side::Sell, 102, 15)); // Sell 15 @ 102
    // Expected: Order 4 matches all of Order 5 (Buy 10 @ 101). Order 4 has 10 left.
    // Then: Order 4 (10 left @ 102) matches 10 of Order 6 (Sell 10 @ 102). Order 4 gone. Order 6 has 5 left.
}
