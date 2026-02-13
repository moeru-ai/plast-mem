#[allow(clippy::needless_for_each)]
pub mod api;
pub mod utils;

mod server;
pub use server::server;

// Re-export for OpenAPI documentation
pub use api::ApiDoc;
