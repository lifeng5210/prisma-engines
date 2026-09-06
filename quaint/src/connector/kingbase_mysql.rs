//! Wasm-compatible definitions for the KingbaseES MySQL-compatible connector.
//!
//! KingbaseES uses the PostgreSQL wire protocol, while its MySQL-compatible
//! mode uses MySQL SQL syntax. Native connection handling is isolated in the
//! `native` submodule; SQL rendering continues to use the MySQL visitor.
//!
//! 金仓 MySQL 兼容模式通过 PostgreSQL 协议连接数据库，但 SQL 方言遵循 MySQL；
//! 因此连接与类型编解码由 `native` 子模块处理，SQL 渲染仍复用 MySQL visitor。

mod error;
#[cfg(feature = "kingbase-mysql-native")]
pub(crate) mod native;
mod url;

pub use error::KingbaseError;
#[cfg(feature = "kingbase-mysql-native")]
pub use native::KingbaseMysql;
pub use url::KingbaseMysqlUrl;

/// KingbaseES 默认创建 `test` 作为管理数据库。
pub const DEFAULT_KINGBASE_MYSQL_DB: &str = "test";

/// KingbaseES 在数据库中使用 `public` 作为默认 schema。
pub const DEFAULT_KINGBASE_MYSQL_SCHEMA: &str = "public";

/// KingbaseES 的默认 TCP 端口。
pub const DEFAULT_KINGBASE_MYSQL_PORT: u16 = 54321;
