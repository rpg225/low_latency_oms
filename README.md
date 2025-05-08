# Low-Latency Order Management System (OMS) in Rust

A core Order Management System (OMS) built in Rust, designed as a learning project to explore low-latency design patterns, concurrency, and backend development in Rust. This system handles order creation, modification, cancellation, and matching with an in-memory order book and SQLite persistence.

## Features

*   **REST API:** For order operations (creation, modification, cancellation) built with Axum.
*   **In-Memory Order Book:** Manages buy and sell orders using `VecDeque`.
*   **FIFO Matching Engine:** Matches orders based on first-in, first-out logic for orders at the same price level (current implementation prioritizes orders as they arrive on each side).
*   **Concurrent Processing Foundation:** Utilizes Tokio for asynchronous request handling and `Arc<Mutex<>>` for thread-safe access to the order book and database connection.
*   **Order Persistence:** Uses SQLite via `rusqlite` to save and load order states across server restarts. All order actions (create, modify, cancel, match) are persisted.
*   **Unit Tests:** Includes tests for core order book logic (add, match, modify, cancel).
*   **Logging:** Uses the `tracing` library for application logging.

## Technologies Used

*   **Rust:** Core programming language.
*   **Tokio:** Asynchronous runtime for concurrent operations.
*   **Axum:** Web framework for building the REST API.
*   **Serde:** For JSON serialization and deserialization.
*   **Rusqlite:** SQLite wrapper for database interaction (with `bundled` feature).
*   **Tracing:** For application logging and diagnostics.

## Setup and Installation

1.  **Prerequisites:**
    *   Install Rust: [https://www.rust-lang.org/tools/install](https://www.rust-lang.org/tools/install)

2.  **Clone the repository:**
    ```bash
    git clone https://github.com/rpg225/low_latency_oms
    cd low-latency-oms
    ```


3.  **Build the project:**
    ```bash
    cargo build
    ```
    For a release (optimized) build:
    ```bash
    cargo build --release
    ```

## Running the Server

To start the OMS server:

```bash
cargo run