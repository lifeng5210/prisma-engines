//! Connection definitions for KingbaseES in Oracle-compatible mode.
//!
//! Kingbase Oracle mode uses the Kingbase PostgreSQL-wire driver, but keeps an
//! independent provider identity because its native types and SQL dialect are
//! not PostgreSQL or Kingbase MySQL semantics.

mod error;
#[cfg(feature = "kingbase-oracle-native")]
pub(crate) mod native;
mod url;

pub use error::KingbaseOracleError;
#[cfg(feature = "kingbase-oracle-native")]
pub use native::KingbaseOracle;
pub use url::KingbaseOracleUrl;

/// KingbaseES defaults to the `test` maintenance database.
pub const DEFAULT_KINGBASE_ORACLE_DB: &str = "test";

/// The tested Oracle-compatible installation resolves unqualified objects in
/// `public` unless the connection URL selects another schema.
pub const DEFAULT_KINGBASE_ORACLE_SCHEMA: &str = "public";

/// KingbaseES default TCP port. Compatibility mode does not change it.
pub const DEFAULT_KINGBASE_ORACLE_PORT: u16 = 54321;
