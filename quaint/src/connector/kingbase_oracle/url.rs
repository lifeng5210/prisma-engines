use crate::error::{Error, ErrorKind};
use kingbase_tokio_postgres::Config;
use std::{fmt, time::Duration};
use url::Url;

#[derive(Clone)]
pub(crate) struct Hidden<T>(pub(crate) T);

impl<T> fmt::Debug for Hidden<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<HIDDEN>")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SslAcceptMode {
    Strict,
    AcceptInvalidCerts,
}

#[derive(Debug, Clone)]
pub(crate) struct SslParams {
    pub(crate) certificate_file: Option<String>,
    pub(crate) identity_file: Option<String>,
    pub(crate) identity_password: Hidden<Option<String>>,
    pub(crate) ssl_accept_mode: SslAcceptMode,
}

/// A KingbaseES connection URL for the Oracle-compatible provider.
#[derive(Clone)]
pub struct KingbaseOracleUrl {
    url: Url,
    query_params: KingbaseOracleUrlQueryParams,
}

impl fmt::Debug for KingbaseOracleUrl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("KingbaseOracleUrl")
            .field("scheme", &self.url.scheme())
            .field("host", &self.host())
            .field("port", &self.port())
            .field("database", &self.dbname())
            .field("query_params", &self.query_params)
            .finish()
    }
}

impl KingbaseOracleUrl {
    pub fn new(url: Url) -> crate::Result<Self> {
        if url.scheme() != "kingbase-oracle" {
            let kind = ErrorKind::DatabaseUrlIsInvalid(format!(
                "{} is not a supported Kingbase Oracle database URL scheme.",
                url.scheme()
            ));

            return Err(Error::builder(kind).build());
        }

        let query_params = KingbaseOracleUrlQueryParams::parse(&url)?;

        Ok(Self { url, query_params })
    }

    /// Builds the PostgreSQL-wire driver configuration while preserving the
    /// Oracle-mode provider identity at the Quaint boundary.
    pub(crate) fn to_config(&self) -> crate::Result<Config> {
        let mut driver_url = self.url.clone();
        driver_url.set_scheme("kingbase").map_err(|_| {
            Error::builder(ErrorKind::DatabaseUrlIsInvalid(
                "invalid Kingbase Oracle URL scheme".into(),
            ))
            .build()
        })?;

        if driver_url.port().is_none() {
            driver_url
                .set_port(Some(super::DEFAULT_KINGBASE_ORACLE_PORT))
                .map_err(|_| {
                    Error::builder(ErrorKind::DatabaseUrlIsInvalid(
                        "invalid Kingbase Oracle URL port".into(),
                    ))
                    .build()
                })?;
        }

        let driver_params = self
            .url
            .query_pairs()
            .filter(|(key, _)| !is_quaint_only_parameter(key))
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect::<Vec<_>>();

        driver_url
            .query_pairs_mut()
            .clear()
            .extend_pairs(driver_params.iter().map(|(key, value)| (key.as_str(), value.as_str())));

        let mut config = driver_url
            .as_str()
            .parse::<Config>()
            .map_err(|error| Error::builder(ErrorKind::DatabaseUrlIsInvalid(error.to_string())).build())?;

        config.pgbouncer_mode(self.query_params.pg_bouncer);

        let schema = self
            .query_params
            .schema
            .as_deref()
            .unwrap_or(super::DEFAULT_KINGBASE_ORACLE_SCHEMA);
        config.search_path(format!("\"{}\"", schema.replace('"', "\"\"")));

        Ok(config)
    }

    pub(crate) fn connect_timeout(&self) -> Option<Duration> {
        self.query_params.connect_timeout
    }

    pub fn dbname(&self) -> Option<&str> {
        self.url.path().strip_prefix('/').filter(|name| !name.is_empty())
    }

    pub fn schema(&self) -> Option<&str> {
        self.query_params.schema.as_deref()
    }

    pub fn host(&self) -> &str {
        self.url.host_str().unwrap_or("localhost")
    }

    pub(crate) fn username(&self) -> &str {
        self.url.username()
    }

    pub fn port(&self) -> u16 {
        self.url.port().unwrap_or(super::DEFAULT_KINGBASE_ORACLE_PORT)
    }

    pub(crate) fn socket_timeout(&self) -> Option<Duration> {
        self.query_params.socket_timeout
    }

    pub(crate) fn connection_limit(&self) -> Option<usize> {
        self.query_params.connection_limit
    }

    pub(crate) fn pool_timeout(&self) -> Option<Duration> {
        self.query_params.pool_timeout
    }

    pub(crate) fn max_connection_lifetime(&self) -> Option<Duration> {
        self.query_params.max_connection_lifetime
    }

    pub(crate) fn max_idle_connection_lifetime(&self) -> Option<Duration> {
        self.query_params.max_idle_connection_lifetime
    }

    pub(crate) fn pg_bouncer(&self) -> bool {
        self.query_params.pg_bouncer
    }

    pub(crate) fn ssl_params(&self) -> &SslParams {
        &self.query_params.ssl_params
    }
}

fn is_quaint_only_parameter(key: &str) -> bool {
    matches!(
        key,
        "schema"
            | "pgbouncer"
            | "socket_timeout"
            | "connection_limit"
            | "pool_timeout"
            | "max_connection_lifetime"
            | "max_idle_connection_lifetime"
            | "statement_cache_size"
            | "sslcert"
            | "sslidentity"
            | "sslpassword"
            | "sslaccept"
    )
}

#[derive(Debug, Clone)]
struct KingbaseOracleUrlQueryParams {
    ssl_params: SslParams,
    schema: Option<String>,
    pg_bouncer: bool,
    connect_timeout: Option<Duration>,
    socket_timeout: Option<Duration>,
    connection_limit: Option<usize>,
    pool_timeout: Option<Duration>,
    max_connection_lifetime: Option<Duration>,
    max_idle_connection_lifetime: Option<Duration>,
}

impl KingbaseOracleUrlQueryParams {
    fn parse(url: &Url) -> crate::Result<Self> {
        let mut schema = None;
        let mut certificate_file = None;
        let mut identity_file = None;
        let mut identity_password = None;
        let mut ssl_accept_mode = SslAcceptMode::Strict;
        let mut pg_bouncer = false;
        let mut connect_timeout = None;
        let mut socket_timeout = None;
        let mut connection_limit = None;
        let mut pool_timeout = None;
        let mut max_connection_lifetime = None;
        let mut max_idle_connection_lifetime = None;

        for (key, value) in url.query_pairs() {
            match key.as_ref() {
                "schema" => schema = Some(value.into_owned()),
                "sslcert" => certificate_file = Some(value.into_owned()),
                "sslidentity" => identity_file = Some(value.into_owned()),
                "sslpassword" => identity_password = Some(value.into_owned()),
                "sslaccept" => {
                    ssl_accept_mode = match value.as_ref() {
                        "strict" => SslAcceptMode::Strict,
                        "accept_invalid_certs" => SslAcceptMode::AcceptInvalidCerts,
                        _ => {
                            tracing::debug!(
                                message = "Unsupported SSL accept mode, defaulting to `strict`",
                                mode = &*value
                            );

                            SslAcceptMode::Strict
                        }
                    };
                }
                "pgbouncer" => {
                    pg_bouncer = value
                        .parse()
                        .map_err(|_| Error::builder(ErrorKind::InvalidConnectionArguments).build())?;
                }
                "connect_timeout" => connect_timeout = parse_optional_timeout(&value)?,
                "socket_timeout" => socket_timeout = parse_optional_timeout(&value)?,
                "connection_limit" => {
                    connection_limit = Some(
                        value
                            .parse()
                            .map_err(|_| Error::builder(ErrorKind::InvalidConnectionArguments).build())?,
                    );
                }
                "pool_timeout" => pool_timeout = parse_optional_timeout(&value)?,
                "max_connection_lifetime" => max_connection_lifetime = parse_optional_timeout(&value)?,
                "max_idle_connection_lifetime" => max_idle_connection_lifetime = parse_optional_timeout(&value)?,
                _ => (),
            }
        }

        Ok(Self {
            ssl_params: SslParams {
                certificate_file,
                identity_file,
                identity_password: Hidden(identity_password),
                ssl_accept_mode,
            },
            schema,
            pg_bouncer,
            connect_timeout,
            socket_timeout,
            connection_limit,
            pool_timeout,
            max_connection_lifetime,
            max_idle_connection_lifetime,
        })
    }
}

fn parse_optional_timeout(value: &str) -> crate::Result<Option<Duration>> {
    let seconds = value
        .parse::<u64>()
        .map_err(|_| Error::builder(ErrorKind::InvalidConnectionArguments).build())?;

    Ok((seconds != 0).then(|| Duration::from_secs(seconds)))
}

#[cfg(test)]
mod tests {
    use super::{KingbaseOracleUrl, SslAcceptMode};
    use url::Url;

    #[test]
    fn configures_kingbase_oracle_driver_settings() {
        let url = KingbaseOracleUrl::new(
            Url::parse("kingbase-oracle://user:password@localhost:54325/app?schema=tenant&pgbouncer=true").unwrap(),
        )
        .unwrap();

        let config = url.to_config().unwrap();

        assert!(config.get_pgbouncer_mode());
        assert_eq!(config.get_search_path().map(String::as_str), Some("\"tenant\""));
    }

    #[test]
    fn rejects_non_oracle_kingbase_schemes() {
        for scheme in ["kingbase", "kingbase-mysql", "postgresql"] {
            let error = KingbaseOracleUrl::new(Url::parse(&format!("{scheme}://localhost/app")).unwrap()).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("not a supported Kingbase Oracle database URL scheme")
            );
        }
    }

    #[test]
    fn uses_kingbase_default_port_when_the_url_omits_one() {
        let url = KingbaseOracleUrl::new(Url::parse("kingbase-oracle://localhost/app").unwrap()).unwrap();

        assert_eq!(url.port(), 54321);
        assert_eq!(url.to_config().unwrap().get_ports(), &[54321]);
    }

    #[test]
    fn defaults_to_strict_sslaccept_mode() {
        let url =
            KingbaseOracleUrl::new(Url::parse("kingbase-oracle://localhost/app?sslmode=require").unwrap()).unwrap();

        assert_eq!(url.ssl_params().ssl_accept_mode, SslAcceptMode::Strict);
    }

    #[test]
    fn parses_explicit_accept_invalid_certs_sslaccept_mode() {
        let url = KingbaseOracleUrl::new(
            Url::parse("kingbase-oracle://localhost/app?sslmode=require&sslaccept=accept_invalid_certs").unwrap(),
        )
        .unwrap();

        assert_eq!(url.ssl_params().ssl_accept_mode, SslAcceptMode::AcceptInvalidCerts);
    }

    #[test]
    fn debug_output_hides_connection_credentials() {
        let url = KingbaseOracleUrl::new(
            Url::parse(
                "kingbase-oracle://user:database-secret@localhost/app?sslidentity=client.p12&sslpassword=identity-secret",
            )
            .unwrap(),
        )
        .unwrap();

        let debug = format!("{url:?}");

        assert!(!debug.contains("database-secret"));
        assert!(!debug.contains("identity-secret"));
        assert!(debug.contains("<HIDDEN>"));
    }
}
