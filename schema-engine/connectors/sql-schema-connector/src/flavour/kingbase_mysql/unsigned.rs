use sql_schema_describer::walkers::{TableColumnWalker, TableWalker};

use crate::sql_renderer::Quoted;

pub(super) fn column_type(column: TableColumnWalker<'_>) -> Option<String> {
    if column.column_type_family().is_boolean() {
        return None;
    }
    let (name, _) = sql_schema_describer::kingbase_mysql::unsigned_domain_spec(column.column_native_type()?)?;
    if column.is_autoincrement() {
        return Some("INTEGER".to_owned());
    }
    Some(qualified_name(column.table(), name))
}

fn sql_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn qualified_name(table: TableWalker<'_>, name: &str) -> String {
    match table.namespace() {
        Some(namespace) => format!("{}.{}", Quoted::mysql_ident(namespace), Quoted::mysql_ident(name)),
        None => Quoted::mysql_ident(name).to_string(),
    }
}

fn qualified_table(column: TableColumnWalker<'_>) -> String {
    qualified_name(column.table(), column.table().name())
}

pub(super) fn drop_auto_guard(column: TableColumnWalker<'_>) -> Option<String> {
    if !column.is_autoincrement() || column.column_type_family().is_boolean() {
        return None;
    }
    sql_schema_describer::kingbase_mysql::unsigned_domain_spec(column.column_native_type()?)?;
    Some(format!(
        "DROP TRIGGER IF EXISTS __prisma_unsigned_auto ON {}",
        qualified_table(column)
    ))
}

pub(super) fn create_auto_guard(column: TableColumnWalker<'_>) -> Option<String> {
    drop_auto_guard(column)?;
    let (_, maximum) = sql_schema_describer::kingbase_mysql::unsigned_domain_spec(column.column_native_type()?)?;
    let body = format!(
        "BEGIN IF EXISTS (SELECT 1 FROM {} WHERE {} < 0 OR {} > {maximum}) THEN RAISE EXCEPTION 'Value out of range for small unsigned integer' USING ERRCODE='23514'; END IF; END;",
        qualified_table(column),
        Quoted::mysql_ident(column.name()),
        Quoted::mysql_ident(column.name())
    );
    Some(format!(
        "DO {}; CREATE TRIGGER __prisma_unsigned_auto AFTER INSERT OR UPDATE ON {} FOR EACH ROW EXECUTE FUNCTION {}({},'{maximum}')",
        sql_literal(&body),
        qualified_table(column),
        qualified_name(column.table(), "__prisma_unsigned_auto_guard"),
        sql_literal(column.name())
    ))
}

/// Keep type changes, trigger replacement and validation in one transaction,
/// including when called through schema push's per-statement executor.
pub(super) fn atomic(statements: Vec<String>) -> String {
    let body = format!(
        "BEGIN {} END;",
        statements
            .iter()
            .map(|statement| format!("EXECUTE {};", sql_literal(statement)))
            .collect::<Vec<_>>()
            .join(" ")
    );
    format!("DO {}", sql_literal(&body))
}

/// Domains retain the MySQL native type's range and identity while using the
/// supported uint4 wire type. They live in the selected user schema, not sys.
pub(super) fn initialize(table: TableWalker<'_>) -> Vec<String> {
    let specifications: std::collections::BTreeSet<_> = table
        .columns()
        .filter(|column| !column.is_autoincrement() && !column.column_type_family().is_boolean())
        .filter_map(|column| sql_schema_describer::kingbase_mysql::unsigned_domain_spec(column.column_native_type()?))
        .collect();
    let namespace = table
        .namespace()
        .map(sql_literal)
        .unwrap_or_else(|| "current_schema()".to_owned());
    let mut statements: Vec<_> = specifications.into_iter().map(|(name, maximum)| {
        let identity = format!("t.typname = '{name}' AND n.nspname = {namespace}");
        let body = format!(
            "BEGIN IF NOT EXISTS (SELECT 1 FROM sys_catalog.sys_type t JOIN sys_catalog.sys_namespace n ON n.oid=t.typnamespace WHERE {identity}) THEN \
             CREATE DOMAIN {domain} AS sys.uint4 CONSTRAINT {constraint} CHECK (VALUE OPERATOR(sys.<=) {maximum}); \
             ELSIF NOT EXISTS (SELECT 1 FROM sys_catalog.sys_type t JOIN sys_catalog.sys_namespace n ON n.oid=t.typnamespace \
             JOIN sys_catalog.sys_type base ON base.oid=t.typbasetype JOIN sys_catalog.sys_namespace bn ON bn.oid=base.typnamespace \
             WHERE {identity} AND t.typtype='d' AND NOT t.typnotnull AND t.typdefault IS NULL AND bn.nspname='sys' AND base.typname='uint4' \
             AND (SELECT count(*) FROM sys_catalog.sys_constraint co WHERE co.contypid=t.oid)=1 \
             AND EXISTS (SELECT 1 FROM sys_catalog.sys_constraint co WHERE co.contypid=t.oid AND co.contype='c' AND co.convalidated \
             AND replace(replace(sys_catalog.sys_get_expr(co.conbin,0),'OPERATOR(sys.<=)','<='),' ','')='(VALUE<={maximum})')) \
             THEN RAISE EXCEPTION 'Reserved Prisma unsigned domain has an incompatible definition'; END IF; END;",
            domain=qualified_name(table, name), constraint=Quoted::mysql_ident(format!("{name}_range")),
        );
        // A quoted body also handles schema names containing dollar delimiters.
        format!("DO '{}'", body.replace('\'', "''"))
    }).collect();
    if table.columns().any(|column| drop_auto_guard(column).is_some()) {
        let function_body = sql_schema_describer::kingbase_mysql::UNSIGNED_AUTO_GUARD_BODY;
        let identity = format!(
            "n.nspname={} AND p.proname='__prisma_unsigned_auto_guard' AND p.pronargs=0",
            namespace
        );
        let create = format!(
            "CREATE FUNCTION {}() RETURNS trigger LANGUAGE plpgsql AS {}",
            qualified_name(table, "__prisma_unsigned_auto_guard"),
            sql_literal(function_body)
        );
        let body = format!(
            "BEGIN IF NOT EXISTS (SELECT 1 FROM sys_catalog.sys_proc p JOIN sys_catalog.sys_namespace n ON n.oid=p.pronamespace WHERE {identity}) THEN EXECUTE {};             ELSIF NOT EXISTS (SELECT 1 FROM sys_catalog.sys_proc p JOIN sys_catalog.sys_namespace n ON n.oid=p.pronamespace WHERE {identity} AND p.prorettype='trigger'::regtype AND p.prosrc={}) THEN             RAISE EXCEPTION 'Reserved Prisma unsigned guard has an incompatible definition'; END IF; END;",
            sql_literal(&create),
            sql_literal(function_body)
        );
        statements.push(format!("DO {}", sql_literal(&body)));
    }
    statements
}
