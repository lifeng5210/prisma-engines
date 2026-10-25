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

- Remaining repairs COMPLETE locally (2026-10-25): backticks escaped in shared MySQL Quoted/sql-ddl/visitor incl aliases. ALL new non-PK indexes use table/logical-name FNV64 physical names plus prisma:logical-index: comments; describer retains logical/actual/ownership independently incl external table rename. Small unsigned integers use validated uint4 domains; C010 rejects AUTO_INCREMENT on domains/uint4 and identity CHECK, so auto columns use INTEGER plus AFTER INSERT/UPDATE guards. DO atomically replaces/validates guards, failed narrowing retains values/guard; shared INTEGER auto range changes preserve PK. Use selected/current user schema, never hardcode public. Boolean UnsignedTinyInt stays BOOLEAN. Dynamic fulltext params use PRISMA_MYSQL_BOOLEAN_QUERY and bind-time Boolean Mode subset; unsupported modifiers explicitly rejected. Final 608 functions pass: 62 migration/native,66 introspection,17 describer,15 connector incl forced rebuild real DB10 configs,48 PSL,16 DDL,219 visitors,154 adapter,11 actual Client. 157 DB+451 unit;121 Client matrix skips excluded. Binaryen105 breaks externref growth; checksum-verified official133 fixes packaging. QC fast/small+schema Wasm refreshed in installed fork packages; native release rebuilt and copied into workspace engines; internals/adapter/Client/generators/CLI rebuilt. No npm publication. 150 new DBs removed, original444 names/OIDs preserved, unknown0; host server KEPT RUNNING. Strict clippy all-native still fails two preexisting Oracle/shared diagnostics; do not claim clean. USER EXPLICITLY DECLINED DRIVER PUSH: keep da927b91 exact rev and Cargo.lock local, no remote retry/publication; fresh environments cannot fetch yet. See docs/kingbase-mysql-remaining-fixes.md and results JSON. Preserve existing record/ and Oracle notes.

- SetDefault real-DB acceptance (2026-10-25): the three dedicated tests now execute and pass on V009R001C010 MySQL, zero skipped/ignored. Both ON DELETE and ON UPDATE reset required FK values to a matching default parent, nullable/no-explicit-default values become NULL, missing default parents reject both actions while preserving original parent/child rows, adding the parent permits retry, and pushes are idempotent. Initial real run was 2 pass/1 fail because the negative test incorrectly expected Client P2003 from Schema Engine raw_cmd. Shared render_quaint_error does not map ForeignKeyConstraintViolation to a known error here; that wrapper returns the original message. Corrected only the test to check the native Quaint ForeignKeyConstraintViolation plus original SQLSTATE 23503, retaining data checks, rather than accepting error_code=None or changing production mappings. Final three pass; previous 48 PSL/14 connector passes are historical, not rerun here. Two rounds each created/removed three DBs by inspected name/OID (six DB instances, three names) against baseline444; all originals preserved and host instance left running. See prisma-v7/docs/kingbase-mysql-set-default-live-results.json. Release/Wasm not rebuilt.

- Temporal DECIMAL real-DB acceptance (2026-10-25, after license restoration): all three new migration regressions actually execute and pass on KingbaseES V009R001C010 database_mode=mysql, zero ignored/internal skips. Four temporal families preserve the existing integer numeric encoding and NULL; insufficient-capacity required columns with data are blocked with force both off/on; empty required columns rebuild; nullable columns warn/unforced preserve the original value and forced rebuilding explicitly loses it to NULL while retaining row/id. Subsequent pushes are no-ops. Three new DBs removed by inspected name/OID against a fresh 444-DB baseline, all originals preserved. Server was already running in host namespace and is left running. Sandbox PID namespace can falsely make sys_ctl status report no process; use host-level status before assuming stopped or removing PID locks. No source/test edits or release/Wasm rebuild for this run. This supersedes the date fix's prior license-blocked status only; SetDefault and index-rebuild real-DB follow-ups remain unexecuted in this date-only run, with license now restored. See prisma-v7/docs/kingbase-mysql-temporal-decimal-live-results.json.

- SetDefault warning repair (2026-10-25): removed Kingbase MySQL model validation that copied MySQL's unsupported-SetDefault warning, plus the now-unused connector-local validations module. V9R3C11 MySQL CREATE TABLE explicitly documents SET DEFAULT for both ON DELETE and ON UPDATE and requires a matching parent for non-NULL defaults; historical C010 ON DELETE probe is successful, ON UPDATE is not newly DB-verified here. Native foreign-key actions and renderer already support SetDefault; relationMode=prisma still rejects it and ordinary MySQL retains its warning. New tests explicitly inspect diagnostics, since assert_valid alone ignores warnings. Same new test fails against old production code (2 pass/1 fail with Kingbase warning); repaired Kingbase PSL group passes 48 and connector group 14 (one prior DB test ignored). New SQL-rendering regression checks actual PSL diff emits both SET DEFAULT clauses. Three standard DB regressions compile/list but have NOT executed: license expiry remains unchanged; no startup/DDL/new DB in this turn. psl-core all-feature strict Clippy and changed-file rustfmt pass; full workspace Clippy is not credited. See prisma-v7/docs/kingbase-mysql-set-default-fix.md. Release/Wasm not rebuilt.

- Temporal-to-DECIMAL capacity repair (2026-10-25): datetime_numeric_cast_supported now counts Decimal(p,s) integer capacity as p.saturating_sub(s), rather than total precision. Date/Time/DateTime/Timestamp need 8/6/14/14 integer digits for the existing to_char representation. Capacity-sufficient changes retain RiskyCast and ALTER COLUMN ... USING; insufficient capacity is NotCastable and enters existing drop/recreate/destructive checks, NOT a data-preserving cast. Pure diagnostics block required/no-default columns conservatively; nullable/defaulted columns warn of loss. Runtime empty/nonempty inspection is not credited without DB execution. New boundary regression fails against the old implementation on Date -> Decimal(9,2); repaired connector unit group passed 13, one unrelated native index regression ignored. Three new standard migration tests compile, covering four temporal sources, NULL/results/no-op, empty vs populated required columns (force cannot override), and nullable loss warning/forced value loss. They have NOT run: C010 license expiry remains the blocker from the preceding startup; no DB created or started in this turn. No release/Wasm rebuild. See prisma-v7/docs/kingbase-mysql-temporal-decimal-fix.md.

- Index rebuild fallback repair (2026-10-25): KingbaseMysqlRenderer::render_drop_and_recreate_index now creates via its own render_create_index and deletes via its ownership-aware render_drop_index; ordinary/standalone-unique indexes use DROP INDEX without ON, and constraint-owned unique indexes use the actual owning constraint. Ordinary rebuild keeps create-before-drop; fulltext retains its existing drop-before-create. New tests force a SqlMigrationStep::RedefineIndex through the real script renderer rather than relying on can_rename_index=true; original fallback fails the exact regression with DROP INDEX old_index ON items. Fixed Kingbase connector unit group passes 9 tests, one native-DB regression is explicitly ignored pending execution. The DB regression compiles and covers five index configurations under ANSI_QUOTES on/off, row retention, uniqueness and subsequent no-op diff. Actual C010 startup on 2026-10-25 fails FATAL License file expired; no new DB was created and no real-DB results are credited for this repair. Do not bypass license expiry or count previous 40/66/17 passes as this run. An exploratory literal-backtick identifier also exposes a preexisting shared Quoted::Backticks/sql-ddl escaping gap; current rebuild regressions use ordinary names and this repair does not claim to solve that separate quoting issue. See prisma-v7/docs/kingbase-mysql-index-rebuild-fix.md. Driver publication remains separately pending user authorization; do not retry pushing. Changed-file rustfmt and diff check pass; strict Clippy is not green on rustc 1.97.1: dependency lint stops on five preexisting Quaint warnings, and --no-deps reports four preexisting warnings outside the changed rebuild block/new tests.

- Driver sys-catalog repair (2026-10-24): local rust-kingbase commit da927b91468c446c0527d85461d035c0e5dbc2b0 on codex/kingbase-mysql-sys-catalog now routes MySQL type/enum/composite queries and their fallbacks through sys_catalog.sys_type/sys_range/sys_namespace/sys_enum/sys_attribute. Startup detects mode with SHOW database_mode before choosing sys_catalog.sys_settings for MySQL; unknown setting (42704) preserves ordinary PostgreSQL support, and optional-settings permission errors retain the detected mode. Other modes retain their PG queries. A restricted-role regression revokes PG catalog reads only in an isolated test DB transaction: original prepare.rs fails on pg_type (42501), repaired metadata/enum/SET/domain/range/array/composite/BIT succeeds. Driver tests pass 10 library plus 5 catalog/startup cases (one real DB, four wire-protocol simulations); clippy --tests -D warnings passes. Prisma Cargo.toml pins all three Kingbase aliases to this exact rev, Cargo.lock updates all four driver packages, cargo tree and cached-source hashes confirm actual usage without path overrides, and --locked --offline suites pass 40 migrations, 66 introspection, 17 describer cases. cargo update initially fails on an unrelated existing cuid/getrandom js-feature resolution; the four source-only lock entries were updated directly and validated with locked builds/tests. IMPORTANT: commit is local only. Automatic approval rejected pushing the new branch to lifeng5210/rust-kingbase because specific code-publication authorization was missing; do not retry or publish indirectly without user authorization. Fresh environments cannot fetch the new revision until that publication is approved/completed. Preserve other PG type identity strings and non-MySQL mode behavior; they are not SQL catalog reads. Fourteen new DBs cleaned against baseline444; server restored stopped. See prisma-v7/docs/kingbase-mysql-driver-sys-catalog.md and JSON evidence. No release/Wasm rebuild.

- Test-expectation audit (2026-10-24): 24 existing PSL tests incorrectly reject full-column ordinary/unique/primary keys for the eight TEXT/BLOB families. All 48 single/compound C010 configurations execute; duplicates confirm normal-index acceptance and unique/PK rejection (23505). Introspecting 24 single-key models produces 21 validation errors (implicit LongBlob bypasses explicit-native checks). Bytes @id passes while Bytes @id @db.LongBlob fails despite identical default type. Four further test functions copy CHAR/DECIMAL limits contradicted by V9R3C11 MySQL docs and C010: CHAR(256), CHAR/VARCHAR(655350), DECIMAL(66,20), DECIMAL(44,33), DECIMAL(2,4) execute; the last normalizes to (4,4). The VarChar negative fixture actually uses Char. A parameter introspection round trip fails with four errors. Existing 43 PSL tests still pass; independent counterexamples fail. Audit-only: production validation remains unchanged. Evidence/report: sibling prisma-v7/docs/kingbase-mysql-test-expectation-audit.md. Full-column keys differ from unsupported prefix syntax; short-value probes do not establish unlimited key size.
- Test-expectation repair (2026-10-24): The preceding audit findings are now fixed: remove the TEXT/BLOB full-column key blacklist (prefix syntax stays rejected), allow CHAR/VARCHAR through 10485760 and DECIMAL precision 1..1000 / scale 0..1000, and normalize valid Decimal(p,s) to (max(p,s),s) during native-type parsing. Keep invalid arguments unnormalized so precision zero is rejected even for Decimal(0,4). C010 accepts CHAR(0)/VARCHAR(0), unlike the documented CHAR minimum of 1; preserve and explicitly test that existing zero-length behavior. Replace 24 invalid negative key expectations with positive cases, correct the VarChar fixture, and use real upper-bound/zero-precision failures. Exact native-field assertions retain @db.LongText; the old substring check masked that omission in its expectation. Final Kingbase suites pass 45 PSL, 40 migration and 66 introspection tests (151); psl-core clippy -D warnings and changed-file rustfmt pass. Tests cover 48 generated single/compound key cases, duplicate enforcement, implicit/explicit LongBlob equivalence, large decimal value preservation, canonical decimal no-op pushes, introspection validation and shadow replay. A broader PSL run reports 548 passed / 574 failed, all 574 snapshot differences disappear after removing ANSI color codes; do not claim that full run passed or update those unrelated snapshots. Nine newly created DBs were removed against the 444-name/OID baseline; server restored stopped. Source tests only; release/Wasm artifacts were not rebuilt. See docs/kingbase-mysql-test-expectation-audit.md in prisma-v7 and the separate fix-results JSON.
- The DESC/prefix/visitor follow-up passed 37 dedicated migration/query, 17 describer and 64 introspection tests on C010 (118 real-DB cases), plus 43 Kingbase PSL and 11 visitor unit tests (54), on 2026-10-24. PSL datamodel tests require --all-features (without it other connector imports do not compile). Prefix capability removal also converts six old positive TEXT/BLOB-prefix tests into rejection tests and updates 24 misleading length-advice snapshots. The visitor now shares lexical skipping for function discovery, bind positions and parentheses, protecting quoted identifiers/literals, comments, dollar strings, qualified functions, identifier boundaries and Unicode. Real-DB regression selects function-shaped names including SUM(?) with a genuine bound predicate. Descending unique creation/removal has calculated-schema and shadow-replay coverage. Driver PG metadata dependencies are unchanged in this follow-up.
- The V9R3C11 sys_am/sys_index documentation provides `sys_index_column_has_property`; C010 MySQL confirms `sys_catalog.sys_index_column_has_property(index_oid, ordinal, 'asc'/'desc')` returns the actual order using one-based ordinals. Prefer this documented interface over hardcoded indoption bits. `descending_indexes_are_introspected` previously asserted that DESC disappeared, so its expectation must change together with the describer. A fresh temporary-table prefix probe still rejects `CREATE INDEX ... (a(8))` with 42883; disabling IndexColumnLengthPrefixing must also remove the connector's advice to add length for TEXT/BLOB keys.
- C010 temporary-table probes accept inline ordinary INDEX(... DESC), but reject inline UNIQUE INDEX(... DESC) with 42601, although standalone CREATE UNIQUE INDEX ... DESC works. Descending unique indexes must be created separately, like fulltext indexes. Keep CREATE TABLE filtering, new-table index planning, and calculated unique_constraint_names consistent through the shared `index_is_created_separately` predicate; such a standalone unique index must be removed using DROP INDEX, including calculated-schema diffs and shadow replay. Ascending unique constraints retain their previous inline ownership.
- BLOB-to-BIT migration now remains RiskyCast and renders a composed USING expression, rather than a missing direct cast or destructive NotCastable fallback. It reads the signed ASCII decimal prefix using encode(bytea,'escape') plus regex (so NUL/non-UTF-8 bytes do not cause decoding errors), removes leading zeros, bounds numeric parsing to 21 characters, saturates to signed bigint, and casts to BIT. Widths below 64 reject overflow under STRICT_ALL_TABLES / STRICT_TRANS_TABLES and saturate to all-one bits otherwise; NULL stays NULL. These semantics were compared with native MySQL 8.0.46, not an arbitrary byte-truncation policy. C010 supports STRICT_ALL_TABLES but rejects setting STRICT_TRANS_TABLES; the latter renderer branch is not locally exercised.
- The BLOB-to-BIT follow-up passed all 33 dedicated Kingbase MySQL migration tests on C010 (2026-10-24), including seven new cases: four BLOB families to BIT(1/8/16/64) with actual values, empty required columns, strict overflow preserving source data/schema, and non-strict saturation. Tests check RiskyCast warnings block an unforced push, generated SQL has no DROP COLUMN, NULL/raw bytes/NUL/decimal prefixes/long prefixes/signed limits convert as expected, row counts remain, and subsequent pushes are no-ops. This is source/Cargo-test validation; release CLI/Wasm artifacts were not rebuilt.
- A further C010 MySQL probe on 2026-10-24 tested both native MySQL `MODIFY v BIT(8) NULL` and `ALTER COLUMN v TYPE BIT(8) USING (v::BIT(8))` for all four BLOB families, each with an empty table, NULL, and inserted text '1'. All 24 cases reached ALTER and failed: MODIFY with 42804 (no automatic cast), direct USING with 42846 (no explicit cast). Each case used a transaction-local temporary table and rolled back. This rules out switching to plain MODIFY on this instance; it does not rule out a deliberately defined multistep data conversion. Classifying these pairs NotCastable would use the existing drop/recreate planning and destructive checks; it must not be described as preserving existing column data.
- On 2026-10-24, an isolated MySQL 8.0.46 server successfully executed `ALTER TABLE t MODIFY v BIT(8) NULL` for TINYBLOB/BLOB/MEDIUMBLOB/LONGBLOB with empty tables, `X'01'`, and `X'FFFF'` under `STRICT_ALL_TABLES` (12 cases). Nonempty values became `HEX(v)=0`, so this is executable conversion evidence, not lossless-conversion evidence. C010 Kingbase's direct BLOB-to-BIT USING cast instead fails even on an empty table (42846). Do not generalize either result to every server version or value.
- Kingbase's MySQL compatibility mode should follow the existing MySQL connector's type semantics and test expectations. The underlying database driver is different, but that alone must not change the exposed `ColumnType` behavior; only genuine Kingbase wire-codec differences should be handled in the Kingbase connector.
- Before making a Kingbase compatibility change, verify the behavior against Kingbase's official documentation or the installed Kingbase extension definitions. Use that evidence to distinguish a SQL dialect difference, a server capability difference, and a wire-codec difference.
- The V9R3C11 MySQL-mode DDL manual explicitly documents `DROP INDEX name` without `ON table`, `DROP CONSTRAINT`, and `ALTER COLUMN ... TYPE ... USING`. PG-shaped SQL alone is not evidence of a missing adaptation. Sort/prefix metadata remains an audit finding. Unique-index ownership is now retained in `kingbase_mysql::KingbaseMysqlSchemaExt::unique_constraint_names`, using the official sys_constraint conindid/conrelid/contype='u' fields; foreign-key rows must not be mistaken for ownership. Deletion uses the actual owning constraint name or standalone DROP INDEX, with MySQL identifier quoting.
- Kingbase MySQL calculated PSL schemas need the same connector-data type as described schemas: newly created tables render inline unique constraints, so `push_connector_data()` marks those unique indexes as owned. Database introspection instead reads actual catalog ownership; standalone unique indexes added to existing tables must stay standalone in metadata. Keep both schema-to-schema diff and shadow-database migration replay covered when changing this representation.
- On 2026-10-24 the unique-index ownership fix passed 21 dedicated migration, 15 dedicated describer, and 64 dedicated introspection tests on C010 MySQL (100 total). Six new regressions cover add/remove @unique, inline constraint removal, mixed index deletion, calculated-schema diffs, migration-file/shadow replay, and catalog ownership including compound indexes and foreign-key references. The new add/remove test reproduced the missing-constraint error before the fix. The broad migration entry's preexisting applicable cases also passed; its new shadow-replay test initially needed ordered migration names and then passed in the dedicated rerun. Do not count internally skipped wrappers as database coverage.
- Kingbase MySQL index rename and primary-key removal now also use the renderer's MySQL `self.quote()` instead of `Quoted::postgres_ident`, so the dedicated renderer no longer assumes ANSI_QUOTES for identifiers. The ANSI_QUOTES-off regression reproduced a syntax error at the double-quoted PK name before this change. Verify the actual mode flag when testing: C010 can display an empty sql_mode as `""`, so comparing SHOW's output to an empty string is not a reliable flag assertion.
- The quoting follow-up passed all 23 dedicated Kingbase MySQL migration tests on C010. Two new cases set and verify ANSI_QUOTES on/off, execute the generated index-rename and primary-key-change script, assert the new PK/index names and retained row, and require the next schema push to have no steps. The separate PK-name inference issue was fixed subsequently as described below.
- A subsequent C010 MySQL temporary-table probe explicitly tested MySQL `ALTER TABLE ... DROP PRIMARY KEY`: both default and custom-named PK cases fail with SQLSTATE 42601 at PRIMARY. `DROP CONSTRAINT custom_pk` succeeds and removes the PK. The V9R3C11 MySQL-mode ALTER TABLE manual documents DROP CONSTRAINT and a PK-rebuild example using it. Thus the alternative SQL has evidence; the missing actual PK-name metadata has now been fixed. Do not generalize this syntax result to all Kingbase versions.
- The PK-name fix stores actual conname in `KingbaseMysqlSchemaExt::primary_key_constraint_names`, keyed by primary index ID, joining sys_constraint with contype IN ('p','u') while keeping PK/unique maps separate and excluding foreign-key references. Keep the logical primary-key index name empty for MySQL compatibility; changing that logical name can trigger unwanted rename/diff behavior. PK removal rendering reads the stored physical name without a table-name fallback. Calculated schemas initialize default names with the connector's identifier limit; custom PK names, renamed tables, and execution of a calculated-schema migration with a 63-byte table name now have real-DB regression coverage.
- The PK-name follow-up passed 26 dedicated migration, 16 dedicated describer, and 64 dedicated introspection tests on C010 MySQL (106 total). Four new cases cover custom PK names, a renamed table retaining its old PK name, a calculated-schema migration for a 63-byte table name, and physical-name metadata preservation. The custom-name migration reproduced the guessed-name missing-constraint error before the fix. All three migration cases check the changed PK, preserved row, and a subsequent no-op push.
- The V9R3C11 guide documents `pg_*` views in HTML section 9.3 (PDF 10.3), but the preceding system-view overview classifies them under PG mode; MySQL mode lists KES system views plus MySQL-compatible views. The V9R3 Ruby FAQ also supplies manually created `pg_get_expr` and advisory-lock wrappers. Do not treat a PG chapter or a wrapper example as proof that the alias is built in under MySQL mode. Check mode, schema, and prerequisites as well as fields.
- The sibling Prisma repository's `docs/kingbase-mysql-official-sql-evidence.md` records a seven-PDF review. It found official support for `::`, conversion functions, and LATERAL/WITH ORDINALITY, but no occurrence of the exact `pg_advisory_lock` name; the concepts manual instead uses `sys_advisory_lock`. The documented `attidentity` values are empty/a/d; the extra `i` branch now has local C010 AUTO_INCREMENT evidence (see below). A documentation gap alone does not prove rejection by the server.
- `git blame` attributes the entire MySQL `acquire_lock` method to initial connector commit `07eebefa596dca15826132f80c1ddb836067e3f5` (2026-08-24). Its key 72707369, 10-second timeout, and error template closely match `PostgresConnector::acquire_lock`; the commit does not explain why it kept the pg function name. Treat copying/inheritance as an inference, not documented author intent.
- Kingbase MySQL now calls `sys_advisory_lock(72707369)` and names that function in timeout diagnostics. On 2026-10-24, the local V009R001C010 MySQL instance passed the existing `advisory_locking_works` and `migrations_can_be_created_applied_and_reapplied` migration integration tests after this change. A separate multi-connection probe verified exclusion, cancellable waiting, release on connection close, explicit unlock, and mutual exclusion with the old `pg_advisory_lock` name on this instance. This confirms local compatibility, not all Kingbase versions.
- On V009R001C010 MySQL, `sys_catalog.sys_class` is a view: its regclass OID is 12371, while relation dependencies use classid 1259. Do not replace `'pg_catalog.pg_class'::regclass` with `'sys_catalog.sys_class'::regclass` when changing catalog names. `sys_catalog.sys_get_object_address('view', ARRAY['sys_catalog', 'sys_class'], ARRAY[]::text[])` returns the underlying relation classid without hardcoding a catalog OID. Keep extension-owned-view filtering covered when changing this query.
- The Kingbase MySQL describer now uses `sys_catalog.sys_*` views and `sys_catalog.sys_get_expr`, with `sys_get_object_address` for the dependency classid. On 2026-10-24, 14 dedicated describer tests (including the new `extension_owned_views_are_not_described` regression), 16 dedicated migration tests, and 64 dedicated introspection tests passed on V009R001C010 MySQL. The regression adds an owned view to the existing plpgsql extension only inside its isolated test database; ordinary views must remain visible.
- The 2026-10-24 full SQL audit also inspected the Cargo.lock-pinned rust-kingbase revision `2732da19`: its `tokio-postgres/src/prepare.rs` still queries `pg_catalog.pg_type`, `pg_range`, `pg_namespace`, `pg_enum`, and `pg_attribute`, and connection probing reads `pg_settings`. Changing Prisma's describer does not remove these driver-level dependencies. They work on the local C010 MySQL instance; the official mode-view matrix alone does not guarantee those aliases on every target installation.
- Local C010 MySQL probes confirm `AUTO_INCREMENT` uses `attidentity='i'`; retain that branch. The official V9R3C11 FAQ parameter section 3.6 explicitly documents a default `max_identifier_length` of 63, and this instance reports 63 bytes (64-byte ASCII / 66-byte Chinese identifiers are truncated to 63). These are no longer unverified PG assumptions.
- Full-audit runtime findings on C010: standalone `CREATE UNIQUE INDEX` does not create a unique constraint, so deleting it via `DROP CONSTRAINT` fails; inline `UNIQUE INDEX` does create a constraint and cannot be dropped directly. DESC is stored in `sys_index.indoption` but discarded by the describer; prefix-index syntax is rejected although PSL advertises it. Double-quoted index DDL requires `ANSI_QUOTES`, while backticks work with it disabled. The MySQL `DROP INDEX ... ON table` fallback is rejected, but its RedefineIndex caller is currently bypassed by `can_rename_index() == true`.
- The final cast probe covers 33 configurations (32 native type variants plus Boolean's actual BOOLEAN), 783 admitted pairs: 779 succeed; TinyBlob/Blob/MediumBlob/LongBlob -> Bit fail on an empty table (42846). This checks representative type signatures, not every typmod or value. `INTEGER/INT/BIGINT UNSIGNED` work locally; `TINYINT/SMALLINT/MEDIUMINT UNSIGNED` do not. Do not repeat the old blanket claim that `INT UNSIGNED` is unsupported. ON DELETE SET DEFAULT also executes correctly on C010, contrary to the copied MySQL validation warning.
- Visitor SQL rewrites currently search quoted identifiers as well as function calls: a mapped column named `uuid()` is rendered as `` `CAST(sys_guid() AS text)` ``, and JSON_UNQUOTE/JSON_CONTAINS/JSON_EXTRACT-shaped names are also changed. Treat this as an implementation defect, separate from whether the replacement SQL has official support. Detailed current evidence lives in the sibling repository's `docs/kingbase-mysql-full-audit.md`.
- The Kingbase MySQL compatibility manual documents `information_schema` views for tables, columns, views, key usage, referential constraints, and routines. Treat those as separate describer contracts; metadata definition text is database-native and must not be asserted as byte-for-byte MySQL output.
- `information_schema.referential_constraints.delete_rule` and `.update_rule` are documented as enum columns and arrive through Quaint as `ValueType::Enum`, not `Text`. In Kingbase-only describer SQL, cast them to `TEXT` before using the shared string-to-`ForeignKeyAction` mapping.
- Kingbase MySQL documents `INT` and `INTEGER` as aliases for the MySQL `INT` type. Its catalog can report the latter, so normalize both to `ColumnTypeFamily::Int` / `KingbaseMySqlType::Int` in the Kingbase-only describer.
- Unlike MySQL/InnoDB, Kingbase does not automatically create an index on foreign-key columns. Describer tests must not expect non-unique child-column indexes unless the test DDL creates them explicitly; preserve the actual catalog rather than synthesizing MySQL-only indexes.
- Kingbase's catalog can report a MySQL `DECIMAL` column as `number`, with `numeric(...)` in the full type; normalize `decimal`, `numeric`, and `number` to the Kingbase MySQL decimal family in the describer.
- For Kingbase MySQL `BIT(1)`, `information_schema` may omit `numeric_precision`; determine the width from `full_data_type` before falling back to metadata, so `BIT(1)` remains `Boolean` and `BIT(n > 1)` is `Binary`.
- Kingbase may expose extension-owned views in the same schema as user views (for example `sys_stat_statements`). `sys_depend.deptype = 'e'` denotes an extension member in Kingbase's native catalog, so the Kingbase MySQL describer filters those rows from user-view introspection.
- Kingbase MySQL full-text rendering uses a standalone GIN index over immutable `to_tsvector('simple', textcat(COALESCE(...), ...))`, and introspection identifies that exact expression. The current visitor rewrites `MATCH ... AGAINST (... IN BOOLEAN MODE)` to `to_tsquery('simple', ...)` / `ts_rank`: concrete text parameters receive a partial Boolean Mode conversion, but opaque Query Compiler runtime parameters currently require PG-style tsquery input. The Client functional matrix explicitly tests that contract; it is not full MySQL Boolean Mode compatibility. In MySQL mode `||` is logical OR rather than string concatenation, so never use it in the document expression.
- Kingbase MySQL accepts `sslmode` through the PostgreSQL-wire driver. Parse Quaint's `sslaccept`: the default remains `accept_invalid_certs` for parity with MySQL/PostgreSQL, while `sslaccept=strict` enables certificate and hostname validation. `sslcert`, `sslidentity`, and `sslpassword` configure the native TLS root certificate or client identity.
- KingbaseES defaults to TCP port 54321, whereas `kingbase_tokio_postgres` inherits PostgreSQL's 5432 fallback. When a Kingbase URL omits its port, insert 54321 into the driver URL before parsing it into `Config`; adding a port afterwards merely appends a second value, leaving 5432 selected for the first host.
- Kingbase MySQL SQL rendering shares `SqlFamily::Mysql`, but its PostgreSQL-wire prepared-statement parameter count is a signed 16-bit value. Clamp both native and external Kingbase connection info to 32,767 bind values so batch planning never emits an unexecutable statement.

### Kingbase Oracle Compatibility
- A local V009R001C010 probe confirmed that quoted ASCII all-uppercase identifiers are stored lowercase (`"A"` -> `a`), while mixed-case quoted names are preserved. Migration schema calculation must account for that fold without making all mixed-case names case-insensitive. Oracle foreign keys require a primary/unique constraint; a standalone unique index is not a valid referenced key. Preserve the catalog distinction when dropping indexes versus constraints.
- Oracle stores `''` as NULL (`pg_get_expr` reports `NULL::varchar`), so schema calculation normalizes empty-string defaults to the absent default. Required-string fixtures must insert a nonempty value when testing unrelated behavior. SQL string defaults escape apostrophes by doubling them; the tested instance has `ora_escape_compatible_pg=off`.
- Oracle migration scripts sent as one batch roll back their DDL on a later error. Shared tests that assume partially committed DDL exclude Oracle just as they exclude PostgreSQL; `oracle_failed_migration_rolls_back_ddl_without_schema_drift` verifies the Oracle rollback and migration-history behavior directly.
- Oracle `NUMBER` raw query results are Numeric even for Prisma Int/BigInt. Migration test assertions compare integer values without truncating decimals. Enum parameters use an inferred wire type, and native enum results decode their UTF-8 variant labels as Quaint Enum values.
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
- `reintrospect_force_invalid_config` compares plain diagnostic text after stripping ANSI escapes in its test helper. Diagnostic colors depend on TTY/`NO_COLOR`/`CLICOLOR_FORCE`, not database provider; do not select colored versus plain snapshots by Kingbase tags or change global color state in concurrent tests.
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

- Migration directory timestamps have second precision and directories are replayed lexicographically. Tests creating several migrations in a tight loop must use ordered names such as `01_init`, `02_add_unique`, `03_remove_unique`; unprefixed add_unique can sort before init when timestamps match. Do not add sleeps to make migration ordering pass.
- `sql-migration-tests` must create Kingbase MySQL databases with
  `TestApiArgs::create_kingbase_mysql_database()` and construct the engine with
  `SqlSchemaConnector::new_kingbase_mysql()`. It cannot use the MySQL test
  database helper or connector constructor.
- A Kingbase renderer cannot wholly delegate `MysqlRenderer`: MySQL's create
  table SQL includes `DEFAULT CHARACTER SET utf8mb4 COLLATE ...`, which
  Kingbase rejects, and MySQL renderer downcasts native types to `MySqlType`.
  Render tables/columns/alterations with `KingbaseMySqlType` instead.
- The current Kingbase business-column renderer uses `sys.uint4` for unsigned
  native types up to 32 bits, and `sys.uint8` for UnsignedBigInt. The differ
  considers the aliases sharing uint4 equivalent. Do not render these as signed
  names: that loses their unsigned range. The migration-history table still
  contains `INTEGER UNSIGNED`; verify that separate spelling on the target
  server before assuming all unsigned syntax has been normalized.
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
- Kingbase default primary-key names derive from the table name, but custom
  constraint names and table renames make `<current table>_pkey` unreliable.
  The MySQL-compatible logical PK name stays empty; the actual physical name
  is retained in Kingbase connector metadata and used directly for deletion.
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
  `barrel` has no built-in Oracle variant: the independent test backend in
  `sql-introspection-tests/src/oracle_ddl.rs` renders portable fixtures with Oracle native types and owned sequences.
  Unsupported fixture types fail explicitly. Do not render Oracle fixtures through PostgreSQL's backend or the production renderer under test.
- Oracle now uses normal shared `test_setup::should_skip_test()` tag/exclusion/capability selection; the previous requirement for an explicit Oracle tag is removed. Provider-specific SQL snapshots and native-type assertions still need explicit Oracle expectations. MySQL-mode schema-engine tags contain `KingbaseMysql`, not `Mysql`, and its test-setup capability set is empty. Inspect tags, exclusions, capabilities, and test-body guards before counting shared tests as Kingbase database coverage. Cargo reports a successful early return as a pass; use `--nocapture --test-threads=1` to distinguish `Test skipped` from actual execution.
- A local Oracle introspection run after shared-suite integration executed 120 cases (44 dedicated, 76 shared) successfully, while 572 other-provider/capability entries returned `Test skipped`. These counts describe the current Rust integration binary, not the separate SQL fixtures or the Cargo headline alone. Shared migration eligibility differs from MySQL; do not copy its 229 shared migration / 70 shared introspection count onto Oracle.
- Oracle shared migration integration now passes 291 connector cases (23 dedicated, 268 shared), plus 28 ordinary Rust tests; 744 wrappers skip internally and 1 case is ignored. Module filters match substrings and can repeat names across groups, so deduplicate full test paths and inspect source attributes before counting database coverage. `schema-engine/kingbase-oracle-shared-tests.md` records commands and the verified scope. MySQL regression remains 255 migration connector cases (26 dedicated, 229 shared) and 134 introspection cases (64 dedicated, 70 shared).
- Source inventories must include the MySQL-mode `test_type_tinyint1_kingbase_bool` Quaint case: filtering only `kingbase_mysql` misses it. Existing test binaries may also predate source changes (for example, removed Oracle native ROWID/TINYINT tests), so `target/debug/deps/* --list` alone is not authoritative for the current source.
- A source/registration inventory on 2026-10-16 counted 604 MySQL-mode and 613 Oracle-mode related tests/fixtures across PSL, Quaint, schema-engine, and query compiler/Wasm modules. These are inventory counts, not a fresh full run; the 28 ordinary migration-entry Rust tests are listed separately rather than credited as Kingbase database coverage. Quaint selects 149 per mode at this baseline: the MySQL `kingbase_mysql` name filter selects 148 and needs the separate Boolean TINYINT(1) case; its new DOUBLE-parameter metadata test requires a real connection. Include Oracle describer identifier-folding unit coverage from the newer binary when checking current source registrations.
- Kingbase Oracle's PostgreSQL-compatible catalog can expose extension-owned views (for example
  `sys_stat_statements`). Filter `pg_depend.deptype = 'e'` view members before rendering Prisma views, so
  `db pull` returns user views only.
- Kingbase Oracle schema calculation creates Prisma implicit many-to-many join tables with a primary key on
  `(A, B)`. Its introspection flavour must return `true` from `uses_pk_in_m2m_join_tables()`; otherwise a
  `"_ModelAToModelB"` table is incorrectly emitted as an explicit model instead of being restored as an
  implicit many-to-many relation.
