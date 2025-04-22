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
    // 1. Create the OrderBook wrapped in Arc and Mutex
    let order_book = Arc::new(Mutex::new(OrderBook::new()));
    println!("Initial empty book created."); // Fixed typo

    // 2. create a place to store thread handles
    let mut handles = vec![];

    // 3. Spawn mutliple threads
    for i in 0..5 { // spawn 5 threads
        // 3a. Clone the Arc for the new thread.
        let book_clone = Arc::clone(&order_book);

        // 3b. spawn the thread
        let handle = thread::spawn(move || {
            // This code runs in the new thread
            let thread_id = i + 1; // Simple ID for the thread

            // Create a unique order for this thread
            let order = if thread_id % 2 == 0 {
                Order::new(100 + thread_id, Side::Sell, 100 + thread_id as u64, 10 + thread_id as u64)
            } else {
                Order::new(100 + thread_id, Side::Buy, 100 - thread_id as u64, 15 + thread_id as u64)
            };
            println!("Thread {} trying to add order: {:?}", thread_id, order);

            // --- FIX 1: Use '=' for assignment ---
            let mut book_guard = book_clone.lock().unwrap();
            println!("Thread {} acquired lock.", thread_id);

            // Call add_order using the guard
            book_guard.add_order(order);

            // Lock is released automatically when book_guard goes out of scope
            println!("Thread {} released lock.", thread_id);

            // Optional delay
            thread::sleep(Duration::from_millis(10));

        }); // end of thread closure

        handles.push(handle);

    } // end of loop spawning threads

    // 4. Wait for all threads to finish
    println!("Main thread waiting for worker threads to finish...");
    for handle in handles { // Renamed 'handlie' to 'handle'
        handle.join().unwrap(); // Use the loop variable 'handle' here
    }

    println!("All threads finished."); // Fixed typo

    // 5. Print the final state
    let final_book = order_book.lock().unwrap();
    println!("\n--- Final Order Book State ---");
    println!("{:?}", *final_book); // Use * to dereference the MutexGuard

}
