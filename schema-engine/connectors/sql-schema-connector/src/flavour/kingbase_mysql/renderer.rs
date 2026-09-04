use super::schema_differ::explicit_cast_supported;
use crate::{
    flavour::MysqlRenderer,
    migration_pair::MigrationPair,
    sql_migration::{AlterColumn, AlterEnum, AlterTable, RedefineTable, TableChange},
    sql_renderer::{Quoted, QuotedWithPrefix, SqlRenderer},
    sql_schema_differ::ColumnChanges,
};
use itertools::Itertools;
use psl::builtin_connectors::KingbaseMySqlType;
use regex::Regex;
use sql_ddl::{IndexColumn, SortOrder, mysql as ddl};
use sql_schema_describer::{
    self as sql, ColumnTypeFamily, DefaultKind, DefaultValue, PrismaValue, SQLSortOrder, SqlSchema,
    walkers::{
        EnumWalker, ForeignKeyWalker, IndexWalker, TableColumnWalker, TableWalker, UserDefinedTypeWalker, ViewWalker,
    },
};
use std::{borrow::Cow, fmt::Write as _, sync::LazyLock};

/// Kingbase MySQL migration renderer.
///
/// This starts with the exact MySQL renderer behavior. Kingbase-specific DDL
/// differences can be overridden here without changing standard MySQL output.
#[derive(Debug, Default)]
pub(crate) struct KingbaseMysqlRenderer;

impl SqlRenderer for KingbaseMysqlRenderer {
    fn quote<'a>(&self, name: &'a str) -> Quoted<&'a str> {
        MysqlRenderer.quote(name)
    }

    fn render_add_foreign_key(&self, foreign_key: ForeignKeyWalker<'_>) -> String {
        MysqlRenderer.render_add_foreign_key(foreign_key)
    }

    fn render_alter_enum(&self, alter_enum: &AlterEnum, schemas: MigrationPair<&SqlSchema>) -> Vec<String> {
        MysqlRenderer.render_alter_enum(alter_enum, schemas)
    }

    fn render_rename_index(&self, indexes: MigrationPair<IndexWalker<'_>>) -> Vec<String> {
        vec![format!(
            "ALTER INDEX {} RENAME TO {}",
            Quoted::postgres_ident(indexes.previous.name()),
            Quoted::postgres_ident(indexes.next.name()),
        )]
    }

    fn render_alter_table(&self, alter_table: &AlterTable, schemas: MigrationPair<&SqlSchema>) -> Vec<String> {
        let AlterTable { table_ids, changes } = alter_table;
        let tables = schemas.walk(*table_ids);
        let mut lines = Vec::new();

        for change in changes {
            match change {
                TableChange::DropPrimaryKey => {
                    let primary_key = tables.previous.primary_key().unwrap();
                    // Kingbase names an unnamed primary-key constraint `<table>_pkey`,
                    // while the describer intentionally leaves the MySQL-style name empty.
                    let name = if primary_key.name().is_empty() {
                        format!("{}_pkey", tables.previous.name())
                    } else {
                        primary_key.name().to_owned()
                    };
                    lines.push(format!("DROP CONSTRAINT {}", Quoted::postgres_ident(&name)));
                }
                TableChange::RenamePrimaryKey => unreachable!("Kingbase MySQL does not support renaming primary keys"),
                TableChange::AddPrimaryKey => lines.push(format!(
                    "ADD PRIMARY KEY ({})",
                    tables
                        .next
                        .primary_key_columns()
                        .unwrap()
                        .map(|column| {
                            let mut rendered = self.quote(column.as_column().name()).to_string();

                            if let Some(length) = column.length() {
                                write!(rendered, "({length})").unwrap();
                            }

                            if let Some(sort_order) = column.sort_order() {
                                rendered.push(' ');
                                rendered.push_str(sort_order.as_ref());
                            }

                            rendered
                        })
                        .join(", ")
                )),
                TableChange::AddColumn {
                    column_id,
                    has_virtual_default: _,
                } => lines.push(format!("ADD COLUMN {}", render_column(tables.next.walk(*column_id)))),
                TableChange::DropColumn { column_id } => lines.push(
                    sql_ddl::mysql::AlterTableClause::DropColumn {
                        column_name: tables.previous.walk(*column_id).name().into(),
                    }
                    .to_string(),
                ),
                TableChange::AlterColumn(AlterColumn {
                    changes,
                    column_id,
                    type_change,
                }) => {
                    let columns = schemas.walk(*column_id);

                    if changes.only_default_changed() && columns.next.default().is_none() {
                        lines.push(format!(
                            "ALTER COLUMN {} DROP DEFAULT",
                            Quoted::mysql_ident(columns.previous.name())
                        ));
                    } else {
                        let defaults = (
                            columns.previous.default().as_ref().map(|default| default.kind()),
                            columns.next.default().as_ref().map(|default| default.kind()),
                        );
                        let new_default = match defaults {
                            (Some(DefaultKind::DbGenerated(Some(previous))), Some(DefaultKind::DbGenerated(next)))
                                if (next.is_none() || next.as_deref() == Some("")) && !previous.is_empty() =>
                            {
                                Some(DefaultValue::db_generated(previous.clone()))
                            }
                            _ => columns.next.default().map(|default| default.inner()).cloned(),
                        };

                        if matches!(type_change, Some(crate::sql_migration::ColumnTypeChange::RiskyCast))
                            && explicit_cast_supported(
                                columns.previous.column_native_type::<KingbaseMySqlType>(),
                                columns.next.column_native_type::<KingbaseMySqlType>(),
                            )
                        {
                            lines.extend(render_alter_column_with_using(changes, new_default.as_ref(), columns));
                        } else {
                            lines.push(render_modify_column(changes, new_default.as_ref(), columns.next));
                        }
                    }
                }
                TableChange::DropAndRecreateColumn { column_id, changes: _ } => {
                    let columns = schemas.walk(*column_id);
                    lines.push(format!("DROP COLUMN `{}`", columns.previous.name()));
                    lines.push(format!("ADD COLUMN {}", render_column(columns.next)));
                }
            }
        }

        if lines.is_empty() {
            Vec::new()
        } else {
            vec![format!(
                "ALTER TABLE {} {}",
                self.quote(tables.previous.name()),
                lines.join(",\n    ")
            )]
        }
    }

    fn render_create_enum(&self, create_enum: EnumWalker<'_>) -> Vec<String> {
        MysqlRenderer.render_create_enum(create_enum)
    }

    fn render_create_index(&self, index: IndexWalker<'_>) -> String {
        if index.index_type() == sql::IndexType::Fulltext {
            return render_fulltext_index(index);
        }

        MysqlRenderer.render_create_index(index)
    }

    fn render_create_table(&self, table: TableWalker<'_>) -> String {
        self.render_create_table_as(table, QuotedWithPrefix(None, Quoted::mysql_ident(table.name())))
    }

    fn render_create_table_as(&self, table: TableWalker<'_>, table_name: QuotedWithPrefix<&str>) -> String {
        // Kingbase accepts the MySQL column and index syntax, but does not
        // accept MySQL's table-level character-set/collation options.
        ddl::CreateTable {
            table_name: &table_name,
            columns: table.columns().map(render_column).collect(),
            // The GIN expression used by full-text indexes cannot be defined
            // inline. Normal and unique indexes stay inline so constraints
            // referenced by foreign keys exist when the table is created.
            indexes: table
                .indexes()
                .filter(|index| !index.is_primary_key() && index.index_type() != sql::IndexType::Fulltext)
                .map(|index| ddl::IndexClause {
                    index_name: Some(Cow::from(index.name())),
                    r#type: match index.index_type() {
                        sql::IndexType::Unique => ddl::IndexType::Unique,
                        sql::IndexType::Normal => ddl::IndexType::Normal,
                        sql::IndexType::Fulltext | sql::IndexType::PrimaryKey => unreachable!(),
                    },
                    columns: index.columns().map(render_index_column).collect(),
                })
                .collect(),
            primary_key: table
                .primary_key_columns()
                .into_iter()
                .flatten()
                .map(render_index_column)
                .collect(),
            default_character_set: None,
            collate: None,
        }
        .to_string()
    }

    fn render_drop_and_recreate_index(&self, indexes: MigrationPair<IndexWalker<'_>>) -> Vec<String> {
        if indexes.previous.index_type() == sql::IndexType::Fulltext
            || indexes.next.index_type() == sql::IndexType::Fulltext
        {
            return vec![
                self.render_drop_index(None, indexes.previous),
                self.render_create_index(indexes.next),
            ];
        }

        MysqlRenderer.render_drop_and_recreate_index(indexes)
    }

    fn render_drop_enum(&self, namespace: Option<&str>, dropped_enum: EnumWalker<'_>) -> Vec<String> {
        MysqlRenderer.render_drop_enum(namespace, dropped_enum)
    }

    fn render_drop_foreign_key(&self, namespace: Option<&str>, foreign_key: ForeignKeyWalker<'_>) -> String {
        let _ = namespace;
        format!(
            "ALTER TABLE {table} DROP CONSTRAINT {constraint_name}",
            table = self.quote(foreign_key.table().name()),
            constraint_name = Quoted::mysql_ident(foreign_key.constraint_name().unwrap()),
        )
    }

    fn render_drop_index(&self, namespace: Option<&str>, index: IndexWalker<'_>) -> String {
        let _ = namespace;
        if index.is_unique() {
            format!(
                "ALTER TABLE {} DROP CONSTRAINT {}",
                self.quote(index.table().name()),
                Quoted::postgres_ident(index.name()),
            )
        } else {
            format!("DROP INDEX {}", Quoted::postgres_ident(index.name()))
        }
    }

    fn render_drop_table(&self, namespace: Option<&str>, table_name: &str) -> Vec<String> {
        MysqlRenderer.render_drop_table(namespace, table_name)
    }

    fn render_redefine_tables(&self, tables: &[RedefineTable], schemas: MigrationPair<&SqlSchema>) -> Vec<String> {
        MysqlRenderer.render_redefine_tables(tables, schemas)
    }

    fn render_rename_table(&self, namespace: Option<&str>, name: &str, new_name: &str) -> String {
        MysqlRenderer.render_rename_table(namespace, name, new_name)
    }

    fn render_drop_view(&self, namespace: Option<&str>, view: ViewWalker<'_>) -> String {
        MysqlRenderer.render_drop_view(namespace, view)
    }

    fn render_drop_user_defined_type(&self, namespace: Option<&str>, udt: &UserDefinedTypeWalker<'_>) -> String {
        MysqlRenderer.render_drop_user_defined_type(namespace, udt)
    }

    fn render_rename_foreign_key(&self, fks: MigrationPair<ForeignKeyWalker<'_>>) -> String {
        MysqlRenderer.render_rename_foreign_key(fks)
    }

    fn render_create_namespace(&self, namespace: sql::NamespaceWalker<'_>) -> Vec<String> {
        MysqlRenderer.render_create_namespace(namespace)
    }
}

fn render_fulltext_index(index: IndexWalker<'_>) -> String {
    let document = index
        .columns()
        .map(|column| format!("COALESCE({}, '')", Quoted::mysql_ident(column.as_column().name())))
        .reduce(|document, column| format!("textcat({document}, textcat(' ', {column}))"))
        .expect("a full-text index must contain at least one column");

    format!(
        "CREATE INDEX {index_name} ON {table_name} USING GIN (to_tsvector('simple', {document}))",
        index_name = Quoted::mysql_ident(index.name()),
        table_name = Quoted::mysql_ident(index.table().name()),
    )
}

fn render_column(col: TableColumnWalker<'_>) -> ddl::Column<'_> {
    let default = col
        .default()
        .filter(|default| {
            !matches!(
                default.kind(),
                DefaultKind::Sequence(_) | DefaultKind::DbGenerated(None)
            ) && !matches!(col.column_type_family(), ColumnTypeFamily::Json)
                && !matches!(col.column_type_family(), ColumnTypeFamily::Binary if !default.is_db_generated())
        })
        .map(|default| render_default(col, default.inner()));

    ddl::Column {
        column_name: col.name().into(),
        not_null: col.arity().is_required(),
        column_type: render_column_type(col),
        default,
        auto_increment: col.is_autoincrement(),
        ..Default::default()
    }
}

fn render_index_column(column: sql::walkers::IndexColumnWalker<'_>) -> IndexColumn<'_> {
    IndexColumn {
        name: column.as_column().name().into(),
        length: column.length(),
        sort_order: column.sort_order().map(|sort_order| match sort_order {
            SQLSortOrder::Asc => SortOrder::Asc,
            SQLSortOrder::Desc => SortOrder::Desc,
        }),
        operator_class: None,
    }
}

fn render_column_type(column: TableColumnWalker<'_>) -> Cow<'static, str> {
    if let ColumnTypeFamily::Enum(enum_id) = column.column_type_family() {
        let variants = column.walk(*enum_id).values().map(Quoted::mysql_string).join(", ");
        return format!("ENUM({variants})").into();
    }

    if let ColumnTypeFamily::Unsupported(description) = &column.column_type().family {
        return description.to_string().into();
    }

    let native_type = column
        .column_native_type::<KingbaseMySqlType>()
        .expect("Column native type missing in kingbase_mysql_renderer::render_column_type()");

    fn render(input: Option<u32>) -> String {
        input.map_or_else(String::new, |arg| format!("({arg})"))
    }

    fn render_decimal(input: Option<(u32, u32)>) -> String {
        input.map_or_else(String::new, |(precision, scale)| format!("({precision}, {scale})"))
    }

    match native_type {
        KingbaseMySqlType::Int => "INTEGER".into(),
        KingbaseMySqlType::SmallInt => "SMALLINT".into(),
        KingbaseMySqlType::TinyInt if column.column_type_family().is_boolean() => "BOOLEAN".into(),
        KingbaseMySqlType::TinyInt => "TINYINT".into(),
        KingbaseMySqlType::MediumInt => "MEDIUMINT".into(),
        KingbaseMySqlType::BigInt => "BIGINT".into(),
        KingbaseMySqlType::Decimal(precision) => format!("DECIMAL{}", render_decimal(*precision)).into(),
        KingbaseMySqlType::Float => "FLOAT".into(),
        KingbaseMySqlType::Double => "DOUBLE".into(),
        KingbaseMySqlType::Bit(size) => format!("BIT({size})").into(),
        KingbaseMySqlType::Char(size) => format!("CHAR({size})").into(),
        KingbaseMySqlType::VarChar(size) => format!("VARCHAR({size})").into(),
        KingbaseMySqlType::Binary(size) => format!("BINARY({size})").into(),
        KingbaseMySqlType::VarBinary(size) => format!("VARBINARY({size})").into(),
        KingbaseMySqlType::TinyBlob => "TINYBLOB".into(),
        KingbaseMySqlType::Blob => "BLOB".into(),
        KingbaseMySqlType::MediumBlob => "MEDIUMBLOB".into(),
        KingbaseMySqlType::LongBlob => "LONGBLOB".into(),
        KingbaseMySqlType::TinyText => "TINYTEXT".into(),
        KingbaseMySqlType::Text => "TEXT".into(),
        KingbaseMySqlType::MediumText => "MEDIUMTEXT".into(),
        KingbaseMySqlType::LongText => "LONGTEXT".into(),
        KingbaseMySqlType::Date => "DATE".into(),
        KingbaseMySqlType::Time(precision) => format!("TIME{}", render(*precision)).into(),
        KingbaseMySqlType::DateTime(precision) => format!("DATETIME{}", render(*precision)).into(),
        KingbaseMySqlType::Timestamp(precision) => format!("TIMESTAMP{}", render(*precision)).into(),
        KingbaseMySqlType::Year => "YEAR".into(),
        KingbaseMySqlType::Json => "JSON".into(),
        // Kingbase's MySQL compatibility layer accepts these integer names,
        // but not MySQL's `UNSIGNED` modifier.
        KingbaseMySqlType::UnsignedInt => "INTEGER".into(),
        KingbaseMySqlType::UnsignedSmallInt => "SMALLINT".into(),
        KingbaseMySqlType::UnsignedTinyInt => "TINYINT".into(),
        KingbaseMySqlType::UnsignedMediumInt => "MEDIUMINT".into(),
        KingbaseMySqlType::UnsignedBigInt => "BIGINT".into(),
    }
}

fn render_modify_column(
    changes: &ColumnChanges,
    new_default: Option<&DefaultValue>,
    next_column: TableColumnWalker<'_>,
) -> String {
    let column_type = if changes.type_changed() {
        Some(next_column.column_type().full_data_type.clone())
            .filter(|type_name| !type_name.is_empty() || type_name.to_ascii_lowercase().contains("datetime"))
    } else {
        Some(next_column.column_type().full_data_type.clone()).filter(|type_name| !type_name.is_empty())
    }
    .map(Cow::Owned)
    .unwrap_or_else(|| render_column_type(next_column));

    let default = new_default
        .filter(|default| !default.is_empty_dbgenerated())
        .map(|default| render_default(next_column, default))
        .filter(|expression| !expression.is_empty())
        .map(|expression| format!(" DEFAULT {expression}"))
        .unwrap_or_default();

    format!(
        "MODIFY {column_name} {column_type}{nullability}{default}{sequence}",
        column_name = Quoted::mysql_ident(next_column.name()),
        nullability = if next_column.arity().is_required() {
            " NOT NULL"
        } else {
            " NULL"
        },
        sequence = if next_column.is_autoincrement() {
            " AUTO_INCREMENT"
        } else {
            ""
        },
    )
}

fn render_alter_column_with_using(
    changes: &ColumnChanges,
    new_default: Option<&DefaultValue>,
    columns: MigrationPair<TableColumnWalker<'_>>,
) -> Vec<String> {
    let column_name = Quoted::mysql_ident(columns.previous.name());
    let target_type = render_column_type(columns.next);
    let using = render_using_expression(columns).expect("explicit cast support must have a USING expression");
    let mut clauses = vec![format!("ALTER COLUMN {column_name} TYPE {target_type} USING ({using})",)];

    if changes.default_changed() {
        match new_default {
            Some(default) if !default.is_empty_dbgenerated() => clauses.push(format!(
                "ALTER COLUMN {column_name} SET DEFAULT {default}",
                default = render_default(columns.next, default),
            )),
            _ => clauses.push(format!("ALTER COLUMN {column_name} DROP DEFAULT")),
        }
    }

    if changes.arity_changed() {
        clauses.push(if columns.next.arity().is_required() {
            format!("ALTER COLUMN {column_name} SET NOT NULL")
        } else {
            format!("ALTER COLUMN {column_name} DROP NOT NULL")
        });
    }

    clauses
}

fn render_using_expression(columns: MigrationPair<TableColumnWalker<'_>>) -> Option<String> {
    let previous = columns.previous.column_native_type::<KingbaseMySqlType>()?;
    let next = columns.next.column_native_type::<KingbaseMySqlType>()?;
    let column = Quoted::mysql_ident(columns.previous.name()).to_string();
    let target_type = render_column_type(columns.next);

    render_using_expression_for_types(previous, next, &column, &target_type)
}

fn render_using_expression_for_types(
    previous: &KingbaseMySqlType,
    next: &KingbaseMySqlType,
    column: &str,
    target_type: &str,
) -> Option<String> {
    if !explicit_cast_supported(Some(previous), Some(next)) {
        return None;
    }

    let expression = if is_datetime(previous) && is_numeric(next) {
        let format = match previous {
            KingbaseMySqlType::Date => "YYYYMMDD",
            KingbaseMySqlType::Time(_) => "HH24MISS",
            _ => "YYYYMMDDHH24MISS",
        };
        format!("to_char({column}, '{format}')::{target_type}")
    } else if is_numeric(previous) && matches!(next, KingbaseMySqlType::Bit(_)) {
        format!("({column})::bigint::{target_type}")
    } else if matches!(previous, KingbaseMySqlType::Bit(_)) {
        format!("({column})::text::{target_type}")
    } else if is_binary(previous) && !is_binary(next) {
        format!("convert_from(({column})::bytea, 'UTF8')::{target_type}")
    } else if is_json(previous) && !is_string(next) {
        format!("({column})::text::{target_type}")
    } else if is_binary(next) && !is_binary(previous) {
        format!("({column})::text::{target_type}")
    } else if is_json(next) && is_numeric(previous) {
        format!("to_json({column})")
    } else if is_json(next) {
        format!("({column})::text::{target_type}")
    } else {
        format!("{column}::{target_type}")
    };

    Some(expression)
}

fn is_binary(native_type: &KingbaseMySqlType) -> bool {
    matches!(
        native_type,
        KingbaseMySqlType::Bit(_)
            | KingbaseMySqlType::Binary(_)
            | KingbaseMySqlType::VarBinary(_)
            | KingbaseMySqlType::TinyBlob
            | KingbaseMySqlType::Blob
            | KingbaseMySqlType::MediumBlob
            | KingbaseMySqlType::LongBlob
    )
}

fn is_string(native_type: &KingbaseMySqlType) -> bool {
    matches!(
        native_type,
        KingbaseMySqlType::Char(_)
            | KingbaseMySqlType::VarChar(_)
            | KingbaseMySqlType::TinyText
            | KingbaseMySqlType::Text
            | KingbaseMySqlType::MediumText
            | KingbaseMySqlType::LongText
    )
}

fn is_numeric(native_type: &KingbaseMySqlType) -> bool {
    matches!(
        native_type,
        KingbaseMySqlType::Int
            | KingbaseMySqlType::UnsignedInt
            | KingbaseMySqlType::SmallInt
            | KingbaseMySqlType::UnsignedSmallInt
            | KingbaseMySqlType::TinyInt
            | KingbaseMySqlType::UnsignedTinyInt
            | KingbaseMySqlType::MediumInt
            | KingbaseMySqlType::UnsignedMediumInt
            | KingbaseMySqlType::BigInt
            | KingbaseMySqlType::UnsignedBigInt
            | KingbaseMySqlType::Decimal(_)
            | KingbaseMySqlType::Float
            | KingbaseMySqlType::Double
            | KingbaseMySqlType::Year
    )
}

fn is_datetime(native_type: &KingbaseMySqlType) -> bool {
    matches!(
        native_type,
        KingbaseMySqlType::Date
            | KingbaseMySqlType::Time(_)
            | KingbaseMySqlType::DateTime(_)
            | KingbaseMySqlType::Timestamp(_)
    )
}

fn is_json(native_type: &KingbaseMySqlType) -> bool {
    matches!(native_type, KingbaseMySqlType::Json)
}

fn escape_string_literal(s: &str) -> Cow<'_, str> {
    static STRING_LITERAL_CHARACTER_TO_ESCAPE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"'"#).unwrap());
    STRING_LITERAL_CHARACTER_TO_ESCAPE_RE.replace_all(s, "'$0")
}

fn render_default<'a>(column: TableColumnWalker<'a>, default: &'a DefaultValue) -> Cow<'a, str> {
    match default.kind() {
        DefaultKind::DbGenerated(Some(value)) => value.as_str().into(),
        DefaultKind::Value(PrismaValue::String(value)) | DefaultKind::Value(PrismaValue::Enum(value)) => {
            Quoted::mysql_string(escape_string_literal(value)).to_string().into()
        }
        DefaultKind::Now => column
            .column_native_type::<KingbaseMySqlType>()
            .and_then(KingbaseMySqlType::timestamp_precision)
            .map(|precision| format!("CURRENT_TIMESTAMP({precision})").into())
            .unwrap_or_else(|| "CURRENT_TIMESTAMP".into()),
        DefaultKind::Value(PrismaValue::DateTime(value)) if column.column_type_family().is_datetime() => {
            Quoted::mysql_string(value.to_rfc3339()).to_string().into()
        }
        DefaultKind::Value(value) => value.to_string().into(),
        DefaultKind::DbGenerated(None) | DefaultKind::Sequence(_) | DefaultKind::UniqueRowid => unreachable!(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_with_mysql_identifier_quoting() {
        assert_eq!(KingbaseMysqlRenderer.quote("users").to_string(), "`users`");
    }

    #[test]
    fn renders_using_expressions_for_non_implicit_casts() {
        assert_eq!(
            render_using_expression_for_types(
                &KingbaseMySqlType::DateTime(Some(0)),
                &KingbaseMySqlType::Double,
                "`created_at`",
                "DOUBLE",
            ),
            Some("to_char(`created_at`, 'YYYYMMDDHH24MISS')::DOUBLE".to_owned())
        );
        assert_eq!(
            render_using_expression_for_types(
                &KingbaseMySqlType::VarChar(32),
                &KingbaseMySqlType::Json,
                "`payload`",
                "JSON",
            ),
            Some("(`payload`)::text::JSON".to_owned())
        );
        assert_eq!(
            render_using_expression_for_types(
                &KingbaseMySqlType::Blob,
                &KingbaseMySqlType::VarChar(20),
                "`payload`",
                "VARCHAR(20)",
            ),
            Some("convert_from((`payload`)::bytea, 'UTF8')::VARCHAR(20)".to_owned())
        );
        assert_eq!(
            render_using_expression_for_types(
                &KingbaseMySqlType::BigInt,
                &KingbaseMySqlType::Bit(54),
                "`value`",
                "BIT(54)",
            ),
            Some("(`value`)::bigint::BIT(54)".to_owned())
        );
        assert_eq!(
            render_using_expression_for_types(&KingbaseMySqlType::Bit(32), &KingbaseMySqlType::Text, "`value`", "TEXT",),
            Some("(`value`)::text::TEXT".to_owned())
        );
    }

    #[test]
    fn does_not_render_using_for_unrepresentable_casts() {
        assert_eq!(
            render_using_expression_for_types(
                &KingbaseMySqlType::Blob,
                &KingbaseMySqlType::Int,
                "`payload`",
                "INTEGER",
            ),
            None
        );
    }
}
