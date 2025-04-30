use axum::{
    routing::{get, post, put, delete}, // Added put, delete
    Router,
    response::Json,
    extract::{State, Path}, // Added Path
    http::StatusCode, // Added StatusCode
};
use std::net::SocketAddr;
use tokio::net::TcpListener;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU64, Ordering};
use std::collections::VecDeque;
use std::time::{SystemTime, UNIX_EPOCH};

// Tracing / Logging
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use tracing; // <-- Added base tracing import

// --- Core Data Structures ---

// Unique ID for each order
type OrderId = u64;

// Represents Buy or Sell
// FIX 1: Added Serialize/Deserialize HERE
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Side {
    Buy,
    Sell,
}

// Our main Order structure
// FIX 1: Added Serialize/Deserialize HERE
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Order {
    id: OrderId,
    side: Side,
    price: u64, // using integers for price
    quantity: u64,
    timestamp: u128, // Note: u128 might not be ideal for all JSON usages, but okay for now
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

    // --- Add Order and Matching Logic (from previous steps) ---
    pub fn add_order(&mut self, order: Order) {
        let order_id = order.id;
        let side = order.side.clone();

        match side {
            Side::Buy => {
                self.bids.push_back(order);
                // TODO: Add proper sorting for Bids (highest price first)
            }
            Side::Sell => {
                self.asks.push_back(order);
                // TODO: Add proper sorting for Asks (lowest price first)
            }
        }

        // Log using tracing instead of println!
        tracing::debug!(order_id = order_id, book = ?self, "Added order. Book state before match attempt");

        self.try_match(); // Call matching logic

        tracing::debug!(book = ?self, "Book state after match attempt");
    }
    

  // try and match
  fn try_match(&mut self) {
    tracing::debug!("Attempting match...");
    // Loop while both sides have orders
    while !self.bids.is_empty() && !self.asks.is_empty() {

        // Check price without mutable borrow first (more efficient if no match)
        let can_match = {
            let best_bid = self.bids.front().unwrap(); // Safe due to loop condition
            let best_ask = self.asks.front().unwrap(); // Safe due to loop condition
            tracing::debug!(bid_price = best_bid.price, bid_qty = best_bid.quantity, ask_price = best_ask.price, ask_qty = best_ask.quantity, "Checking best bid/ask");
            best_bid.price >= best_ask.price
        };

        if can_match {
            // Prices cross, get mutable references and perform match
            let best_bid_mut = self.bids.front_mut().unwrap();
            let best_ask_mut = self.asks.front_mut().unwrap();

            tracing::info!(bid_id = best_bid_mut.id, ask_id = best_ask_mut.id, price = best_ask_mut.price, "MATCH FOUND!");

            let matched_quantity = std::cmp::min(best_bid_mut.quantity, best_ask_mut.quantity);
            tracing::info!(quantity = matched_quantity, "Matched Quantity");

            best_bid_mut.quantity -= matched_quantity;
            best_ask_mut.quantity -= matched_quantity;

            let bid_id = best_bid_mut.id;
            let ask_id = best_ask_mut.id;
            let bid_fully_filled = best_bid_mut.quantity == 0;
            let ask_fully_filled = best_ask_mut.quantity == 0;

            // Remove fully filled orders
            if bid_fully_filled {
                self.bids.pop_front();
                tracing::info!(order_id = bid_id, "Bid order fully filled and removed.");
            }
            if ask_fully_filled {
                self.asks.pop_front();
                tracing::info!(order_id = ask_id, "Ask order fully filled and removed.");
            }
             // Loop continues automatically to check the new front orders (or the partially filled ones)

        } else {
            // Prices don't cross, no more matches possible in this cycle
            tracing::debug!("No match possible (bid price < ask price)");
            break; // Exit the while loop
        }
    } // End of while loop

    tracing::debug!("Finished matching cycle.");
}


    // --- FIX 2: ADD OrderBook Modify and Cancel Methods ---
    pub fn modify_order(&mut self, id: OrderId, new_quantity: u64) -> Option<Order> {
        if new_quantity == 0 {
            tracing::warn!(order_id = id, "Modification requested with quantity 0. Redirecting to cancel order.");
            return self.cancel_order(id);
        }
    
        // Search bids
        if let Some(order) = self.bids.iter_mut().find(|o| o.id == id) {
            tracing::info!(order_id = id, old_qty = order.quantity, new_qty = new_quantity, "Modifying bid order quantity");
            order.quantity = new_quantity;
            return Some(order.clone());
        }
    
        // --- ADD THIS BLOCK BACK ---
        // Search asks if not found in bids
        if let Some(order) = self.asks.iter_mut().find(|o| o.id == id) {
            tracing::info!(order_id = id, old_qty = order.quantity, new_qty = new_quantity, "Modifying ask order quantity");
            order.quantity = new_quantity;
            return Some(order.clone());
        }
        // --- END OF ADDED BLOCK ---
    
        // Order not found
        tracing::warn!(order_id = id, "Order not found for modification"); // Corrected warning message
        None
    }

    pub fn cancel_order(&mut self, id: OrderId) -> Option<Order> {
        tracing::info!(order_id = id, "Attempting to cancel order");
    
        // --- Performance Note (Step 7 Refinement) ---
        // Finding the order by iterating (`position()`) and removing from VecDeque (`remove()`)
        // are both potentially O(n) operations (linear time complexity relative to the
        // number of orders on that side of the book).
        // For a truly low-latency system with many orders, a different structure is needed,
        // typically involving a HashMap<OrderId, ...> for fast lookups combined with
        // sorted structures holding references/IDs (e.g., BTreeMap, custom heap).
        // We keep VecDeque here for simplicity in this project.
        // --- End Performance Note ---
    
    
        // Find the index in bids
        if let Some(index) = self.bids.iter().position(|o| o.id == id) {
            // Remove the order by index and return it
            let removed = self.bids.remove(index); // O(n) removal
            tracing::info!(order_id = id, "Cancelled bid order");
            return removed;
        }
    
        // Find the index in asks if not in bids
        if let Some(index) = self.asks.iter().position(|o| o.id == id) {
            // Remove the order by index and return it
            let removed = self.asks.remove(index); // O(n) removal
            tracing::info!(order_id = id, "Cancelled ask order");
            return removed;
        }
    
        // Order not found
        tracing::warn!(order_id = id, "Order not found for cancellation");
        None
    }
    // --- End OrderBook impl ---
}


// --- API Payload Structs ---
#[derive(Deserialize, Debug)] // Added Debug for logging
struct CreateOrderPayload {
    side: Side, // Uses the main Side enum
    price: u64,
    quantity: u64,
}

#[derive(Deserialize, Debug)] // Added Debug for logging
struct ModifyOrderPayload {
    quantity: u64,
}

// --- FIX 1: REMOVED Redefinitions of Order and Side ---


// --- Shared Application State ---
struct AppState {
    order_book: Mutex<OrderBook>,
    next_order_id: AtomicU64,
}

// --- Main Application Entry Point ---
#[tokio::main]
async fn main() {
    // --- Basic Logging Setup ---
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "low_latency_oms=debug,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    tracing::info!("Logger initialized"); // Use info! macro
    // --- End Logging Setup ---

    // --- Create Shared AppState ---
    let shared_state = Arc::new(AppState {
        order_book: Mutex::new(OrderBook::new()),
        next_order_id: AtomicU64::new(1),
    });
    tracing::info!("Shared AppState created."); // Use info! macro
    // --- End Shared State ---

    // --- Define API Routes ---
    let app = Router::new()
        .route("/", get(root_handler))
        // Add routes for orders
        .route("/orders", post(create_order_handler)) // POST to create
        .route("/orders/:id", put(modify_order_handler)) // PUT to modify by ID
        .route("/orders/:id", delete(cancel_order_handler)) // DELETE to cancel by ID
        // Pass the shared state to handlers
        .with_state(shared_state);

    tracing::info!("API routes defined."); // Use info! macro
    // --- End API Routes ---

    // --- Run the Server ---
    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    tracing::info!("Starting server on {}", addr); // Use info! macro
    let listener = TcpListener::bind(addr).await.unwrap();
    tracing::info!("Server listening on {}", addr); // Use info! macro
    axum::serve(listener, app).await.unwrap();
    // --- End Run Server ---
}


// --- Basic Root Handler ---
async fn root_handler() -> &'static str {
    tracing::info!("Root handler called"); // Use info! macro
    "Hello from Low Latency OMS!" // Corrected response message typo
}


// --- FIX 3: ADD API Handlers ---

// Handler for POST /orders
async fn create_order_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<CreateOrderPayload>,
) -> Result<(StatusCode, Json<Order>), StatusCode> {
    // Log the received payload using Debug trait derived earlier
    tracing::info!(payload = ?payload, "Received create order request");

    // Get the next unique order ID atomically
    let order_id = state.next_order_id.fetch_add(1, Ordering::Relaxed);

    // Create the Order struct
    let new_order = Order::new(
        order_id,
        payload.side,
        payload.price,
        payload.quantity,
    );

    // Clone the order *before* moving it into the order book for the response
    let order_to_return = new_order.clone();

    // Lock the order book mutex to add the order
    { // Scope for the mutex guard
        let mut book_guard = state.order_book.lock().expect("Mutex lock failed"); // Use expect for clearer panic msg
        tracing::debug!(order_id = order_id, "Acquired lock for adding order");
        book_guard.add_order(new_order); // This also triggers try_match
    } // Mutex guard is dropped, lock released automatically here
    tracing::debug!(order_id = order_id, "Released lock after adding order");

    // Return 201 Created status and the created order as JSON
    Ok((StatusCode::CREATED, Json(order_to_return)))
}

// Handler for PUT /orders/{id}
async fn modify_order_handler(
    State(state): State<Arc<AppState>>,
    Path(order_id): Path<OrderId>,
    Json(payload): Json<ModifyOrderPayload>,
) -> Result<Json<Order>, StatusCode> {
    tracing::info!(order_id = order_id, payload = ?payload, "Received modify order request");

    // Lock the order book mutex to modify
    let modified_order_option = { // Scope for the mutex guard
        let mut book_guard = state.order_book.lock().expect("Mutex lock failed");
        tracing::debug!(order_id = order_id, "Acquired lock for modifying order");
        // Call the modify_order method
        book_guard.modify_order(order_id, payload.quantity)
    }; // Mutex guard is dropped, lock released

    tracing::debug!(order_id = order_id, "Released lock after attempting modify");

    // Check if the modification was successful
    match modified_order_option {
        Some(order) => Ok(Json(order)), // Return 200 OK with the modified order
        None => Err(StatusCode::NOT_FOUND), // Return 404 Not Found if order didn't exist
    }
}

// Handler for DELETE /orders/{id}
async fn cancel_order_handler(
    State(state): State<Arc<AppState>>,
    Path(order_id): Path<OrderId>,
) -> Result<Json<Order>, StatusCode> {
    tracing::info!(order_id = order_id, "Received cancel order request");

    // Lock the order book mutex to cancel
    let cancelled_order_option = { // Scope for the mutex guard
        let mut book_guard = state.order_book.lock().expect("Mutex lock failed");
        tracing::debug!(order_id = order_id, "Acquired lock for cancelling order");
        // Call the cancel_order method
        book_guard.cancel_order(order_id)
    }; // Mutex guard is dropped, lock released

    tracing::debug!(order_id = order_id, "Released lock after attempting cancel");

    // Check if cancellation was successful
    match cancelled_order_option {
        Some(order) => Ok(Json(order)), // Return 200 OK with the cancelled order
        None => Err(StatusCode::NOT_FOUND), // Return 404 Not Found if order didn't exist
    }
}
// --- End API Handlers ---

#[cfg(test)] // Only compile this module when running tests
mod tests {
    use super::*; // Import items from outer module

    #[test]
    fn test_order_creation() {
        let order = Order::new(1, Side::Buy, 100, 50);
        assert_eq!(order.id, 1);
        assert_eq!(order.side, Side::Buy);
        assert_eq!(order.price, 100);
        assert_eq!(order.quantity, 50);
        assert!(order.timestamp > 0);
    }

    #[test]
    fn test_add_order_to_book() {
        let mut book = OrderBook::new();
        let buy_order = Order::new(1, Side::Buy, 100, 10);
        let sell_order = Order::new(2, Side::Sell, 105, 20);

        book.add_order(buy_order.clone());
        book.add_order(sell_order.clone());

        assert_eq!(book.bids.len(), 1);
        assert_eq!(book.asks.len(), 1);
        assert_eq!(book.bids.front().unwrap().id, buy_order.id);
        assert_eq!(book.asks.front().unwrap().id, sell_order.id);
    }

    #[test]
    fn test_simple_match_full() {
        let mut book = OrderBook::new();
        let buy_order = Order::new(1, Side::Buy, 100, 10);
        let sell_order = Order::new(2, Side::Sell, 100, 10);

        book.add_order(buy_order);
        book.add_order(sell_order); // Match occurs here

        // Assert: Both orders should be gone
        assert!(book.bids.is_empty(), "Bids should be empty after full match");
        assert!(book.asks.is_empty(), "Asks should be empty after full match");
        // Removed misplaced #[test] and incorrect assertions/nested function from here
    }

    #[test]
    fn test_simple_match_partial_buy_fills() {
        let mut book = OrderBook::new();
        let buy_order = Order::new(1, Side::Buy, 100, 5); // Buy 5
        let sell_order = Order::new(2, Side::Sell, 100, 10); // Sell 10

        book.add_order(buy_order);
        book.add_order(sell_order); // Match occurs here

        assert!(book.bids.is_empty(), "Bids should be empty after partial match (buy filled)");
        assert_eq!(book.asks.len(), 1, "Asks should have 1 remaining order");
        assert_eq!(book.asks.front().unwrap().id, 2);
        assert_eq!(book.asks.front().unwrap().quantity, 5, "Remaining sell quantity should be 5");
    }


    #[test]
    fn test_simple_match_partial_sell_fills() {
        let mut book = OrderBook::new();
        let buy_order = Order::new(1, Side::Buy, 100, 10); // Buy 10
        let sell_order = Order::new(2, Side::Sell, 100, 5); // Sell 5

        book.add_order(buy_order);
        book.add_order(sell_order); // Match occurs here

        assert!(book.asks.is_empty(), "Asks should be empty after partial match (sell filled)");
        assert_eq!(book.bids.len(), 1, "Bids should have 1 remaining order");
        assert_eq!(book.bids.front().unwrap().id, 1);
        assert_eq!(book.bids.front().unwrap().quantity, 5, "Remaining buy quantity should be 5");
    }

    // Correctly placed test_no_match_price_gap
    #[test]
    fn test_no_match_price_gap() {
        let mut book = OrderBook::new();
        // Fix: Add underscore to unused variables
        let _buy_order = Order::new(1, Side::Buy, 100, 10);
        let _sell_order = Order::new(2, Side::Sell, 105, 10);

        book.add_order(_buy_order.clone());
        book.add_order(_sell_order.clone());

        // Correct Assertions: No match occurred, both orders remain
        assert_eq!(book.bids.len(), 1, "Bids should still contain the buy order");
        assert_eq!(book.asks.len(), 1, "Asks should still contain the sell order");
        assert_eq!(book.bids.front().unwrap().id, 1); // Check ID
        assert_eq!(book.asks.front().unwrap().id, 2); // Check ID
    }

    #[test]
    fn test_match_with_better_price() {
        let mut book = OrderBook::new();
        let buy_order = Order::new(1, Side::Buy, 105, 10); // Buy higher
        let sell_order = Order::new(2, Side::Sell, 100, 10); // Sell lower

        book.add_order(buy_order);
        book.add_order(sell_order); // Match occurs here

        assert!(book.bids.is_empty(), "Bids should be empty after match");
        assert!(book.asks.is_empty(), "Asks should be empty after match"); // Fixed typo: empthy -> empty
    }

    #[test]
    fn test_multiple_matches_from_one_order() {
        let mut book = OrderBook::new();
        // Setup: Sell 5@100 (1), Sell 15@101 (2). Then Buy 15@101 (3).
        let sell_order1 = Order::new(1, Side::Sell, 100, 5);
        let sell_order2 = Order::new(2, Side::Sell, 101, 15); // Original quantity was 15
        let buy_order = Order::new(3, Side::Buy, 101, 15);

        book.add_order(sell_order1); // Add Sell 1
        book.add_order(sell_order2); // Add Sell 2
        book.add_order(buy_order);   // Add Buy -> triggers matches

        // Expected Outcome:
        // 1. Buy 3 (15@101) vs Sell 1 (5@100) -> Match 5. Buy 3 is now 10@101. Sell 1 gone.
        // 2. Buy 3 (10@101) vs Sell 2 (15@101) -> Match 10. Buy 3 is now 0@101. Sell 2 is now 5@101. Buy 3 gone.
        // Final: Bids empty. Asks has Order 2 with Qty 5.

        // Corrected Assertions:
        assert!(book.bids.is_empty(), "Bids should be empty after buy order is fully matched");
        assert_eq!(book.asks.len(), 1, "Asks should have 1 remaining order");
        assert_eq!(book.asks.front().unwrap().id, 2, "Remaining ask should be order 2");
        assert_eq!(book.asks.front().unwrap().quantity, 5, "Remaining quantity for ask order 2 should be 5");
    }

    #[test]
    fn test_modify_order_quantity_bid(){
        let mut book = OrderBook::new();
        let order1 = Order::new(1, Side::Buy, 100, 10);
        book.add_order(order1);

        let result = book.modify_order(1, 5);

        // Assert: Modification successfulm quantity updated
        assert!(result.is_some(), "Modification shoul return the modified");
        assert_eq!(result.as_ref().unwrap().id, 1);
        assert_eq!(result.as_ref().unwrap().quantity, 5);
        assert_eq!(book.bids.len(), 1);
        assert_eq!(book.bids.front().unwrap().quantity, 5, "Quantity in book should be updater");
    }

    #[test]
    fn test_modify_order_quantity_ask(){
        let mut book = OrderBook::new();
        let order1 = Order::new(1, Side::Sell, 105, 20);
        book.add_order(order1);

        // Modify quantity
        let result = book.modify_order(1, 15);

        // Assert: Modification successful, quantity update
        assert!(result.is_some());
        assert_eq!(result.as_ref().unwrap().id, 1);
        assert_eq!(book.asks.len(), 1);
        assert_eq!(book.asks.front().unwrap().quantity, 15);
    }

    #[test]
    fn test_modify_order_not_found(){
        let mut book = OrderBook::new();
        let order1 = Order::new(1, Side::Buy, 100, 10);
        book.add_order(order1);

        // Try to modify a non-existent ID
        let result = book.modify_order(999, 5);

        assert!(result.is_none(), "Modifying non-existent order should return None");
        assert_eq!(book.bids.len(), 1);
        assert_eq!(book.bids.front().unwrap().id, 1);
    }

    #[test]
    fn test_modify_order_zero_quantity_cancels(){
        let mut book = OrderBook::new();
        let order1 = Order::new(1, Side::Buy, 100, 10);
        book.add_order(order1);

        let result = book.modify_order(1, 0);


        assert!(result.is_some(), "Modify with qty 0 should return the cancelled order via cancel_order");
        assert_eq!(result.unwrap().id, 1 , "Returned order should have the correct ID");
        assert!(book.bids.is_empty(), "Book should be empty after modify to zero quantity");
    }

    #[test]
    fn test_cancel_order_bid() {
        let mut book = OrderBook::new();
        let order1 = Order::new(1, Side::Buy, 100, 10);
        let order2 = Order::new(2, Side::Buy, 99, 5);
        book.add_order(order1.clone());
        book.add_order(order2.clone());

        assert_eq!(book.bids.len(), 2);

        // Cancel order 1
        let result = book.cancel_order(1);

        // Assert: Cancellation successful, order 1 removed, order 2 remains
        assert!(result.is_some(), "Cancellation should return the cancelled order");
        assert_eq!(result.unwrap().id, 1); // Check it's the correct order
        assert_eq!(book.bids.len(), 1, "Only one bid should remain");
        assert_eq!(book.bids.front().unwrap().id, 2, "Remaining bid should be order 2");
    }

    #[test]
    fn test_cancel_order_ask() {
        let mut book = OrderBook::new();
        let order1 = Order::new(1, Side::Sell, 105, 10);
        let order2 = Order::new(2, Side::Sell, 110, 5);
        book.add_order(order1.clone());
        book.add_order(order2.clone());

        assert_eq!(book.asks.len(), 2);

        // Cancel order 1
        let result = book.cancel_order(1);

        // Assert: Cancellation successful, order 1 removed, order 2 remains
        assert!(result.is_some());
        assert_eq!(result.unwrap().id, 1);
        assert_eq!(book.asks.len(), 1);
        assert_eq!(book.asks.front().unwrap().id, 2);
    }

    #[test]
    fn test_cancel_order_not_found() {
        let mut book = OrderBook::new();
        let order1 = Order::new(1, Side::Buy, 100, 10);
        book.add_order(order1.clone());

        // Try to cancel a non-existent ID
        let result = book.cancel_order(999);

        // Assert: Cancellation failed (None returned), book unchanged
        assert!(result.is_none(), "Cancelling non-existent order should return None");
        assert_eq!(book.bids.len(), 1); // Original order still there
        assert_eq!(book.bids.front().unwrap().id, 1);
    }


}
// --- End Unit Tests ---


