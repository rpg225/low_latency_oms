use axum::{
    routing::{get, post},
    Router,
    response::Json,
    extract::State,
};

use std::net::SocketAddr;
use tokio::net::TcpListener;
use serde::{Deserialize, Serialize};


// Add tracing imports
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};


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

#[tokio::main] async fn main() {
  // --- Basic Loggin Setup
  tracing_subscriber::registry()
    .with(
        tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_|
                "low_latency_oms=debug,tower_http=debug".into()),

            )
            .with(tracing_subscriber::fmt::layer())
            .init();
            
            tracing::info!("Logger initialized");
            // --- End Logging Setup ---

            // --- Create Shared State (OrderBook) ---
            // use later
            let order_book_state = Arc::new(Mutex::new(OrderBook::new()));

            tracing::info!("Shared OrderBook state created.");
            // -- End Shared State ---

            // -- Define API Routes ---
            let app = Router::new()
                // Simple GET route for testing
                .route("/", get(root_handler))

                .with_state(order_book_state); // make the order book available to handlers

            tracing::info!("API routes defined.");
            // --- End API Routes

            // -- Run the server ---

            let addr = SocketAddr::from(([127, 0, 0,1], 3000));

            tracing::info!("Statring server on {}", addr);

            let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
            axum::serve(listener, app).await.unwrap();
            // -- End Run Server ---
}


    // --- Basic Handler Function ---
    // This function will handle requests to the "/" route
    async  fn root_handler() -> &'static str {
        tracing::info!("Root handler called");
        "Hello from Low Latency OMS" // Send a simple text response
    }