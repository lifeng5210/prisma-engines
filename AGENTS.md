# Agent Playbook — Prisma Engines

## 1. Big Picture
- This repo hosts the **Prisma Engines**: PSL (schema parser/validator), schema-engine (migrate, introspect), query components (query compiler, driver adapters, compatibility harnesses), and utilities shared with Prisma Client.
- Prisma 7 roadmap status:
  - `url`, `directUrl` and `shadowDatabaseUrl` are **invalid** in PSL.
  - CLI/tests override connection info via schema-engine CLI (`--datasource`) or shared `TestApi::new_engine_with_connection_strings`.
  - Reference commit: `34b5a692b7bd79939a9a2c3ef97d816e749cda2f` (driver adapter override plumbing).
- Prisma has removed the **native Rust query engine** in favor of the **Query Compiler (QC)** architecture:
  - Query planning happens in Rust (`query-compiler` crate). Output: an expression tree (“query plan”).
  - Query interpretation/execution runs in Prisma Client TypeScript using driver adapters. The interpreter has no knowledge of connection strings or even whether it talks to a real DB.
  - A compatibility harness (`qc-test-runner.ts` in the main repo) still emulates the legacy GraphQL protocol so the CLI and tests behave as before while consumers migrate.
  - MongoDB support is not yet implemented for QC; Prisma 7 will ship without MongoDB, to be added later once a driver adapter exists.

---

## 2. Repository Orientation
Key directories:
- `psl/` – Prisma Schema Language parser, validator, config tooling.
- `schema-engine/` – Migration/introspection engine plus test suites.
- `prisma-fmt/` – Language server & formatter entry point (tests rely on `expect!` snapshots).
- `schema-engine/sql-migration-tests` / `sql-introspection-tests` – Heavy integration suites (require DBs).
- `query-engine/` – Unused MongoDB connector crate left for future reference and the connector test kit (integration tests exercise QC through the driver adapter executor here).
- `query-compiler/` – Query planner + associated Wasm, playground, and the new `core-tests` crate.
- `libs/` – Shared libraries (value types, driver adapters, test setup).
- `driver-adapters/` – Rust-side adapter utilities for the new query interpreter.

Supporting infra:
- Tests use Rust `cargo test`. Some suites expect database URLs in env (see §5).
- `test-setup` crate provisions databases when env vars are defined (Docker-based in CI).
- `UPDATE_EXPECT=1 cargo test …` regenerates `expect!` snapshots (common when diagnostics shift).

---

## 3. Current Domain Knowledge
### Kingbase MySQL Compatibility
- Kingbase's MySQL compatibility mode should follow the existing MySQL connector's type semantics and test expectations. The underlying database driver is different, but that alone must not change the exposed `ColumnType` behavior; only genuine Kingbase wire-codec differences should be handled in the Kingbase connector.
- Before making a Kingbase compatibility change, verify the behavior against Kingbase's official documentation or the installed Kingbase extension definitions. Use that evidence to distinguish a SQL dialect difference, a server capability difference, and a wire-codec difference.
- The Kingbase MySQL compatibility manual documents `information_schema` views for tables, columns, views, key usage, referential constraints, and routines. Treat those as separate describer contracts; metadata definition text is database-native and must not be asserted as byte-for-byte MySQL output.
- `information_schema.referential_constraints.delete_rule` and `.update_rule` are documented as enum columns and arrive through Quaint as `ValueType::Enum`, not `Text`. In Kingbase-only describer SQL, cast them to `TEXT` before using the shared string-to-`ForeignKeyAction` mapping.
- Kingbase MySQL documents `INT` and `INTEGER` as aliases for the MySQL `INT` type. Its catalog can report the latter, so normalize both to `ColumnTypeFamily::Int` / `KingbaseMySqlType::Int` in the Kingbase-only describer.
- Unlike MySQL/InnoDB, Kingbase does not automatically create an index on foreign-key columns. Describer tests must not expect non-unique child-column indexes unless the test DDL creates them explicitly; preserve the actual catalog rather than synthesizing MySQL-only indexes.
- Kingbase's catalog can report a MySQL `DECIMAL` column as `number`, with `numeric(...)` in the full type; normalize `decimal`, `numeric`, and `number` to the Kingbase MySQL decimal family in the describer.
- For Kingbase MySQL `BIT(1)`, `information_schema` may omit `numeric_precision`; determine the width from `full_data_type` before falling back to metadata, so `BIT(1)` remains `Boolean` and `BIT(n > 1)` is `Binary`.
- Kingbase may expose extension-owned views in the same schema as user views (for example `sys_stat_statements`). `pg_depend.deptype = 'e'` denotes an extension member in Kingbase's catalog, so the Kingbase MySQL describer filters those rows from user-view introspection.
- Kingbase MySQL does not accept MySQL `FULLTEXT INDEX` or `MATCH ... AGAINST` SQL. Render Prisma `@@fulltext` as a standalone GIN index over immutable `to_tsvector('simple', textcat(COALESCE(...), ...))`, identify that exact expression during introspection, and rewrite Query Compiler `MATCH ... AGAINST (... IN BOOLEAN MODE)` output to `websearch_to_tsquery('simple', ...)` / `ts_rank`; unlike `to_tsquery`, it accepts ordinary multi-word MySQL searches and `+` / `-` operators. In MySQL mode `||` is logical OR rather than string concatenation, so never use it in the `to_tsvector` document expression.
- Kingbase MySQL accepts `sslmode` through the PostgreSQL-wire driver. Parse Quaint's `sslaccept`: the default remains `accept_invalid_certs` for parity with MySQL/PostgreSQL, while `sslaccept=strict` enables certificate and hostname validation. `sslcert`, `sslidentity`, and `sslpassword` configure the native TLS root certificate or client identity.
- KingbaseES defaults to TCP port 54321, whereas `kingbase_tokio_postgres` inherits PostgreSQL's 5432 fallback. When a Kingbase URL omits its port, insert 54321 into the driver URL before parsing it into `Config`; adding a port afterwards merely appends a second value, leaving 5432 selected for the first host.
- Kingbase MySQL SQL rendering shares `SqlFamily::Mysql`, but its PostgreSQL-wire prepared-statement parameter count is a signed 16-bit value. Clamp both native and external Kingbase connection info to 32,767 bind values so batch planning never emits an unexecutable statement.

### Kingbase Oracle Compatibility
- Oracle `TINYINT` has OID 8100 and is a signed 8-bit wire value. PSL, rendering, migration, and introspection support it as `Int @db.TinyInt`; the Node adapter converts the driver's text result to a number. Do not add native Quaint raw-result support until the Kingbase Rust driver supplies an `ORACLE_TINYINT` codec.
- Oracle `ROWID` has OID 6123, but its binary wire representation is a compact internal value rather than UTF-8. Do not expose it through native Quaint raw results until the Kingbase Rust driver can request its textual representation or decode the wire value; the Node adapter's public text form is 23 characters.

### Datasource URLs
- PSL rejects `directUrl`/`shadowDatabaseUrl` with targeted diagnostics (`DatamodelError::new_datasource_*_removed_error`).
- Parser still records `url` (and uses span for override fallbacks).
- `Datasource::override_urls()` now fakes spans because overrides bypass PSL parsing.
- Schema-engine tests must supply overrides via `TestApi::new_engine_with_connection_strings(connection_string, Some(shadow_connection))`. The wrapper returns an `EngineTestApi`.
- Old fixtures relying on `directUrl` inside PSL must be rewritten or deleted.
- Query compiler already assumes datasource URLs are supplied externally (from Prisma Client).

### Text Completions
- `prisma-fmt` completions now only offer `url` (no more direct/shadow suggestions).
- Completion scenarios removed for the deprecated properties. Expect JSON fixtures to change if docs/completions change again.

### Diagnostics / Tests
- Many tests assert on colored output via `expect!`. Always regenerate expectations when diagnostics wording changes.
- Integration tests around multi-schema migrations still need real DB URLs; without them they skip/fail early.
- Query compiler tests use insta snapshots (`query-compiler/tests`). Regenerate with `UPDATE_EXPECT=1 cargo test -p query-compiler`.
- The connector test kit (`query-engine-tests`) relies on `cargo insta` snapshots too (see `connector-test-kit-rs` README).

---

## 4. Typical Workflows
### Linting / Formatting
- Rustfmt + cargo fmt (standard). JSON fixtures kept raw (no formatter).
- Full lint pass (formatting + clippy warnings as errors):
  ```bash
  make pedantic
  ```
  This runs `cargo fmt -- --check` and `cargo clippy --all-features --all-targets -Dwarnings`. Fix the compiler/clippy diagnostics first, then formatting.

### Running Tests
1. **Fast PSL/LSP suites**
   ```bash
   cargo test -p prisma-fmt -F psl/all
   ```
   Use `UPDATE_EXPECT=1` to refresh snapshots.

2. **Unit tests in PSL**
   ```bash
   cargo test -p psl -F all
   ```

3. **Unit tests for the whole workspace**
  ```bash
  make test-unit
  ```
  Use this one if you can't figure out the correct cargo features for a specific crate.
  Some library crates may be tricky to compile in isolation without feature unification.
  Unit tests are very fast so there's no problem running them for the whole workspace.

4. **Schema engine SQL tests**
   Require DB env vars (see `.test_database_urls/` in repo root). Example:
   ```bash
   source .test_database_urls/postgres
   cargo test -p sql-migration-tests migration_with_shadow_database -- --nocapture
   ```

5. **Schema engine integration**
  Similar pattern; rely on generated DB URLs. Without env vars tests will refuse to run (by design).

6. **Query compiler snapshots**
   ```bash
   UPDATE_EXPECT=1 cargo test -p query-compiler
   ```
   Graphviz (`dot`) optional; set `RENDER_DOT_TO_PNG` for visuals (requires Graphviz installed).

7. **Connector test kit (driver adapters + QC)**
   ```bash
   make dev-pg-qc    # or another dev-*-qc helper; builds QC Wasm + driver adapters and writes .test_config
   cargo test -p query-engine-tests -- --nocapture
   ```
   Set `DRIVER_ADAPTER=<adapter>` (see Makefile) when you want to run against a specific adapter target. See `query-engine/connector-test-kit-rs/README.md` for env vars and adapter-specific notes.

### Updating expect! snapshots
```bash
UPDATE_EXPECT=1 cargo test -p prisma-fmt [optional::test::path]
```
Ensure diffs make sense and rerun without `UPDATE_EXPECT` to confirm.

---

## 5. Environment Essentials
- **Databases**: env vars follow `TEST_DATABASE_URL`, `TEST_SHADOW_DATABASE_URL`, etc. Use the `.test_database_urls/` helper scripts or docker-compose setup from team docs.
- **Linear tickets**: two key Prisma 7 projects – *Breaking Changes* and *New Features*. Search via Linear MCP server if context needed.
- **Feature flags**: driver adapters live behind configuration (`prisma.config.ts` with `engine: 'classic' | 'js'`). Schema engine CLI accepts `--datasource` JSON payload – reuse the structure from commit `34b5a69…`.
- **Graphviz (`dot`)**: optional but useful for rendering query graphs (required if `RENDER_DOT_TO_PNG` set in QC tests/playground).
- **Node.js & pnpm**: required to build the driver adapters kit (`make build-driver-adapters-kit-qc`) and run the QC interpreter harness.
- **Docker**: used for local DBs via `docker-compose.yml`; make targets (`make dev-postgres15`, `make start-mongo6`, etc.) orchestrate containers + config files.

---

## 6. Common Gotchas
- Quaint's `quaint-test-setup` dev-dependency enables `quaint/all-native`, so `--no-default-features --features kingbase-mysql-native` does not prevent other native connector test wrappers from compiling. Cargo test filters are substring matches: `on_kingbase_mysql` can also match `test_type_json_kingbase_mysql` because `json_kingbase_mysql` contains that substring. Use a fully qualified generated test name (with `--exact`) or add an explicit `--skip test_type_` when selecting Kingbase query wrappers.
- Kingbase MySQL `JSON_EXTRACT` path arguments are inferred as the binary `jsonpath` type. Binding the MySQL text payload directly makes the first byte (`$`, 36) look like a JSONPATH version and fails with `unsupported jsonpath version number: 36`; build Kingbase queries with JSONPATH metadata and use the driver's `MySqlJsonPath` codec.
- Running prisma-fmt tests after updating diagnostics **without** refreshing expect files will cause failures. Always run with `UPDATE_EXPECT=1`.
- Some fixtures expect **CRLF** endings (`create_missing_block_composite_type_crlf`). Avoid rewriting line endings when not necessary. If Git warns, restore file from `HEAD`.
- Integration tests bail with “Missing TEST_DATABASE_URL”. Set env vars or skip running them locally.
- `TestApi` inside `sql-migration-tests` exposes `new_engine_with_connection_strings`; use it to pass overrides.
- When touching overrides, update both PSL and schema-engine sides; they share assumptions about spans and optional URLs.
- The connector test kit relies on code from the main repo. Keep it in sync when you change request/response shapes or query plan types.
- MongoDB currently has no QC driver adapter; related tests are skipped. Avoid surprising regressions until an adapter lands.

---

## 7. Useful Commands & Snippets
- Show diff for specific file:
  `git diff path/to/file.rs`
- Re-run single Rust test:
  `cargo test -p prisma-fmt validate::tests::validate_direct_url_direct_empty -- --nocapture`
- Search for legacy attributes:
  `rg "directUrl"`, `rg "shadowDatabaseUrl"`
- Build schema-engine CLI:
  `cargo build -p schema-engine-cli`
- Build query compiler Wasm:
  `make build-qc-wasm`
- Build driver adapters kit for QC:
  `make build-driver-adapters-kit-qc`
- Query compiler playground (generate plan + graph):
  `cargo run -p query-compiler-playground`

Prefer using Makefile targets that take care of setting up the environment correctly or running prerequisite commands.

---

## 8. Open Themes / Future Tasks
- Removing `url` from PSL will be a follow-up; expect similar pattern (PSL error + override path).
- Scrub remaining `query-engine` terminology and unused scaffolding now that the native engine is gone.
- Additional schema-engine tests may need migration to the new override helper.
- Documentation updates (internal + public) should mirror code changes; check when editing diagnostics to keep docs consistent.
- Query compiler MongoDB support: implement translation path + driver adapters, update tests once ready.
- Post-QE cleanup: strip no-longer-needed feature flags/branches in shared crates (`query_core`, `query_structure`), simplify driver adapter plumbing.

---

## 9. External References
- Prisma Config (`prisma.config.ts`) implementation lives in the main Prisma repo (`@prisma/config` package).
- Linear roadmap items for Prisma 7 (Breaking Changes, New Features) hold context.
- Commit `34b5a69…` – canonical example for datasource override wiring.
- Connector test kit guide: `query-engine/connector-test-kit-rs/README.md`.
- QC playground usage: `query-compiler/query-compiler-playground/`.
- QC harness in Prisma repo: `packages/cli/src/__tests__/queryCompiler/qc-test-runner.ts` (mirrors QE behavior).

---

**When modifying anything involving diagnostics or fixtures:** run relevant tests, refresh expectations, and ensure Git diffs are readable (no accidental CRLF/encoding swaps). Keep this file updated whenever we discover new traps.

### Kingbase MySQL migration-test notes

- `sql-migration-tests` must create Kingbase MySQL databases with
  `TestApiArgs::create_kingbase_mysql_database()` and construct the engine with
  `SqlSchemaConnector::new_kingbase_mysql()`. It cannot use the MySQL test
  database helper or connector constructor.
- A Kingbase renderer cannot wholly delegate `MysqlRenderer`: MySQL's create
  table SQL includes `DEFAULT CHARACTER SET utf8mb4 COLLATE ...`, which
  Kingbase rejects, and MySQL renderer downcasts native types to `MySqlType`.
  Render tables/columns/alterations with `KingbaseMySqlType` instead.
- Kingbase accepts MySQL-compatible integer type names but not the `UNSIGNED`
  modifier. Render Kingbase unsigned native types as their signed SQL names and
  make the schema differ consider signed/unsigned pairs equivalent, otherwise
  every schema push reports drift after introspection.
- The generic `apply_migrations::migrations_should_fail_when_the_script_is_invalid`
  test also runs on Kingbase. Kingbase returns PostgreSQL SQLSTATE `42601`, but
  its parser message is localized; assert the stable `ERROR:` prefix rather than
  an English error string.
- `sql-migration-tests` now has 20 dedicated `migrations::kingbase_mysql` tests,
  plus Kingbase branches in `apply_migrations`, `create_migration`, `errors`,
  and `existing_data`. The full `migration_tests` binary passes 1034 tests
  (1 intentionally ignored) against the Kingbase URL.
- Kingbase information_schema adds `::varchar`/`::numeric` casts to literal
  defaults; the Kingbase describer strips those catalog decorations. A reported
  datetime precision of `0` is treated as equivalent to unspecified precision.
- Kingbase primary-key constraints are physically named `<table>_pkey`, while
  the MySQL-compatible describer keeps the primary-key name empty. The Kingbase
  renderer falls back to `<table>_pkey` when dropping such a primary key.
- MySQL-only migration scenarios that Kingbase cannot express (descending
  indexes, selected schema-filter fixtures, named-PK rebuilds and some identity
  rebuilds) are explicitly excluded with `KingbaseMysql`; they are not silently
  reported as passing.

### Kingbase Oracle PSL notes

- Kingbase Oracle mode must be verified against both the Oracle compatibility baseline and the
  actual Kingbase instance. The tested V009R001C010 instance accepts `VARCHAR2(4000)` with character
  length semantics, while Kingbase's `VARCHAR2(*)` is a non-Oracle extension and is not the default.
- The server warns and normalizes `NUMBER(2,3)` to `NUMBER(3,3)`, and warns and reduces
  `TIMESTAMP(9)` to precision 6. PSL rejects scale greater than precision and timestamp precision
  greater than 6 to prevent schema drift.
- The default Oracle-mode installation exposes `kdb_oracle_datatype` but does not install the
  optional `kdb_raw` extension. Do not advertise `RAW` or `LONG RAW` as built-in native types until
  extension configuration and prerequisite validation are implemented.
- On the tested instance, `BLOB`, `CLOB`, and `NCLOB` can back unique constraints, but JSON and XML
  have no default B-tree operator class. Key validation must also resolve the default `Json -> JSON`
  mapping rather than checking only explicit `@db.Json` annotations.
- Keep `kingbase-oracle://` as a distinct public URL scheme and normalize it to the driver's
  `kingbase://` scheme only inside the URL-to-driver configuration boundary. The wire protocol does
  not justify reporting the connection as PostgreSQL or Kingbase MySQL.
- The tested Oracle-mode instance uses `public` as `current_schema()` and listens on a locally
  configured port 54325. The connector default remains Kingbase's 54321; deployment-specific ports
  must stay explicit in the URL. New Oracle-mode TLS URLs default to `sslaccept=strict`.
- A new `SqlFamily` variant must compile both as a standalone Quaint feature and under Cargo feature
  unification. Downstream exhaustive matches must return an explicit unsupported-family error until
  the dialect visitor exists; never route Oracle-mode SQL through the PostgreSQL visitor merely to compile.
- Match Kingbase URL schemes exactly. A broad `starts_with("kingbase")` sends `kingbase-oracle://`
  through the MySQL-mode connector. URL wrapper `Debug` implementations must omit the database password
  and keep TLS identity passwords hidden.
- Keep Kingbase Oracle native conversion and error handling under `connector/kingbase_oracle`; do not move
  the established `connector/kingbase_mysql/native` files into a shared directory merely because both modes
  use the same wire driver. Protocol reuse does not imply shared type semantics.
- Kingbase Oracle scalar binds use Oracle-compatible target types (`NUMBER`, `VARCHAR`, `BLOB`, and temporal
  types). The tested instance accepts PostgreSQL-wire `$1` placeholders. Cover single-connection and pooled
  behavior through the integration-test matrix; do not add connector-local `#[ignore]` database tests that are
  skipped by the normal test command.
- Kingbase Oracle AST SQL is rendered by the independent `visitor::KingbaseOracle`, not by the PostgreSQL visitor.
  It uses `OFFSET … ROWS FETCH NEXT … ROWS ONLY`, `DEFAULT VALUES`, `RETURNING`, `NUMBER`-compatible Boolean
  literals (`1`/`0`) and `VARCHAR2` stringification. Oracle SQL/JSON functions operate on `JSONB` in the tested
  instance even when the public column type is `JSON`; the visitor must cast JSON expressions to `JSONB`, including
  both operands of JSON equality/inequality, bind JSON values as `Type::JSONB`, and use
  `JSON_VALUE` / `JSON_QUERY` / `JSON_ARRAYAGG` / `JSON_OBJECT` with JSONB results. `JSON_VALUE` only handles
  scalar values: `JsonUnquote` must use the PostgreSQL-compatible `#>> ARRAY[]::text[]` operation on a JSONB cast,
  so object and array values retain their JSON text while JSON strings are unquoted.
  `TimestampTz` and `TimestampLocalTz` must bind as `Type::TIMESTAMPTZ`, not plain `TIMESTAMP`, so non-UTC sessions
  preserve the instant. Oracle `MERGE` now implements `OnConflict::DoNothing` and an `OnConflict::Update` that does
  not modify its conflict columns. Kingbase rejects `MERGE ... RETURNING` and updating a column named by the `ON`
  condition, so those cases must fail before SQL is sent. SQL array parameters remain explicit unsupported errors:
  although the server accepts PostgreSQL array syntax, the Oracle schema describer and the Oracle visitor do not
  implement Prisma scalar-list schema or parameter semantics. Native full-text filtering uses the verified
  `to_tsvector(concat_ws(...)) @@ to_tsquery(...)` / `ts_rank(...)` form without a Prisma-managed full-text index;
  do not advertise `FullTextIndex` until its fixed text-configuration lifecycle is implemented.
- Keep real-connection Oracle type coverage in `quaint/src/tests/types/kingbase_oracle.rs`; do not reuse the
  MySQL or PostgreSQL type suites. The Oracle-mode test API creates an `INTEGER GENERATED BY DEFAULT AS IDENTITY`
  key because Kingbase rejects identity declarations on `NUMBER`; `NCHAR(n)` values are fixed-width and return
  server-added trailing spaces, which the tests must assert as an input/output conversion.
- Prisma `Int`/`BigInt` fields in Oracle mode default to `NUMBER(10,0)`/`NUMBER(19,0)`. Do not render their
  `autoincrement()` defaults as `SERIAL`/`BIGSERIAL`: those aliases create PostgreSQL integer types and make a
  normal Oracle `NUMBER` relation scalar incompatible with its referenced key. Keep the `NUMBER` type and create
  an explicit owned sequence plus `nextval()` default; migration coverage must include both Int and BigInt
  autoincrementing primary keys referenced by foreign keys.
- The tested Oracle-mode server accepts `FLOAT(p)` only for `1 <= p <= 53`, despite Oracle documentation commonly
  allowing 126. PSL must reject `Float(54+)` so migrations cannot generate server-rejected DDL. `String @db.Uuid`
  values must bind as PostgreSQL-wire `UUID` after parsing the string, not as `VARCHAR`. `TIMESTAMP WITH LOCAL TIME
  ZONE` is returned as wire `TIMESTAMP` (OID 1114) localized to the session time zone; every Oracle connection must
  initialize `SET TIME ZONE 'UTC'` before decoding it as a Prisma UTC DateTime.
- Register Kingbase Oracle in `quaint-test-setup::connector_names()` so compatible `quaint/src/tests/query.rs` and
  `query/error.rs` cases run through `#[test_each_connector]`. Do not register it by pretending it has every generic
  capability: Oracle `MERGE` covers the shared `ON CONFLICT DO NOTHING` cases and safe non-key update cases, while
  `MERGE RETURNING` and updates of conflict columns have dedicated rejection cases; SQL arrays remain explicitly
  rejected, while full-text query filters are supported without a Prisma-managed full-text index. Oracle supports
  only `READ COMMITTED` and `SERIALIZABLE` transaction isolation
  levels. Its `NUMBER` CTE literals decode as `Numeric`, `REAL` division as `Float`, and multi-expression
  concatenation must render with `||`, not the unavailable `sys.concat` function.
- The local Kingbase MySQL test instance on port 54321 must use the `prisma` database for the full Quaint matrix.
  That database has the MySQL-compatible JSON functions used by the visitor and passes all 147 Kingbase MySQL tests;
  `test_connect` on the same instance lacks `JSON_EXTRACT`, `sys.JSON_CONTAINS`, and the required JSON path operator,
  causing 11 environment-specific JSON failures with the same code.
- `sql-schema-describer` integration tests share one Test API that compiles the SQLite in-memory branch even when a
  test filter selects Kingbase Oracle. Run its Oracle describer suite with `--all-features`; enabling only
  `kingbase-oracle-native` leaves `Quaint::new_in_memory()` unavailable at compile time.
- In the tested Kingbase Oracle catalog, `NCHAR`/`NVARCHAR2` are exposed as `bpchar`/`varchar`, and
  `TIMESTAMP WITH LOCAL TIME ZONE` as `timestamp`. The describer must therefore emit the canonical
  `Char`/`VarChar2`/`Timestamp` forms rather than infer source spellings the catalog no longer retains.
- `sql-introspection-tests` must create an isolated Oracle database with
  `TestApiArgs::create_kingbase_oracle_database()` and construct `SqlSchemaConnector::new_kingbase_oracle()`.
  `barrel` has no Oracle SQL variant, so Kingbase Oracle runs only explicitly tagged
  `tags(KingbaseOracle)` tests whose DDL and expected PSL have been verified against Oracle mode; do not run
  generic Barrel-based suites by pretending they are PostgreSQL tests.
- Kingbase Oracle's PostgreSQL-compatible catalog can expose extension-owned views (for example
  `sys_stat_statements`). Filter `pg_depend.deptype = 'e'` view members before rendering Prisma views, so
  `db pull` returns user views only.
- Kingbase Oracle schema calculation creates Prisma implicit many-to-many join tables with a primary key on
  `(A, B)`. Its introspection flavour must return `true` from `uses_pk_in_m2m_join_tables()`; otherwise a
  `"_ModelAToModelB"` table is incorrectly emitted as an explicit model instead of being restored as an
  implicit many-to-many relation.
