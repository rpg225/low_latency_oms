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

    fn try_match(&mut self) {
        tracing::debug!("Attempting match...");
        // Loop to handle multiple matches from one add
        while let (Some(best_bid), Some(best_ask)) = (self.bids.front_mut(), self.asks.front_mut()) {
            tracing::debug!(bid_price = best_bid.price, bid_qty = best_bid.quantity, ask_price = best_ask.price, ask_qty = best_ask.quantity, "Checking best bid/ask");

            // Match condition: highest bid price >= lowest ask price?
            if best_bid.price >= best_ask.price {
                tracing::info!(bid_id = best_bid.id, ask_id = best_ask.id, price = best_ask.price, "MATCH FOUND!"); // Match at the ask price (or bid, could be either if equal)

                let matched_quantity = std::cmp::min(best_bid.quantity, best_ask.quantity);
                tracing::info!(quantity = matched_quantity, "Matched Quantity");

                // Reduce quantities
                best_bid.quantity -= matched_quantity;
                best_ask.quantity -= matched_quantity;

                let bid_id = best_bid.id; // Store IDs before potential removal
                let ask_id = best_ask.id;
                let bid_fully_filled = best_bid.quantity == 0;
                let ask_fully_filled = best_ask.quantity == 0;

                // Remove filled orders (quantity is 0)
                if bid_fully_filled {
                    self.bids.pop_front(); // Remove the filled bid
                    tracing::info!(order_id = bid_id, "Bid order fully filled and removed.");
                }
                if ask_fully_filled {
                    self.asks.pop_front(); // Remove the filled ask
                    tracing::info!(order_id = ask_id, "Ask order fully filled and removed.");
                }

                // If either side was not fully filled, the loop must stop
                // because the remaining part of that order needs to stay.
                // If both were filled, we continue checking the *new* front orders.
                 if !bid_fully_filled || !ask_fully_filled {
                    tracing::debug!("Partial fill occurred, stopping match attempts for this cycle.");
                    break;
                 }
                 // Both were filled, continue loop to check next pair

            } else {
                // Prices don't cross, no match possible with these two orders
                tracing::debug!("No match possible (bid price < ask price)");
                break; // Exit the while loop, no more matches possible
            }
        } // End of while loop

        if self.bids.is_empty() || self.asks.is_empty() {
            tracing::debug!("No match possible (one side is empty)");
        }
    }


    // --- FIX 2: ADD OrderBook Modify and Cancel Methods ---
    pub fn modify_order(&mut self, id: OrderId, new_quantity: u64) -> Option<Order> {
        // Search bids
        if let Some(order) = self.bids.iter_mut().find(|o| o.id == id) {
             if new_quantity == 0 { // Treat quantity 0 as cancellation
                tracing::warn!(order_id = id, "Modification with quantity 0 requested, use DELETE instead. Cancelling.");
                return self.cancel_order(id); // Call cancel logic
             }
            tracing::info!(order_id = id, old_qty = order.quantity, new_qty = new_quantity, "Modifying bid order quantity");
            order.quantity = new_quantity;
            // TODO: In a real system, might need re-sorting or matching check here
            return Some(order.clone()); // Return the modified order
        }
        // Search asks if not found in bids
        if let Some(order) = self.asks.iter_mut().find(|o| o.id == id) {
             if new_quantity == 0 { // Treat quantity 0 as cancellation
                tracing::warn!(order_id = id, "Modification with quantity 0 requested, use DELETE instead. Cancelling.");
                return self.cancel_order(id); // Call cancel logic
             }
            tracing::info!(order_id = id, old_qty = order.quantity, new_qty = new_quantity, "Modifying ask order quantity");
            order.quantity = new_quantity;
            // TODO: In a real system, might need re-sorting or matching check here
            return Some(order.clone()); // Return the modified order
        }
        // Order not found
        tracing::warn!(order_id = id, "Order not found for modification");
        None
    }

    pub fn cancel_order(&mut self, id: OrderId) -> Option<Order> {
         tracing::info!(order_id = id, "Attempting to cancel order");
         // Find the index in bids
         if let Some(index) = self.bids.iter().position(|o| o.id == id) {
             // Remove the order by index and return it
             // VecDeque::remove is potentially O(n), consider alternatives for high performance
             let removed = self.bids.remove(index);
             tracing::info!(order_id = id, "Cancelled bid order");
             return removed;
         }
         // Find the index in asks if not in bids
         if let Some(index) = self.asks.iter().position(|o| o.id == id) {
             // Remove the order by index and return it
             let removed = self.asks.remove(index);
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