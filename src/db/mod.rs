mod connect;
pub mod entities;
mod migration;

pub use connect::{DatabaseConfig, DbError, connect};
pub use migration::migrate;
