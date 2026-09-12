//! Kingbase Oracle-mode schema description.
//!
//! Kingbase Oracle mode exposes PostgreSQL system catalogs, but several SQL
//! types are Oracle-compatible aliases or domains. We can therefore reuse the
//! catalog traversal from the PostgreSQL describer, then translate its result
//! to the public Kingbase Oracle native types before handing it to PSL.

use crate::{
    ColumnArity, ColumnType, ColumnTypeFamily, DefaultKind, DefaultValue, DescriberResult, PrismaValue, SqlSchema,
    SqlSchemaDescriberBackend, postgres,
};
use psl::{
    builtin_connectors::{KingbaseOracleNumberArguments, KingbaseOracleType, KnownPostgresType, PostgresType},
    datamodel_connector::NativeTypeInstance,
};
use quaint::{
    Value,
    connector::{DescribedQuery, ExternalConnector, IsolationLevel, ResultSet, Transaction},
    prelude::Queryable,
};
use std::borrow::Cow;

/// A schema describer for KingbaseES running in Oracle compatibility mode.
pub struct SqlSchemaDescriber<'a> {
    conn: &'a dyn Queryable,
}

impl<'a> SqlSchemaDescriber<'a> {
    /// Create a describer using the PostgreSQL-compatible Kingbase catalog.
    pub fn new(conn: &'a dyn Queryable) -> Self {
        Self { conn }
    }
}

#[async_trait::async_trait]
impl SqlSchemaDescriberBackend for SqlSchemaDescriber<'_> {
    async fn describe(&self, schemas: &[&str]) -> DescriberResult<SqlSchema> {
        // PostgreSQL's catalog describer normally passes the namespace filter
        // as `text[]` and uses `= ANY ($1)`. Oracle mode intentionally does
        // not implement public SQL-array values, so wrap the catalog-only
        // calls and expand this narrow filter into escaped SQL string literals.
        // This does not enable scalar lists, array fields or array parameters
        // in the Prisma query surface.
        let catalog_connection = OracleCatalogConnection::new(self.conn);
        let mut schema = postgres::SqlSchemaDescriber::new(&catalog_connection, Default::default())
            .describe(schemas)
            .await?;

        for (_, column) in &mut schema.table_columns {
            rewrite_column_type(&mut column.tpe);
        }

        for (_, column) in &mut schema.view_columns {
            rewrite_column_type(&mut column.tpe);
        }

        // The shared PostgreSQL catalog parser sees Oracle NUMBER as a
        // generic numeric type before this module has canonicalized the
        // column family, so literal integer defaults initially become decimal
        // values and `nextval()` defaults become opaque dbgenerated values.
        // Reinterpret them using the final Oracle column family.
        for (column_id, default) in &mut schema.table_default_values {
            let column = &mut schema.table_columns[column_id.0 as usize].1;
            let tpe = &column.tpe;
            let family = &tpe.family;
            rewrite_number_default(default, family);
            rewrite_sequence_default(default, tpe);

            if matches!(default.kind(), DefaultKind::Sequence(_)) {
                column.auto_increment = true;
            }
        }

        Ok(schema)
    }

    async fn version(&self) -> DescriberResult<Option<String>> {
        Ok(self.conn.version().await?)
    }
}

fn rewrite_number_default(default: &mut DefaultValue, family: &ColumnTypeFamily) {
    let DefaultKind::Value(PrismaValue::Float(decimal)) = default.kind() else {
        return;
    };

    let Ok(value) = decimal.to_string().parse::<i64>() else {
        return;
    };

    match family {
        ColumnTypeFamily::Int => *default = DefaultValue::value(PrismaValue::Int(value)),
        ColumnTypeFamily::BigInt => *default = DefaultValue::value(PrismaValue::BigInt(value)),
        ColumnTypeFamily::Boolean if value == 0 || value == 1 => *default = DefaultValue::value(value == 1),
        _ => {}
    }
}

fn rewrite_sequence_default(default: &mut DefaultValue, tpe: &ColumnType) {
    let DefaultKind::DbGenerated(Some(expression)) = default.kind() else {
        return;
    };

    let normalized = expression.trim_start().to_ascii_lowercase();
    if !normalized.starts_with("nextval(") && !normalized.starts_with("pg_catalog.nextval(") {
        return;
    }

    if let Some(parsed) = postgres::default::get_default_value(expression, tpe) {
        *default = parsed;
    }
}

/// Delegates to the Oracle native connection while replacing the PostgreSQL
/// describer's internal `= ANY ($1)` namespace predicate. The only rewritten
/// parameter is a `Value::array()` containing schema names built by the
/// describer itself.
struct OracleCatalogConnection<'a> {
    inner: &'a dyn Queryable,
}

impl<'a> OracleCatalogConnection<'a> {
    fn new(inner: &'a dyn Queryable) -> Self {
        Self { inner }
    }

    async fn query_raw_without_schema_array(
        &self,
        sql: &str,
        params: &[Value<'_>],
        typed: bool,
    ) -> quaint::Result<ResultSet> {
        let sql = exclude_extension_owned_views(sql);

        match rewrite_schema_array_filter(&sql, params) {
            Some(SchemaArrayFilter::Empty) => Ok(ResultSet::default()),
            Some(SchemaArrayFilter::Sql(sql)) => {
                if typed {
                    self.inner.query_raw_typed(&sql, &[]).await
                } else {
                    self.inner.query_raw(&sql, &[]).await
                }
            }
            None if typed => self.inner.query_raw_typed(&sql, params).await,
            None => self.inner.query_raw(&sql, params).await,
        }
    }
}

#[async_trait::async_trait]
impl Queryable for OracleCatalogConnection<'_> {
    fn as_external_connector(&self) -> Option<&dyn ExternalConnector> {
        self.inner.as_external_connector()
    }

    async fn query(&self, query: quaint::ast::Query<'_>) -> quaint::Result<ResultSet> {
        self.inner.query(query).await
    }

    async fn query_raw(&self, sql: &str, params: &[Value<'_>]) -> quaint::Result<ResultSet> {
        self.query_raw_without_schema_array(sql, params, false).await
    }

    async fn query_raw_typed(&self, sql: &str, params: &[Value<'_>]) -> quaint::Result<ResultSet> {
        self.query_raw_without_schema_array(sql, params, true).await
    }

    async fn execute(&self, query: quaint::ast::Query<'_>) -> quaint::Result<u64> {
        self.inner.execute(query).await
    }

    async fn execute_raw(&self, sql: &str, params: &[Value<'_>]) -> quaint::Result<u64> {
        self.inner.execute_raw(sql, params).await
    }

    async fn execute_raw_typed(&self, sql: &str, params: &[Value<'_>]) -> quaint::Result<u64> {
        self.inner.execute_raw_typed(sql, params).await
    }

    async fn raw_cmd(&self, sql: &str) -> quaint::Result<()> {
        self.inner.raw_cmd(sql).await
    }

    async fn version(&self) -> quaint::Result<Option<String>> {
        self.inner.version().await
    }

    async fn describe_query(&self, sql: &str) -> quaint::Result<DescribedQuery> {
        self.inner.describe_query(sql).await
    }

    fn is_healthy(&self) -> bool {
        self.inner.is_healthy()
    }

    async fn server_reset_query(&self, transaction: &dyn Transaction) -> quaint::Result<()> {
        self.inner.server_reset_query(transaction).await
    }

    fn begin_statement(&self) -> &'static str {
        self.inner.begin_statement()
    }

    async fn set_tx_isolation_level(&self, isolation_level: IsolationLevel) -> quaint::Result<()> {
        self.inner.set_tx_isolation_level(isolation_level).await
    }

    fn requires_isolation_first(&self) -> bool {
        self.inner.requires_isolation_first()
    }
}

enum SchemaArrayFilter {
    Empty,
    Sql(String),
}

fn rewrite_schema_array_filter(sql: &str, params: &[Value<'_>]) -> Option<SchemaArrayFilter> {
    let [
        Value {
            typed: quaint::ValueType::Array(Some(values)),
            ..
        },
    ] = params
    else {
        return None;
    };

    let namespaces = values
        .iter()
        .map(|value| match &value.typed {
            quaint::ValueType::Text(Some(value)) => Some(value.as_ref()),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()?;

    if !sql.contains("= ANY ( $1 )") {
        return None;
    }

    if namespaces.is_empty() {
        // PostgreSQL accepts an empty text array for `= ANY ($1)` and returns
        // no rows. Kingbase Oracle mode rejects array bind parameters, so
        // preserve the same observable result without issuing a query.
        return Some(SchemaArrayFilter::Empty);
    }

    let literals = namespaces
        .into_iter()
        .map(|namespace| format!("'{}'", namespace.replace('\'', "''")))
        .collect::<Vec<_>>()
        .join(", ");
    let replacement = format!("IN ({literals})");
    let rewritten = sql.replace("= ANY ( $1 )", &replacement);

    (rewritten != sql).then_some(SchemaArrayFilter::Sql(rewritten))
}

fn exclude_extension_owned_views(sql: &str) -> Cow<'_, str> {
    const VIEW_QUERY: &str = "FROM pg_catalog.pg_views views";
    const NAMESPACE_FILTER: &str = "WHERE schemaname = ANY ( $1 )";
    const EXTENSION_FILTER: &str = r#"WHERE schemaname = ANY ( $1 )
              AND NOT EXISTS (
                  SELECT 1
                  FROM pg_catalog.pg_depend dependency
                  WHERE dependency.classid = 'pg_catalog.pg_class'::regclass
                    AND dependency.objid = class.oid
                    AND dependency.deptype = 'e'
              )"#;

    if !sql.contains(VIEW_QUERY) || !sql.contains(NAMESPACE_FILTER) {
        return Cow::Borrowed(sql);
    }

    Cow::Owned(sql.replacen(NAMESPACE_FILTER, EXTENSION_FILTER, 1))
}

fn rewrite_column_type(tpe: &mut ColumnType) {
    // Kingbase Oracle mode does not expose Prisma scalar-list semantics. Keep
    // PostgreSQL array catalog entries unsupported instead of leaking a
    // PostgreSQL native type through an Oracle connection.
    if tpe.arity == ColumnArity::List {
        tpe.family = ColumnTypeFamily::Unsupported(tpe.full_data_type.clone());
        tpe.native_type = None;
        return;
    }

    let type_name = tpe.full_data_type.to_ascii_lowercase();
    let (mapped_type, has_postgres_native_type) = {
        let postgres_native_type = tpe
            .native_type
            .as_ref()
            .map(|native_type| native_type.downcast_ref::<PostgresType>());

        let mapped_type = match type_name.as_str() {
            "tinyint" => Some((ColumnTypeFamily::Int, KingbaseOracleType::TinyInt)),
            "int2" | "int4" => Some(number_type(KingbaseOracleNumberArguments::PrecisionAndScale(10, 0))),
            "int8" => Some(number_type(KingbaseOracleNumberArguments::PrecisionAndScale(19, 0))),
            "numeric" => Some(number_type(number_arguments(postgres_native_type))),
            "float4" => Some((ColumnTypeFamily::Float, KingbaseOracleType::BinaryFloat)),
            "float8" => Some((ColumnTypeFamily::Float, KingbaseOracleType::BinaryDouble)),
            "bool" => Some((ColumnTypeFamily::Boolean, KingbaseOracleType::Boolean)),
            "bpchar" | "char" => Some((
                ColumnTypeFamily::String,
                KingbaseOracleType::Char(character_length(postgres_native_type)),
            )),
            "varchar" => Some((
                ColumnTypeFamily::String,
                KingbaseOracleType::VarChar2(character_length(postgres_native_type)),
            )),
            // In Oracle mode, `text` is the underlying Kingbase representation of
            // an Oracle character LOB. `CLOB` and `NCLOB` retain their type names.
            "text" | "clob" => Some((ColumnTypeFamily::String, KingbaseOracleType::Clob)),
            "nclob" => Some((ColumnTypeFamily::String, KingbaseOracleType::NClob)),
            "bytea" | "blob" => Some((ColumnTypeFamily::Binary, KingbaseOracleType::Blob)),
            "date" => Some((ColumnTypeFamily::DateTime, KingbaseOracleType::Date)),
            "timestamp" => Some((
                ColumnTypeFamily::DateTime,
                KingbaseOracleType::Timestamp(timestamp_precision(postgres_native_type)),
            )),
            "timestamptz" => Some((
                ColumnTypeFamily::DateTime,
                KingbaseOracleType::TimestampTz(timestamp_precision(postgres_native_type)),
            )),
            "json" | "jsonb" => Some((ColumnTypeFamily::Json, KingbaseOracleType::Json)),
            "uuid" => Some((ColumnTypeFamily::String, KingbaseOracleType::Uuid)),
            "xml" => Some((ColumnTypeFamily::String, KingbaseOracleType::Xml)),
            _ => None,
        };

        (mapped_type, postgres_native_type.is_some())
    };

    if let Some((family, native_type)) = mapped_type {
        tpe.family = family;
        tpe.native_type = Some(NativeTypeInstance::new::<KingbaseOracleType>(native_type));
    } else {
        // A PostgreSQL native type must never remain attached to an Oracle
        // schema. Retain enum/UDT families for generic introspection, and make
        // other unsupported catalog types explicit.
        tpe.native_type = None;
        if has_postgres_native_type {
            tpe.family = ColumnTypeFamily::Unsupported(tpe.full_data_type.clone());
        }
    }
}

fn number_type(arguments: KingbaseOracleNumberArguments) -> (ColumnTypeFamily, KingbaseOracleType) {
    use KingbaseOracleNumberArguments::*;

    let family = match arguments {
        // NUMBER(1) can contain any single-digit number, not only 0 and 1.
        // Reserve Prisma Boolean for the actual Oracle-mode BOOLEAN type.
        Precision(precision) | PrecisionAndScale(precision, 0) if precision <= 10 => ColumnTypeFamily::Int,
        Precision(precision) | PrecisionAndScale(precision, 0) if precision <= 19 => ColumnTypeFamily::BigInt,
        _ => ColumnTypeFamily::Decimal,
    };

    (family, KingbaseOracleType::Number(arguments))
}

fn number_arguments(native_type: Option<&PostgresType>) -> KingbaseOracleNumberArguments {
    match native_type {
        Some(PostgresType::Known(KnownPostgresType::Decimal(Some((precision, scale))))) => {
            KingbaseOracleNumberArguments::PrecisionAndScale(*precision, *scale)
        }
        Some(PostgresType::Known(KnownPostgresType::Decimal(None))) | None => {
            KingbaseOracleNumberArguments::Unspecified
        }
        _ => KingbaseOracleNumberArguments::Unspecified,
    }
}

fn character_length(native_type: Option<&PostgresType>) -> Option<u32> {
    match native_type {
        Some(PostgresType::Known(KnownPostgresType::Char(length)))
        | Some(PostgresType::Known(KnownPostgresType::VarChar(length))) => *length,
        _ => None,
    }
}

fn timestamp_precision(native_type: Option<&PostgresType>) -> Option<u32> {
    match native_type {
        Some(PostgresType::Known(KnownPostgresType::Timestamp(precision)))
        | Some(PostgresType::Known(KnownPostgresType::Timestamptz(precision))) => *precision,
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ColumnArity, ColumnType};

    fn postgres_column(full_data_type: &str, family: ColumnTypeFamily, native_type: PostgresType) -> ColumnType {
        ColumnType {
            full_data_type: full_data_type.to_owned(),
            family,
            arity: ColumnArity::Required,
            native_type: Some(NativeTypeInstance::new::<PostgresType>(native_type)),
        }
    }

    #[test]
    fn maps_postgres_catalog_numeric_to_oracle_number() {
        let mut tpe = postgres_column(
            "numeric",
            ColumnTypeFamily::Decimal,
            PostgresType::Known(KnownPostgresType::Decimal(Some((10, 2)))),
        );

        rewrite_column_type(&mut tpe);

        assert_eq!(tpe.family, ColumnTypeFamily::Decimal);
        assert_eq!(
            tpe.native_type.as_ref().unwrap().downcast_ref::<KingbaseOracleType>(),
            &KingbaseOracleType::Number(KingbaseOracleNumberArguments::PrecisionAndScale(10, 2))
        );
    }

    #[test]
    fn maps_number_one_to_int_instead_of_boolean() {
        assert_eq!(
            number_type(KingbaseOracleNumberArguments::Precision(1)).0,
            ColumnTypeFamily::Int
        );
        assert_eq!(
            number_type(KingbaseOracleNumberArguments::PrecisionAndScale(1, 0)).0,
            ColumnTypeFamily::Int
        );
    }

    #[test]
    fn maps_oracle_lobs_without_postgres_native_type() {
        let mut tpe = ColumnType {
            full_data_type: "clob".to_owned(),
            family: ColumnTypeFamily::Unsupported("clob".to_owned()),
            arity: ColumnArity::Required,
            native_type: None,
        };

        rewrite_column_type(&mut tpe);

        assert_eq!(tpe.family, ColumnTypeFamily::String);
        assert_eq!(
            tpe.native_type.as_ref().unwrap().downcast_ref::<KingbaseOracleType>(),
            &KingbaseOracleType::Clob
        );
    }

    #[test]
    fn leaves_postgres_arrays_unsupported() {
        let mut tpe = postgres_column(
            "_varchar",
            ColumnTypeFamily::String,
            PostgresType::Known(KnownPostgresType::VarChar(Some(40))),
        );
        tpe.arity = ColumnArity::List;

        rewrite_column_type(&mut tpe);

        assert!(matches!(tpe.family, ColumnTypeFamily::Unsupported(_)));
        assert!(tpe.native_type.is_none());
    }
}
