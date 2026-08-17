//! Native KingbaseES connector for the MySQL-compatible provider.
//!
//! Kingbase speaks the PostgreSQL wire protocol, but this connector is kept
//! separate from the PostgreSQL connector so provider-specific URL routing and
//! value conversion do not change the existing PostgreSQL path.

mod column_type;
mod conversion;
mod error;

use column_type::column_type_from_column;

use crate::{
    ast::{Query, Value},
    connector::{
        ColumnType, DescribedColumn, DescribedParameter, DescribedQuery, IsolationLevel, ResultSet, Transaction,
        queryable::*, timeout, trace,
    },
    error::{Error, ErrorKind, NativeErrorKind},
    visitor::KingbaseMysql as KingbaseMysqlVisitor,
};
use async_trait::async_trait;
use futures::{Future, StreamExt, future::FutureExt};
use native_tls::TlsConnector;
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use tokio::task::JoinHandle;
use tracing_futures::WithSubscriber;

use super::KingbaseMysqlUrl;
use kingbase_postgres_native_tls::MakeTlsConnector;
use kingbase_tokio_postgres::{Client, types::Type};

const DB_SYSTEM_NAME: &str = "kingbase";

/// A native Kingbase connection using the existing MySQL SQL visitor.
pub struct KingbaseMysql {
    client: Client,
    handle: JoinHandle<()>,
    socket_timeout: Option<Duration>,
    pg_bouncer: bool,
    is_healthy: AtomicBool,
}

impl KingbaseMysql {
    pub async fn new(url: KingbaseMysqlUrl) -> crate::Result<Self> {
        let config = url.to_config()?;
        let tls = TlsConnector::builder()
            .danger_accept_invalid_certs(true)
            .build()
            .map_err(|error| {
                Error::builder(ErrorKind::Native(NativeErrorKind::TlsError {
                    message: error.to_string(),
                }))
                .build()
            })?;
        let tls = MakeTlsConnector::new(tls);
        let (client, connection) = timeout::connect(url.connect_timeout(), config.connect(tls)).await?;

        let handle = tokio::spawn(
            connection
                .map(|result| {
                    if let Err(error) = result {
                        tracing::error!("Error in Kingbase connection: {error:?}");
                    }
                })
                .with_current_subscriber(),
        );

        Ok(Self {
            client,
            handle,
            socket_timeout: url.socket_timeout(),
            pg_bouncer: url.pg_bouncer(),
            is_healthy: AtomicBool::new(true),
        })
    }

    async fn perform_io<F, T>(&self, fut: F) -> crate::Result<T>
    where
        F: Future<Output = Result<T, kingbase_tokio_postgres::Error>>,
    {
        match timeout::socket(self.socket_timeout, fut).await {
            Err(error) if error.is_closed() => {
                self.is_healthy.store(false, Ordering::SeqCst);
                Err(error.into())
            }
            result => result,
        }
    }

    fn check_bind_variables_len(&self, params: &[Value<'_>]) -> crate::Result<()> {
        if params.len() > i16::MAX as usize {
            Err(Error::builder(ErrorKind::QueryInvalidInput(format!(
                "too many bind variables in prepared statement, expected maximum of {}, received {}",
                i16::MAX,
                params.len()
            )))
            .build())
        } else {
            Ok(())
        }
    }

    async fn query_raw_impl(&self, sql: &str, params: &[Value<'_>], types: &[Type]) -> crate::Result<ResultSet> {
        self.check_bind_variables_len(params)?;

        let statement = self.perform_io(self.client.prepare_typed(sql, types)).await?;

        if statement.params().len() != params.len() {
            return Err(Error::builder(ErrorKind::IncorrectNumberOfParameters {
                expected: statement.params().len(),
                actual: params.len(),
            })
            .build());
        }

        let mut rows = Box::pin(
            self.perform_io(self.client.query_raw(&statement, conversion::conv_params(params)))
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
            .map(column_type_from_column)
            .collect::<Vec<_>>();
        let mut result = ResultSet::new(names, types, Vec::new());

        while let Some(row) = rows.next().await {
            result.rows.push(row?.get_result_row()?);
        }

        Ok(result)
    }

    /// Closes the connection and waits for its background task.
    pub async fn close(self) {
        drop(self.client);
        self.handle.await.expect("Kingbase connection task panicked");
    }
}

impl_default_TransactionCapable!(KingbaseMysql);

#[async_trait]
impl Queryable for KingbaseMysql {
    async fn query(&self, query: Query<'_>) -> crate::Result<ResultSet> {
        let (sql, params) = KingbaseMysqlVisitor::build(query)?;
        self.query_raw(sql.as_str(), &params).await
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
        let (sql, params) = KingbaseMysqlVisitor::build(query)?;
        self.execute_raw(sql.as_str(), &params).await
    }

    async fn execute_raw(&self, sql: &str, params: &[Value<'_>]) -> crate::Result<u64> {
        trace::query(DB_SYSTEM_NAME, sql, params, move || async move {
            self.execute_raw_with_types(sql, params, &[]).await
        })
        .await
    }

    async fn execute_raw_typed(&self, sql: &str, params: &[Value<'_>]) -> crate::Result<u64> {
        trace::query(DB_SYSTEM_NAME, sql, params, move || async move {
            self.execute_raw_with_types(sql, params, &conversion::params_to_types(params))
                .await
        })
        .await
    }

    async fn raw_cmd(&self, cmd: &str) -> crate::Result<()> {
        trace::query(DB_SYSTEM_NAME, cmd, &[], move || async move {
            self.perform_io(self.client.simple_query(cmd)).await?;
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
            .map(|column| DescribedColumn::new_named(column.name(), column_type_from_column(column)).is_nullable(true))
            .collect();
        let parameters = statement
            .params()
            .iter()
            .enumerate()
            .map(|(idx, typ)| DescribedParameter::new_unnamed(idx, ColumnType::from(typ)))
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

    async fn server_reset_query(&self, tx: &dyn Transaction) -> crate::Result<()> {
        if self.pg_bouncer {
            tx.raw_cmd("DEALLOCATE ALL").await
        } else {
            Ok(())
        }
    }

    async fn set_tx_isolation_level(&self, isolation_level: IsolationLevel) -> crate::Result<()> {
        if matches!(isolation_level, IsolationLevel::Snapshot) {
            return Err(Error::builder(ErrorKind::invalid_isolation_level(&isolation_level)).build());
        }

        self.raw_cmd(&format!("SET TRANSACTION ISOLATION LEVEL {isolation_level}"))
            .await
    }

    fn requires_isolation_first(&self) -> bool {
        false
    }
}

impl KingbaseMysql {
    async fn execute_raw_with_types(&self, sql: &str, params: &[Value<'_>], types: &[Type]) -> crate::Result<u64> {
        self.check_bind_variables_len(params)?;
        let statement = self.perform_io(self.client.prepare_typed(sql, types)).await?;

        if statement.params().len() != params.len() {
            return Err(Error::builder(ErrorKind::IncorrectNumberOfParameters {
                expected: statement.params().len(),
                actual: params.len(),
            })
            .build());
        }

        self.perform_io(
            self.client
                .execute(&statement, conversion::conv_params(params).as_slice()),
        )
        .await
    }
}
