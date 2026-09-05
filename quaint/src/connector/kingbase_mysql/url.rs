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

/// A KingbaseES connection URL for the MySQL-compatible provider.
///
/// The public Prisma URL scheme is `kingbase-mysql://`. The underlying driver
/// accepts the equivalent `kingbase://` spelling, so this module normalizes it
/// only at the driver boundary.
#[derive(Debug, Clone)]
pub struct KingbaseMysqlUrl {
    url: Url,
    query_params: KingbaseMysqlUrlQueryParams,
}

impl KingbaseMysqlUrl {
    pub fn new(url: Url) -> crate::Result<Self> {
        if !matches!(url.scheme(), "kingbase-mysql" | "kingbase") {
            let kind = ErrorKind::DatabaseUrlIsInvalid(format!(
                "{} is not a supported Kingbase database URL scheme.",
                url.scheme()
            ));

            return Err(Error::builder(kind).build());
        }

        let query_params = KingbaseMysqlUrlQueryParams::parse(&url)?;

        Ok(Self { url, query_params })
    }

    /// Builds the driver configuration from the connection URL.
    pub(crate) fn to_config(&self) -> crate::Result<Config> {
        let mut driver_url = self.url.clone();
        if driver_url.scheme() == "kingbase-mysql" {
            driver_url.set_scheme("kingbase").map_err(|_| {
                Error::builder(ErrorKind::DatabaseUrlIsInvalid("invalid Kingbase URL scheme".into())).build()
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

        // Keep unqualified DDL and information_schema introspection on the same
        // schema. The URL may override this with `?schema=...`.
        let schema = self
            .query_params
            .schema
            .as_deref()
            .unwrap_or(super::DEFAULT_KINGBASE_MYSQL_SCHEMA);
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
        self.url.port().unwrap_or(5432)
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
struct KingbaseMysqlUrlQueryParams {
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

impl KingbaseMysqlUrlQueryParams {
    fn parse(url: &Url) -> crate::Result<Self> {
        let mut schema = None;
        let mut certificate_file = None;
        let mut identity_file = None;
        let mut identity_password = None;
        let mut ssl_accept_mode = SslAcceptMode::AcceptInvalidCerts;
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
                "connect_timeout" => {
                    connect_timeout = parse_optional_timeout(&value)?;
                }
                "socket_timeout" => {
                    socket_timeout = parse_optional_timeout(&value)?;
                }
                "connection_limit" => {
                    connection_limit = Some(
                        value
                            .parse()
                            .map_err(|_| Error::builder(ErrorKind::InvalidConnectionArguments).build())?,
                    );
                }
                "pool_timeout" => {
                    pool_timeout = parse_optional_timeout(&value)?;
                }
                "max_connection_lifetime" => {
                    max_connection_lifetime = parse_optional_timeout(&value)?;
                }
                "max_idle_connection_lifetime" => {
                    max_idle_connection_lifetime = parse_optional_timeout(&value)?;
                }
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
    use super::{KingbaseMysqlUrl, SslAcceptMode};
    use url::Url;

    #[test]
    fn configures_kingbase_driver_settings() {
        let url = KingbaseMysqlUrl::new(
            Url::parse("kingbase://user:password@localhost:54321/app?schema=tenant&pgbouncer=true").unwrap(),
        )
        .unwrap();

        let config = url.to_config().unwrap();

        assert!(config.get_pgbouncer_mode());
        assert_eq!(config.get_search_path().map(String::as_str), Some("\"tenant\""));
    }

    #[test]
    fn rejects_other_url_schemes() {
        let error = KingbaseMysqlUrl::new(Url::parse("postgresql://localhost/app").unwrap()).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("not a supported Kingbase database URL scheme")
        );
    }

    #[test]
    fn accepts_public_provider_scheme() {
        let url = KingbaseMysqlUrl::new(Url::parse("kingbase-mysql://localhost/app").unwrap()).unwrap();

        assert!(!url.to_config().unwrap().get_pgbouncer_mode());
    }

    #[test]
    fn parses_strict_sslaccept_mode() {
        let url = KingbaseMysqlUrl::new(
            Url::parse("kingbase-mysql://localhost/app?sslmode=require&sslaccept=strict").unwrap(),
        )
        .unwrap();

        assert_eq!(url.ssl_params().ssl_accept_mode, SslAcceptMode::Strict);
    }
}
