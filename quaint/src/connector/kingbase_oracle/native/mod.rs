//! Native connection support for KingbaseES in Oracle-compatible mode.
//!
//! SQL AST queries are rendered by the dedicated Kingbase Oracle visitor and
//! executed with Oracle-compatible bind parameter types.

mod column_type;
mod conversion;
mod error;

use column_type::{column_type_from_type, is_catalog_text_array, is_oracle_text_type};
use error::convert_driver_error;

use crate::{
    ast::{Query, Value, ValueType},
    connector::{
        DescribedColumn, DescribedParameter, DescribedQuery, IsolationLevel, ResultSet, Transaction, queryable::*,
        trace,
    },
    error::{Error, ErrorKind, NativeErrorKind},
    visitor::{KingbaseOracle as KingbaseOracleVisitor, Visitor},
};
use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use futures::{Future, StreamExt, future::FutureExt};
use kingbase_postgres_native_tls::MakeTlsConnector;
use kingbase_tokio_postgres::{Client, Row, types::Type};
use native_tls::{Certificate, Identity, TlsConnector};
use std::{
    borrow::Cow,
    fs,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use tokio::task::JoinHandle;
use tracing_futures::WithSubscriber;

use super::{
    KingbaseOracleUrl,
    url::{SslAcceptMode, SslParams},
};

const DB_SYSTEM_NAME: &str = "kingbase";

/// A native KingbaseES connection using Oracle-compatible database semantics.
pub struct KingbaseOracle {
    client: Client,
    handle: JoinHandle<()>,
    socket_timeout: Option<Duration>,
    pg_bouncer: bool,
    is_healthy: AtomicBool,
}

impl KingbaseOracle {
    pub async fn new(url: KingbaseOracleUrl) -> crate::Result<Self> {
        let config = url.to_config()?;
        let tls = make_tls_connector(url.ssl_params())?;
        let connect = config.connect(tls);
        let (client, connection) = match url.connect_timeout() {
            Some(duration) => tokio::time::timeout(duration, connect)
                .await
                .map_err(|_| Error::builder(ErrorKind::Native(NativeErrorKind::ConnectTimeout)).build())?
                .map_err(convert_driver_error)?,
            None => connect.await.map_err(convert_driver_error)?,
        };

        let handle = tokio::spawn(
            connection
                .map(|result| {
                    if let Err(error) = result {
                        tracing::error!("Error in Kingbase Oracle connection: {error:?}");
                    }
                })
                .with_current_subscriber(),
        );

        let connection = Self {
            client,
            handle,
            socket_timeout: url.socket_timeout(),
            pg_bouncer: url.pg_bouncer(),
            is_healthy: AtomicBool::new(true),
        };

        // `TIMESTAMP WITH LOCAL TIME ZONE` is returned over the wire as a
        // plain `TIMESTAMP`, localized to the session time zone. Prisma
        // represents DateTime values as UTC instants, so keep the session in
        // UTC before any value can be written or decoded.
        connection
            .perform_io(connection.client.batch_execute("SET TIME ZONE 'UTC'"))
            .await?;

        Ok(connection)
    }

    async fn perform_io<F, T>(&self, future: F) -> crate::Result<T>
    where
        F: Future<Output = Result<T, kingbase_tokio_postgres::Error>>,
    {
        let result = match self.socket_timeout {
            Some(duration) => tokio::time::timeout(duration, future)
                .await
                .map_err(|_| Error::builder(ErrorKind::SocketTimeout).build())?,
            None => future.await,
        }
        .map_err(convert_driver_error);

        match result {
            Err(error) if error.is_closed() => {
                self.is_healthy.store(false, Ordering::SeqCst);
                Err(error)
            }
            result => result,
        }
    }

    fn check_bind_variables_len(params: &[Value<'_>]) -> crate::Result<()> {
        if params.len() <= i16::MAX as usize {
            Ok(())
        } else {
            Err(Error::builder(ErrorKind::QueryInvalidInput(format!(
                "too many bind variables in prepared statement, expected maximum of {}, received {}",
                i16::MAX,
                params.len()
            )))
            .build())
        }
    }

    async fn query_raw_impl(&self, sql: &str, params: &[Value<'_>], types: &[Type]) -> crate::Result<ResultSet> {
        Self::check_bind_variables_len(params)?;

        let statement = self.perform_io(self.client.prepare_typed(sql, types)).await?;
        if statement.params().len() != params.len() {
            return Err(Error::builder(ErrorKind::IncorrectNumberOfParameters {
                expected: statement.params().len(),
                actual: params.len(),
            })
            .build());
        }

        let converted = conversion::convert_params(params);
        let parameter_refs = conversion::as_params(&converted);
        let mut rows = Box::pin(
            self.perform_io(self.client.query_raw(&statement, parameter_refs))
                .await?,
        );

        let names = statement
            .columns()
            .iter()
            .map(|column| column.name().to_owned())
            .collect::<Vec<_>>();
        let types = statement
            .columns()
            .iter()
            .map(|column| column_type_from_type(column.type_()))
            .collect::<Vec<_>>();
        let mut result = ResultSet::new(names, types, Vec::new());

        while let Some(row) = rows.next().await {
            result.rows.push(convert_row(&driver_result(row)?)?);
        }

        Ok(result)
    }

    async fn execute_raw_impl(&self, sql: &str, params: &[Value<'_>], types: &[Type]) -> crate::Result<u64> {
        Self::check_bind_variables_len(params)?;

        let statement = self.perform_io(self.client.prepare_typed(sql, types)).await?;
        if statement.params().len() != params.len() {
            return Err(Error::builder(ErrorKind::IncorrectNumberOfParameters {
                expected: statement.params().len(),
                actual: params.len(),
            })
            .build());
        }

        let converted = conversion::convert_params(params);
        let parameter_refs = conversion::as_params(&converted);
        self.perform_io(self.client.execute(&statement, &parameter_refs)).await
    }

    /// Closes the connection and waits for its background task.
    pub async fn close(self) {
        drop(self.client);
        self.handle.await.expect("Kingbase Oracle connection task panicked");
    }
}

fn driver_result<T>(result: Result<T, kingbase_tokio_postgres::Error>) -> crate::Result<T> {
    result.map_err(convert_driver_error)
}

fn convert_row(row: &Row) -> crate::Result<Vec<Value<'static>>> {
    let mut values = Vec::with_capacity(row.columns().len());

    for (index, column) in row.columns().iter().enumerate() {
        let typ = column.type_();
        let value = if typ == &Type::BOOL {
            ValueType::Boolean(driver_result(row.try_get(index))?).into()
        } else if typ == &Type::INT2 {
            let value: Option<i16> = driver_result(row.try_get(index))?;
            ValueType::Int32(value.map(i32::from)).into()
        } else if typ == &Type::INT4 {
            ValueType::Int32(driver_result(row.try_get(index))?).into()
        } else if typ == &Type::INT8 {
            ValueType::Int64(driver_result(row.try_get(index))?).into()
        } else if typ == &Type::OID {
            let value: Option<u32> = driver_result(row.try_get(index))?;
            ValueType::Int64(value.map(i64::from)).into()
        } else if typ == &Type::FLOAT4 {
            ValueType::Float(driver_result(row.try_get(index))?).into()
        } else if typ == &Type::FLOAT8 {
            ValueType::Double(driver_result(row.try_get(index))?).into()
        } else if typ == &Type::NUMERIC {
            let value: Option<conversion::DecimalWrapper> = driver_result(row.try_get(index))?;
            ValueType::Numeric(value.map(|value| value.0)).into()
        } else if is_oracle_text_type(typ) {
            let value: Option<String> = driver_result(row.try_get(index))?;
            ValueType::Text(value.map(Cow::Owned)).into()
        } else if typ == &Type::XML {
            let value: Option<String> = driver_result(row.try_get(index))?;
            ValueType::Xml(value.map(Cow::Owned)).into()
        } else if matches!(typ, &Type::BYTEA | &Type::ORACLE_BLOB) {
            let value: Option<Vec<u8>> = driver_result(row.try_get(index))?;
            ValueType::Bytes(value.map(Cow::Owned)).into()
        } else if matches!(typ, &Type::JSON | &Type::JSONB) {
            ValueType::Json(driver_result(row.try_get(index))?).into()
        } else if typ == &Type::UUID {
            ValueType::Uuid(driver_result(row.try_get(index))?).into()
        } else if typ == &Type::CHAR {
            let value: Option<i8> = driver_result(row.try_get(index))?;
            ValueType::Char(value.map(|value| (value as u8) as char)).into()
        } else if matches!(typ, &Type::TIMESTAMP | &Type::ORACLE_SYS_DATE) {
            let value: Option<NaiveDateTime> = driver_result(row.try_get(index))?;
            ValueType::DateTime(value.map(|value| DateTime::<Utc>::from_naive_utc_and_offset(value, Utc))).into()
        } else if typ == &Type::TIMESTAMPTZ {
            ValueType::DateTime(driver_result(row.try_get::<_, Option<DateTime<Utc>>>(index))?).into()
        } else if typ == &Type::DATE {
            ValueType::Date(driver_result(row.try_get::<_, Option<NaiveDate>>(index))?).into()
        } else if typ == &Type::TIME {
            ValueType::Time(driver_result(row.try_get::<_, Option<NaiveTime>>(index))?).into()
        } else if is_catalog_text_array(typ) {
            let values: Option<Vec<Option<&str>>> = driver_result(row.try_get(index))?;
            ValueType::Array(values.map(|values| {
                values
                    .into_iter()
                    .map(|value| ValueType::Text(value.map(ToOwned::to_owned).map(Cow::Owned)).into())
                    .collect()
            }))
            .into()
        } else {
            return Err(Error::builder(ErrorKind::UnsupportedColumnType {
                column_type: typ.to_string(),
            })
            .build());
        };

        values.push(value);
    }

    Ok(values)
}

fn make_tls_connector(ssl_params: &SslParams) -> crate::Result<MakeTlsConnector> {
    let mut tls_builder = TlsConnector::builder();

    if let Some(certificate_file) = &ssl_params.certificate_file {
        let certificate = fs::read(certificate_file).map_err(|error| {
            Error::builder(ErrorKind::Native(NativeErrorKind::TlsError {
                message: format!("cert file not found ({error})"),
            }))
            .build()
        })?;
        let certificate = Certificate::from_pem(&certificate).map_err(|error| {
            Error::builder(ErrorKind::Native(NativeErrorKind::TlsError {
                message: error.to_string(),
            }))
            .build()
        })?;
        tls_builder.add_root_certificate(certificate);
    }

    tls_builder.danger_accept_invalid_certs(ssl_params.ssl_accept_mode == SslAcceptMode::AcceptInvalidCerts);

    if let Some(identity_file) = &ssl_params.identity_file {
        let identity = fs::read(identity_file).map_err(|error| {
            Error::builder(ErrorKind::Native(NativeErrorKind::TlsError {
                message: format!("identity file not found ({error})"),
            }))
            .build()
        })?;
        let password = ssl_params.identity_password.0.as_deref().unwrap_or("");
        let identity = Identity::from_pkcs12(&identity, password).map_err(|error| {
            Error::builder(ErrorKind::Native(NativeErrorKind::TlsError {
                message: error.to_string(),
            }))
            .build()
        })?;
        tls_builder.identity(identity);
    }

    tls_builder
        .build()
        .map_err(|error| {
            Error::builder(ErrorKind::Native(NativeErrorKind::TlsError {
                message: error.to_string(),
            }))
            .build()
        })
        .map(MakeTlsConnector::new)
}

impl_default_TransactionCapable!(KingbaseOracle);

#[async_trait]
impl Queryable for KingbaseOracle {
    async fn query(&self, query: Query<'_>) -> crate::Result<ResultSet> {
        let (sql, params) = KingbaseOracleVisitor::build(query)?;
        self.query_raw_typed(sql.as_str(), &params).await
    }

    async fn query_raw(&self, sql: &str, params: &[Value<'_>]) -> crate::Result<ResultSet> {
        trace::query(DB_SYSTEM_NAME, sql, params, move || async move {
            self.query_raw_impl(sql, params, &[]).await
        })
        .await
    }

    async fn query_raw_typed(&self, sql: &str, params: &[Value<'_>]) -> crate::Result<ResultSet> {
        trace::query(DB_SYSTEM_NAME, sql, params, move || async move {
            self.query_raw_impl(sql, params, &conversion::params_to_types(params))
                .await
        })
        .await
    }

    async fn execute(&self, query: Query<'_>) -> crate::Result<u64> {
        let (sql, params) = KingbaseOracleVisitor::build(query)?;
        self.execute_raw_typed(sql.as_str(), &params).await
    }

    async fn execute_raw(&self, sql: &str, params: &[Value<'_>]) -> crate::Result<u64> {
        trace::query(DB_SYSTEM_NAME, sql, params, move || async move {
            self.execute_raw_impl(sql, params, &[]).await
        })
        .await
    }

    async fn execute_raw_typed(&self, sql: &str, params: &[Value<'_>]) -> crate::Result<u64> {
        trace::query(DB_SYSTEM_NAME, sql, params, move || async move {
            self.execute_raw_impl(sql, params, &conversion::params_to_types(params))
                .await
        })
        .await
    }

    async fn raw_cmd(&self, command: &str) -> crate::Result<()> {
        trace::query(DB_SYSTEM_NAME, command, &[], move || async move {
            self.perform_io(self.client.simple_query(command)).await?;
            Ok(())
        })
        .await
    }

    async fn version(&self) -> crate::Result<Option<String>> {
        let result = self.query_raw("SELECT version()", &[]).await?;
        Ok(result
            .first()
            .and_then(|row| row.get("version").and_then(|value| value.to_string())))
    }

    async fn describe_query(&self, sql: &str) -> crate::Result<DescribedQuery> {
        let statement = self.perform_io(self.client.prepare_typed(sql, &[])).await?;
        let columns = statement
            .columns()
            .iter()
            .map(|column| {
                DescribedColumn::new_named(column.name(), column_type_from_type(column.type_())).is_nullable(true)
            })
            .collect();
        let parameters = statement
            .params()
            .iter()
            .enumerate()
            .map(|(index, typ)| DescribedParameter::new_unnamed(index, column_type_from_type(typ)))
            .collect();

        Ok(DescribedQuery {
            columns,
            parameters,
            enum_names: None,
        })
    }

    fn is_healthy(&self) -> bool {
        self.is_healthy.load(Ordering::SeqCst)
    }

    async fn server_reset_query(&self, transaction: &dyn Transaction) -> crate::Result<()> {
        if self.pg_bouncer {
            transaction.raw_cmd("DEALLOCATE ALL").await
        } else {
            Ok(())
        }
    }

    async fn set_tx_isolation_level(&self, isolation_level: IsolationLevel) -> crate::Result<()> {
        if !matches!(
            isolation_level,
            IsolationLevel::ReadCommitted | IsolationLevel::Serializable
        ) {
            return Err(Error::builder(ErrorKind::invalid_isolation_level(&isolation_level)).build());
        }

        self.raw_cmd(&format!("SET TRANSACTION ISOLATION LEVEL {isolation_level}"))
            .await
    }

    fn requires_isolation_first(&self) -> bool {
        false
    }
}
