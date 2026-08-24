//! Wasm-compatible definitions for the KingbaseES MySQL-compatible connector.
//!
//! KingbaseES uses the PostgreSQL wire protocol, while its MySQL-compatible
//! mode uses MySQL SQL syntax. Native connection handling is isolated in the
//! `native` submodule; SQL rendering continues to use the MySQL visitor.

mod error;
#[cfg(feature = "kingbase-mysql-native")]
pub(crate) mod native;
mod url;

pub use error::KingbaseError;
#[cfg(feature = "kingbase-mysql-native")]
pub use native::KingbaseMysql;
pub use url::KingbaseMysqlUrl;

/// KingbaseES creates `test` as the default administrative database.
pub const DEFAULT_KINGBASE_MYSQL_DB: &str = "test";

/// KingbaseES uses `public` as the default schema in a database.
pub const DEFAULT_KINGBASE_MYSQL_SCHEMA: &str = "public";
