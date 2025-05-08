use axum::{
    routing::{get, post, put, delete},
    Router,
    response::Json,
    extract::{State, Path},
    http::StatusCode,
};
use std::net::SocketAddr;
use tokio::net::TcpListener;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU64, Ordering};
use std::collections::VecDeque;
use std::time::{SystemTime, UNIX_EPOCH};

// --- DB & Async Task Imports ---
use rusqlite::{Connection, Result as SqlResult, params};
use tokio::task;

// Tracing / Logging
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use tracing;

// --- Standard Error and Formatting imports ---
use std::error::Error as StdError; // Alias for clarity
use std::fmt;

// --- Custom Error for DB Conversion ---
#[derive(Debug)]
struct ConversionError(String); // Our custom error struct holding a String

impl fmt::Display for ConversionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0) // Display the inner string
    }
}

impl StdError for ConversionError {} // Implement the Error trait

// --- Core Data Structures ---

// Unique ID for each order
type OrderId = u64;

// Represents Buy or Sell
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Side {
    Buy,
    Sell,
}

// Represents the state of an order
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderStatus {
    Open,
    PartiallyFilled,
    Filled,
    Cancelled,
}

// Our main Order structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Order {
    id: OrderId,
    side: Side,
    price: u64,
    quantity: u64,
    timestamp: u128,
    status: OrderStatus,
}

impl Order {
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
            status: OrderStatus::Open,
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

    pub fn add_order(&mut self, order: Order, db_conn: Arc<Mutex<Connection>>) {
        let order_id = order.id;
        let side = order.side.clone();

        match side {
            Side::Buy => self.bids.push_back(order),
            Side::Sell => self.asks.push_back(order),
        }
        tracing::debug!(order_id = order_id, book = ?self, "Added order. Book state before match attempt");
        self.try_match(db_conn);
        tracing::debug!(book = ?self, "Book state after match attempt");
    }

    fn try_match(&mut self, db_conn: Arc<Mutex<Connection>>) {
        tracing::debug!("Attempting match...");
        while !self.bids.is_empty() && !self.asks.is_empty() {
            let can_match = {
                let best_bid = self.bids.front().unwrap();
                let best_ask = self.asks.front().unwrap();
                tracing::debug!(bid_price = best_bid.price, bid_qty = best_bid.quantity, ask_price = best_ask.price, ask_qty = best_ask.quantity, "Checking best bid/ask");
                best_bid.price >= best_ask.price
            };

            if can_match {
                let best_bid_mut = self.bids.front_mut().unwrap();
                let best_ask_mut = self.asks.front_mut().unwrap();

                let bid_id_for_db = best_bid_mut.id;
                let ask_id_for_db = best_ask_mut.id;

                tracing::info!(bid_id = bid_id_for_db, ask_id = ask_id_for_db, price = best_ask_mut.price, "MATCH FOUND!");
                let matched_quantity = std::cmp::min(best_bid_mut.quantity, best_ask_mut.quantity);
                tracing::info!(quantity = matched_quantity, "Matched Quantity");

                best_bid_mut.quantity -= matched_quantity;
                best_ask_mut.quantity -= matched_quantity;

                best_bid_mut.status = if best_bid_mut.quantity == 0 { OrderStatus::Filled } else { OrderStatus::PartiallyFilled };
                best_ask_mut.status = if best_ask_mut.quantity == 0 { OrderStatus::Filled } else { OrderStatus::PartiallyFilled };

                let bid_status_db = format!("{:?}", best_bid_mut.status);
                let ask_status_db = format!("{:?}", best_ask_mut.status);
                let bid_remaining_qty_db = best_bid_mut.quantity;
                let ask_remaining_qty_db = best_ask_mut.quantity;

                let db_conn_clone: Arc<Mutex<Connection>> = Arc::clone(&db_conn);
                task::spawn_blocking(move || {
                    let mut conn_guard = db_conn_clone.lock().expect("Mutex lock failed for DB in try_match");
                    tracing::debug!(bid_id = bid_id_for_db, ask_id = ask_id_for_db, "Acquired DB lock for UPDATE (match)");
                    let tx = conn_guard.transaction().expect("Failed to start DB transaction in try_match");
                    tx.execute(
                        "UPDATE orders SET remaining_quantity = ?1, status = ?2 WHERE id = ?3",
                        params![bid_remaining_qty_db, bid_status_db, bid_id_for_db],
                    ).expect("DB error updating bid in match");
                    tx.execute(
                        "UPDATE orders SET remaining_quantity = ?1, status = ?2 WHERE id = ?3",
                        params![ask_remaining_qty_db, ask_status_db, ask_id_for_db],
                    ).expect("DB error updating ask in match");
                    tx.commit().expect("Failed to commit DB transaction in try_match");
                    tracing::debug!(bid_id = bid_id_for_db, ask_id = ask_id_for_db, "Released DB lock after UPDATE (match)");
                });

                if best_bid_mut.quantity == 0 {
                    self.bids.pop_front();
                    tracing::info!(order_id = bid_id_for_db, "Bid order fully filled and removed from memory.");
                }
                if best_ask_mut.quantity == 0 {
                    self.asks.pop_front();
                    tracing::info!(order_id = ask_id_for_db, "Ask order fully filled and removed from memory.");
                }
            } else {
                tracing::debug!("No match possible (bid price < ask price)");
                break;
            }
        }
        tracing::debug!("Finished matching cycle.");
    }

    pub fn modify_order(&mut self, id: OrderId, new_quantity: u64) -> Option<Order> {
        if new_quantity == 0 {
            tracing::warn!(order_id = id, "Modification requested with quantity 0. Redirecting to cancel order.");
            return self.cancel_order(id);
        }
        if let Some(order) = self.bids.iter_mut().find(|o| o.id == id) {
            tracing::info!(order_id = id, old_qty = order.quantity, new_qty = new_quantity, "Modifying bid order quantity");
            order.quantity = new_quantity;
            // If order was filled, and now modified, it should become Open or PartiallyFilled
            // For simplicity, let's set it to Open. A more complex logic might check original quantity.
            if order.status == OrderStatus::Filled {
                order.status = OrderStatus::Open;
            } else if order.status != OrderStatus::PartiallyFilled { // If not already partially filled, it's open
                 order.status = OrderStatus::Open;
            }
            return Some(order.clone());
        }
        if let Some(order) = self.asks.iter_mut().find(|o| o.id == id) {
            tracing::info!(order_id = id, old_qty = order.quantity, new_qty = new_quantity, "Modifying ask order quantity");
            order.quantity = new_quantity;
            if order.status == OrderStatus::Filled {
                order.status = OrderStatus::Open;
            } else if order.status != OrderStatus::PartiallyFilled {
                 order.status = OrderStatus::Open;
            }
            return Some(order.clone());
        }
        tracing::warn!(order_id = id, "Order not found for modification");
        None
    }

    pub fn cancel_order(&mut self, id: OrderId) -> Option<Order> {
        tracing::info!(order_id = id, "Attempting to cancel order");
        if let Some(index) = self.bids.iter().position(|o| o.id == id) {
            if let Some(mut order) = self.bids.remove(index) {
                order.status = OrderStatus::Cancelled;
                tracing::info!(order_id = id, "Cancelled bid order from memory.");
                return Some(order);
            }
        }
        if let Some(index) = self.asks.iter().position(|o| o.id == id) {
            if let Some(mut order) = self.asks.remove(index) {
                order.status = OrderStatus::Cancelled;
                tracing::info!(order_id = id, "Cancelled ask order from memory.");
                return Some(order);
            }
        }
        tracing::warn!(order_id = id, "Order not found for cancellation in memory.");
        None
    }
}

// --- API Payload Structs ---
#[derive(Deserialize, Debug)]
struct CreateOrderPayload {
    side: Side,
    price: u64,
    quantity: u64,
}

#[derive(Deserialize, Debug)]
struct ModifyOrderPayload {
    quantity: u64,
}

// --- Shared Application State ---
struct AppState {
    order_book: Mutex<OrderBook>,
    next_order_id: AtomicU64,
    db_conn: Arc<Mutex<Connection>>,
}

// --- Database Setup ---
const DB_PATH: &str = "oms_data.db";

fn init_db() -> SqlResult<Connection> {
    tracing::info!(db_path = DB_PATH, "Initializing database...");
    let conn = Connection::open(DB_PATH)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS orders (
            id INTEGER PRIMARY KEY,
            side TEXT NOT NULL,
            price INTEGER NOT NULL,
            original_quantity INTEGER NOT NULL,
            remaining_quantity INTEGER NOT NULL,
            status TEXT NOT NULL,
            timestamp TEXT NOT NULL -- CHANGED TO TEXT
        )",
        [],
    )?;
    tracing::info!("Database table 'orders' initialized.");
    Ok(conn)
}

fn load_open_orders(conn: &Connection) -> SqlResult<Vec<Order>> {
    tracing::info!("Loading open orders from database...");
    let mut stmt = conn.prepare("SELECT id, side, price, remaining_quantity, timestamp, status FROM orders WHERE status = 'Open' OR status = 'PartiallyFilled'")?;
    let order_iter = stmt.query_map([], |row| {
        let side_str: String = row.get(1)?;
        let side = match side_str.as_str() {
            "Buy" => Side::Buy,
            "Sell" => Side::Sell,
            other => return Err(rusqlite::Error::FromSqlConversionFailure(
                1,
                rusqlite::types::Type::Text,
                Box::new(ConversionError(format!("Invalid side string: {}", other))) // USE ConversionError
            )),
        };
        let status_str: String = row.get(5)?;
        let status = match status_str.as_str() {
            "Open" => OrderStatus::Open,
            "PartiallyFilled" => OrderStatus::PartiallyFilled,
            other => return Err(rusqlite::Error::FromSqlConversionFailure(
                5,
                rusqlite::types::Type::Text,
                Box::new(ConversionError(format!("Invalid status string: {}", other))) // USE ConversionError
            )),
        };
        Ok(Order {
            id: row.get(0)?,
            side,
            price: row.get(2)?,
            quantity: row.get(3)?,
            timestamp: {
                let ts_str: String = row.get(4)?;
                ts_str.parse::<u128>().map_err(|e| rusqlite::Error::FromSqlConversionFailure(
                    4,
                    rusqlite::types::Type::Text,
                    Box::new(ConversionError(format!("Failed to parse u128 from timestamp string: {}", e))) // USE ConversionError
                ))?
            },
            status,
        })
    })?;
    let mut orders = Vec::new();
    for order_result in order_iter {
        orders.push(order_result?);
    }
    tracing::info!("Loaded {} open/partially filled order(s).", orders.len());
    Ok(orders)
}

// --- Main Application Entry Point ---
#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "low_latency_oms=debug,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();
    tracing::info!("Logger initialized");

    let connection = init_db().expect("Failed to initialize database");
    let open_orders = load_open_orders(&connection).expect("Failed to load open orders");

    let mut initial_book = OrderBook::new();
    let mut max_id = 0;
    for order in open_orders {
        if order.id > max_id { max_id = order.id; }
        match order.side {
            Side::Buy => initial_book.bids.push_back(order),
            Side::Sell => initial_book.asks.push_back(order),
        }
    }
    tracing::info!("Order book populated with loaded orders.");

    let shared_state = Arc::new(AppState {
        order_book: Mutex::new(initial_book),
        next_order_id: AtomicU64::new(max_id + 1),
        db_conn: Arc::new(Mutex::new(connection)),
    });
    tracing::info!(next_order_id = max_id + 1, "Shared AppState created.");

    let app = Router::new()
        .route("/", get(root_handler))
        .route("/orders", post(create_order_handler))
        .route("/orders/:id", put(modify_order_handler))
        .route("/orders/:id", delete(cancel_order_handler))
        .with_state(shared_state);
    tracing::info!("API routes defined.");

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    tracing::info!("Starting server on {}", addr);
    let listener = TcpListener::bind(addr).await.unwrap();
    tracing::info!("Server listening on {}", addr);
    axum::serve(listener, app).await.unwrap();
}

// --- Basic Root Handler ---
async fn root_handler() -> &'static str {
    tracing::info!("Root handler called");
    "Hello from Low Latency OMS!"
}

// --- API Handlers ---
async fn create_order_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<CreateOrderPayload>,
) -> Result<(StatusCode, Json<Order>), StatusCode> {
    tracing::info!(payload = ?payload, "Received create order request");

    let order_id = state.next_order_id.fetch_add(1, Ordering::Relaxed);
    let new_order_obj = Order::new(
        order_id,
        payload.side.clone(),
        payload.price,
        payload.quantity,
    );
    let order_to_return = new_order_obj.clone();
    let order_for_db = new_order_obj.clone();
    let order_for_book = new_order_obj;

    {
        let mut book_guard = state.order_book.lock().expect("Mutex lock failed for book");
        tracing::debug!(order_id = order_id, "Acquired book lock for adding order");
        book_guard.add_order(order_for_book, Arc::clone(&state.db_conn));
    }
    tracing::debug!(order_id = order_id, "Released book lock after adding order");

    let db_conn_clone: Arc<Mutex<Connection>> = Arc::clone(&state.db_conn);
    task::spawn_blocking(move || {
        let conn_guard = db_conn_clone.lock().expect("Mutex lock failed for DB insert");
        tracing::debug!(order_id = order_for_db.id, "Acquired DB lock for INSERT");
        conn_guard.execute(
            "INSERT INTO orders (id, side, price, original_quantity, remaining_quantity, status, timestamp) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                order_for_db.id,
                format!("{:?}", order_for_db.side),
                order_for_db.price,
                order_for_db.quantity,
                order_for_db.quantity,
                format!("{:?}", order_for_db.status),
                order_for_db.timestamp.to_string(), // STORE TIMESTAMP AS STRING
            ],
        )
    })
    .await
    .map_err(|e| {
        tracing::error!("Task join error for order insert: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?
    .map_err(|e| {
        tracing::error!("DB error inserting order {}: {}", order_id, e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    tracing::debug!(order_id = order_id, "DB INSERT successful");

    Ok((StatusCode::CREATED, Json(order_to_return)))
}

async fn modify_order_handler(
    State(state): State<Arc<AppState>>,
    Path(order_id): Path<OrderId>,
    Json(payload): Json<ModifyOrderPayload>,
) -> Result<Json<Order>, StatusCode> {
    tracing::info!(order_id = order_id, payload = ?payload, "Received modify order request");

    let modified_order_from_book = {
        let mut book_guard = state.order_book.lock().expect("Mutex lock failed for book modify");
        tracing::debug!(order_id = order_id, "Acquired book lock for modifying order");
        book_guard.modify_order(order_id, payload.quantity)
    };
    tracing::debug!(order_id = order_id, "Released book lock after attempting modify");

    let order_for_db = match modified_order_from_book {
        Some(order) => order,
        None => return Err(StatusCode::NOT_FOUND),
    };

    let db_conn_clone: Arc<Mutex<Connection>> = Arc::clone(&state.db_conn);
    let status_for_db = format!("{:?}", order_for_db.status);
    let quantity_for_db = order_for_db.quantity;
    let id_for_db = order_for_db.id;

    task::spawn_blocking(move || {
        let conn_guard = db_conn_clone.lock().expect("Mutex lock failed for DB update (modify)");
        tracing::debug!(order_id = id_for_db, "Acquired DB lock for UPDATE (modify)");
        conn_guard.execute(
            "UPDATE orders SET remaining_quantity = ?1, status = ?2 WHERE id = ?3",
            params![quantity_for_db, status_for_db, id_for_db],
        )
    })
    .await
    .map_err(|e| {
        tracing::error!("Task join error for order update (modify): {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?
    .map_err(|e| {
        tracing::error!("DB error updating order {} (modify): {}", order_id, e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    tracing::debug!(order_id = order_id, "DB UPDATE (modify) successful");

    Ok(Json(order_for_db))
}

async fn cancel_order_handler(
    State(state): State<Arc<AppState>>,
    Path(order_id): Path<OrderId>,
) -> Result<Json<Order>, StatusCode> {
    tracing::info!(order_id = order_id, "Received cancel order request");

    let cancelled_order_from_book = {
        let mut book_guard = state.order_book.lock().expect("Mutex lock failed for book cancel");
        tracing::debug!(order_id = order_id, "Acquired book lock for cancelling order");
        book_guard.cancel_order(order_id)
    };
    tracing::debug!(order_id = order_id, "Released book lock after attempting cancel");

    let order_for_db = match cancelled_order_from_book {
        Some(order) => order,
        None => return Err(StatusCode::NOT_FOUND),
    };

    let db_conn_clone: Arc<Mutex<Connection>> = Arc::clone(&state.db_conn);
    let status_for_db = format!("{:?}", order_for_db.status);
    let id_for_db = order_for_db.id;

    task::spawn_blocking(move || {
        let conn_guard = db_conn_clone.lock().expect("Mutex lock failed for DB update (cancel)");
        tracing::debug!(order_id = id_for_db, "Acquired DB lock for UPDATE (cancel)");
        conn_guard.execute(
            "UPDATE orders SET status = ?1, remaining_quantity = 0 WHERE id = ?2",
            params![status_for_db, id_for_db],
        )
    })
    .await
    .map_err(|e| {
        tracing::error!("Task join error for order update (cancel): {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?
    .map_err(|e| {
        tracing::error!("DB error updating order {} (cancel): {}", order_id, e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    tracing::debug!(order_id = order_id, "DB UPDATE (cancel) successful");

    Ok(Json(order_for_db))
}

// --- Unit Tests ---
#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_db_conn() -> Arc<Mutex<Connection>> {
        Arc::new(Mutex::new(Connection::open_in_memory().unwrap()))
    }

    #[test]
    fn test_order_creation() {
        let order = Order::new(1, Side::Buy, 100, 50);
        assert_eq!(order.id, 1);
        assert_eq!(order.status, OrderStatus::Open);
    }

    #[test]
    fn test_add_order_to_book() {
        let mut book = OrderBook::new();
        let db_conn = dummy_db_conn();
        let buy_order = Order::new(1, Side::Buy, 100, 10);
        book.add_order(buy_order.clone(), Arc::clone(&db_conn));
        assert_eq!(book.bids.len(), 1);
        assert_eq!(book.bids.front().unwrap().id, 1);
    }

    #[test]
    fn test_simple_match_full() {
        let mut book = OrderBook::new();
        let db_conn = dummy_db_conn();
        let buy_order = Order::new(1, Side::Buy, 100, 10);
        let sell_order = Order::new(2, Side::Sell, 100, 10);
        book.add_order(buy_order, Arc::clone(&db_conn));
        book.add_order(sell_order, Arc::clone(&db_conn));
        assert!(book.bids.is_empty());
        assert!(book.asks.is_empty());
    }

    #[test]
    fn test_simple_match_partial_buy_fills() {
        let mut book = OrderBook::new();
        let db_conn = dummy_db_conn();
        let buy_order = Order::new(1, Side::Buy, 100, 5);
        let sell_order = Order::new(2, Side::Sell, 100, 10);
        book.add_order(buy_order, Arc::clone(&db_conn));
        book.add_order(sell_order, Arc::clone(&db_conn));
        assert!(book.bids.is_empty());
        assert_eq!(book.asks.len(), 1);
        let ask_order = book.asks.front().unwrap();
        assert_eq!(ask_order.id, 2);
        assert_eq!(ask_order.quantity, 5);
        assert_eq!(ask_order.status, OrderStatus::PartiallyFilled);
    }

    #[test]
    fn test_simple_match_partial_sell_fills() {
        let mut book = OrderBook::new();
        let db_conn = dummy_db_conn();
        let buy_order = Order::new(1, Side::Buy, 100, 10);
        let sell_order = Order::new(2, Side::Sell, 100, 5);
        book.add_order(buy_order, Arc::clone(&db_conn));
        book.add_order(sell_order, Arc::clone(&db_conn));
        assert!(book.asks.is_empty());
        assert_eq!(book.bids.len(), 1);
        let bid_order = book.bids.front().unwrap();
        assert_eq!(bid_order.id, 1);
        assert_eq!(bid_order.quantity, 5);
        assert_eq!(bid_order.status, OrderStatus::PartiallyFilled);
    }

     #[test]
    fn test_no_match_price_gap() {
        let mut book = OrderBook::new();
        let db_conn = dummy_db_conn();
        let _buy_order = Order::new(1, Side::Buy, 100, 10);
        let _sell_order = Order::new(2, Side::Sell, 105, 10);

        book.add_order(_buy_order.clone(), Arc::clone(&db_conn));
        book.add_order(_sell_order.clone(), Arc::clone(&db_conn));

        assert_eq!(book.bids.len(), 1);
        assert_eq!(book.asks.len(), 1);
        assert_eq!(book.bids.front().unwrap().id, 1);
        assert_eq!(book.asks.front().unwrap().id, 2);
    }

    #[test]
    fn test_match_with_better_price() {
        let mut book = OrderBook::new();
        let db_conn = dummy_db_conn();
        let buy_order = Order::new(1, Side::Buy, 105, 10);
        let sell_order = Order::new(2, Side::Sell, 100, 10);

        book.add_order(buy_order, Arc::clone(&db_conn));
        book.add_order(sell_order, Arc::clone(&db_conn));

        assert!(book.bids.is_empty());
        assert!(book.asks.is_empty());
    }

    #[test]
    fn test_multiple_matches_from_one_order() {
        let mut book = OrderBook::new();
        let db_conn = dummy_db_conn();
        let sell_order1 = Order::new(1, Side::Sell, 100, 5);
        let sell_order2 = Order::new(2, Side::Sell, 101, 15);
        let buy_order = Order::new(3, Side::Buy, 101, 15);
        book.add_order(sell_order1, Arc::clone(&db_conn));
        book.add_order(sell_order2, Arc::clone(&db_conn));
        book.add_order(buy_order, Arc::clone(&db_conn));
        assert!(book.bids.is_empty());
        assert_eq!(book.asks.len(), 1);
        let ask_order = book.asks.front().unwrap();
        assert_eq!(ask_order.id, 2);
        assert_eq!(ask_order.quantity, 5);
        assert_eq!(ask_order.status, OrderStatus::PartiallyFilled);
    }

    #[test]
    fn test_modify_order_quantity_bid() {
        let mut book = OrderBook::new();
        let db_conn = dummy_db_conn();
        let order1 = Order::new(1, Side::Buy, 100, 10);
        book.add_order(order1, Arc::clone(&db_conn));
        let result = book.modify_order(1, 5);
        assert!(result.is_some());
        assert_eq!(result.as_ref().unwrap().quantity, 5);
        assert_eq!(book.bids.front().unwrap().quantity, 5);
    }

     #[test]
    fn test_modify_order_quantity_ask() {
        let mut book = OrderBook::new();
        let db_conn = dummy_db_conn();
        let order1 = Order::new(1, Side::Sell, 105, 20);
        book.add_order(order1, Arc::clone(&db_conn));

        let result = book.modify_order(1, 15);
        assert!(result.is_some());
        assert_eq!(result.as_ref().unwrap().quantity, 15);
        assert_eq!(book.asks.front().unwrap().quantity, 15);
    }

    #[test]
    fn test_modify_order_not_found() {
        let mut book = OrderBook::new();
        let db_conn = dummy_db_conn(); // Not strictly needed here but good practice
        let order1 = Order::new(1, Side::Buy, 100, 10);
        // Order is not added to book, but modify_order works on the book content
        // book.add_order(order1, Arc::clone(&db_conn)); // Let's test on an empty book

        let result = book.modify_order(order1.id, 5); // Use order1.id
        assert!(result.is_none()); // If order1 was not added, it shouldn't be found
    }


    #[test]
    fn test_modify_order_zero_quantity_cancels() {
        let mut book = OrderBook::new();
        let db_conn = dummy_db_conn();
        let order1 = Order::new(1, Side::Buy, 100, 10);
        book.add_order(order1, Arc::clone(&db_conn));
        let result = book.modify_order(1, 0);
        assert!(result.is_some());
        assert_eq!(result.as_ref().unwrap().status, OrderStatus::Cancelled);
        assert!(book.bids.is_empty());
    }

    #[test]
    fn test_cancel_order_bid() {
        let mut book = OrderBook::new();
        let db_conn = dummy_db_conn();
        let order1 = Order::new(1, Side::Buy, 100, 10);
        let order2 = Order::new(2, Side::Buy, 99, 5);
        book.add_order(order1.clone(), Arc::clone(&db_conn));
        book.add_order(order2.clone(), Arc::clone(&db_conn));
        let result = book.cancel_order(1);
        assert!(result.is_some());
        assert_eq!(result.as_ref().unwrap().status, OrderStatus::Cancelled);
        assert_eq!(book.bids.len(), 1);
        assert_eq!(book.bids.front().unwrap().id, 2);
    }

    #[test]
    fn test_cancel_order_ask() {
        let mut book = OrderBook::new();
        let db_conn = dummy_db_conn();
        let order1 = Order::new(1, Side::Sell, 105, 10);
        let order2 = Order::new(2, Side::Sell, 110, 5);
        book.add_order(order1.clone(), Arc::clone(&db_conn));
        book.add_order(order2.clone(), Arc::clone(&db_conn));

        let result = book.cancel_order(1);
        assert!(result.is_some());
        assert_eq!(result.as_ref().unwrap().status, OrderStatus::Cancelled);
        assert_eq!(book.asks.len(), 1);
        assert_eq!(book.asks.front().unwrap().id, 2);
    }

    #[test]
    fn test_cancel_order_not_found() {
        let mut book = OrderBook::new();
        // let db_conn = dummy_db_conn(); // Not needed if not adding orders
        // let order1 = Order::new(1, Side::Buy, 100, 10);
        // book.add_order(order1.clone(), Arc::clone(&db_conn));

        let result = book.cancel_order(999); // Try to cancel on an empty book
        assert!(result.is_none());
    }
}
// --- End Unit Tests ---