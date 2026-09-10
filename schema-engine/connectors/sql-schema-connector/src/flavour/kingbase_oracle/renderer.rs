use crate::{
    flavour::PostgresRenderer,
    migration_pair::MigrationPair,
    sql_migration::{AlterColumn, AlterEnum, AlterTable, RedefineTable, TableChange},
    sql_renderer::{Quoted, QuotedWithPrefix, SqlRenderer},
};
use itertools::Itertools;
use psl::builtin_connectors::{KingbaseOracleNumberArguments, KingbaseOracleType};
use sql_schema_describer::{
    self as sql, ColumnTypeFamily, DefaultKind, DefaultValue, ForeignKeyAction, PrismaValue, SQLSortOrder, SqlSchema,
    walkers::{
        EnumWalker, ForeignKeyWalker, IndexWalker, TableColumnWalker, TableWalker, UserDefinedTypeWalker, ViewWalker,
    },
};
use std::{borrow::Cow, fmt::Write as _};

/// DDL renderer for KingbaseES in Oracle compatibility mode.
///
/// Oracle-mode public types and `ALTER TABLE ... MODIFY` are emitted here.
/// PostgreSQL catalog-compatible syntax is reused only for enum evolution,
/// which Kingbase Oracle mode implements unchanged.
#[derive(Debug, Default)]
pub(crate) struct KingbaseOracleRenderer;

impl SqlRenderer for KingbaseOracleRenderer {
    fn quote<'a>(&self, name: &'a str) -> Quoted<&'a str> {
        Quoted::postgres_ident(name)
    }

    fn render_add_foreign_key(&self, foreign_key: ForeignKeyWalker<'_>) -> String {
        let constraint = foreign_key
            .constraint_name()
            .map(|name| format!("CONSTRAINT {} ", self.quote(name)))
            .unwrap_or_default();
        let constrained_columns = foreign_key
            .constrained_columns()
            .map(|column| self.quote(column.name()))
            .join(", ");
        let referenced_columns = foreign_key
            .referenced_columns()
            .map(|column| self.quote(column.name()))
            .join(", ");

        format!(
            "ALTER TABLE {table} ADD {constraint}FOREIGN KEY ({constrained_columns}) REFERENCES {referenced_table} ({referenced_columns}) ON DELETE {on_delete} ON UPDATE {on_update}",
            table = quoted_table(foreign_key.table()),
            referenced_table = quoted_table(foreign_key.referenced_table()),
            on_delete = render_foreign_key_action(foreign_key.on_delete_action()),
            on_update = render_foreign_key_action(foreign_key.on_update_action()),
        )
    }

    fn render_alter_enum(&self, alter_enum: &AlterEnum, schemas: MigrationPair<&SqlSchema>) -> Vec<String> {
        // Enum storage and ALTER TYPE semantics are PostgreSQL-compatible in
        // Kingbase Oracle mode. Reusing this narrow implementation keeps the
        // data-preserving drop/recreate path for removed enum values.
        PostgresRenderer::new(false).render_alter_enum(alter_enum, schemas)
    }

    fn render_rename_index(&self, indexes: MigrationPair<IndexWalker<'_>>) -> Vec<String> {
        vec![format!(
            "ALTER INDEX {} RENAME TO {}",
            quoted_index(indexes.previous),
            self.quote(indexes.next.name())
        )]
    }

    fn render_alter_table(&self, alter_table: &AlterTable, schemas: MigrationPair<&SqlSchema>) -> Vec<String> {
        let tables = schemas.walk(alter_table.table_ids);
        let table = quoted_table(tables.previous);
        let mut statements = Vec::new();

        for change in &alter_table.changes {
            match change {
                TableChange::DropPrimaryKey => {
                    let primary_key = tables
                        .previous
                        .primary_key()
                        .expect("drop primary key without a primary key");
                    statements.push(format!(
                        "ALTER TABLE {table} DROP CONSTRAINT {}",
                        self.quote(primary_key.name())
                    ));
                }
                TableChange::RenamePrimaryKey => {
                    let previous = tables
                        .previous
                        .primary_key()
                        .expect("rename primary key without a primary key");
                    let next = tables
                        .next
                        .primary_key()
                        .expect("rename primary key without a primary key");
                    statements.push(format!(
                        "ALTER TABLE {table} RENAME CONSTRAINT {} TO {}",
                        self.quote(previous.name()),
                        self.quote(next.name())
                    ));
                }
                TableChange::AddPrimaryKey => {
                    let primary_key = tables
                        .next
                        .primary_key()
                        .expect("add primary key without a primary key");
                    let columns = primary_key.columns().map(|column| self.quote(column.name())).join(", ");
                    statements.push(format!(
                        "ALTER TABLE {table} ADD CONSTRAINT {} PRIMARY KEY ({columns})",
                        self.quote(primary_key.name())
                    ));
                }
                TableChange::AddColumn { column_id, .. } => {
                    let column = tables.next.walk(*column_id);
                    statements.push(format!("ALTER TABLE {table} ADD ({})", render_column(column)));
                }
                TableChange::DropColumn { column_id } => {
                    let column = tables.previous.walk(*column_id);
                    statements.push(format!("ALTER TABLE {table} DROP COLUMN {}", self.quote(column.name())));
                }
                TableChange::AlterColumn(AlterColumn { column_id, changes, .. }) => {
                    let columns = schemas.walk(*column_id);
                    statements.extend(render_alter_column(&table, columns, changes));
                }
                TableChange::DropAndRecreateColumn { column_id, .. } => {
                    let columns = schemas.walk(*column_id);
                    statements.push(format!(
                        "ALTER TABLE {table} DROP COLUMN {}",
                        self.quote(columns.previous.name())
                    ));
                    statements.push(format!("ALTER TABLE {table} ADD ({})", render_column(columns.next)));
                }
            }
        }

        statements
    }

    fn render_create_enum(&self, create_enum: EnumWalker<'_>) -> Vec<String> {
        PostgresRenderer::new(false).render_create_enum(create_enum)
    }

    fn render_create_index(&self, index: IndexWalker<'_>) -> String {
        assert_ne!(
            index.index_type(),
            sql::IndexType::Fulltext,
            "Kingbase Oracle does not support Prisma full-text indexes"
        );

        let unique = if index.is_unique() { "UNIQUE " } else { "" };
        let columns = index
            .columns()
            .map(|column| {
                let mut output = self.quote(column.as_column().name()).to_string();
                if let Some(sort_order) = column.sort_order() {
                    output.push(' ');
                    output.push_str(match sort_order {
                        SQLSortOrder::Asc => "ASC",
                        SQLSortOrder::Desc => "DESC",
                    });
                }
                output
            })
            .join(", ");

        format!(
            "CREATE {unique}INDEX {} ON {} ({columns})",
            self.quote(index.name()),
            quoted_table(index.table())
        )
    }

    fn render_create_table(&self, table: TableWalker<'_>) -> String {
        self.render_create_table_as(table, quoted_table(table))
    }

    fn render_create_table_as(&self, table: TableWalker<'_>, table_name: QuotedWithPrefix<&str>) -> String {
        let mut definitions = table.columns().map(render_column).collect::<Vec<_>>();

        if let Some(primary_key) = table.primary_key() {
            let columns = primary_key.columns().map(|column| self.quote(column.name())).join(", ");
            definitions.push(format!(
                "CONSTRAINT {} PRIMARY KEY ({columns})",
                self.quote(primary_key.name())
            ));
        }

        format!("CREATE TABLE {table_name} (\n    {}\n)", definitions.join(",\n    "))
    }

    fn render_drop_and_recreate_index(&self, indexes: MigrationPair<IndexWalker<'_>>) -> Vec<String> {
        vec![
            self.render_drop_index(None, indexes.previous),
            self.render_create_index(indexes.next),
        ]
    }

    fn render_drop_enum(&self, namespace: Option<&str>, dropped_enum: EnumWalker<'_>) -> Vec<String> {
        let enum_name = QuotedWithPrefix::pg_new(namespace, dropped_enum.name());
        vec![format!("DROP TYPE {enum_name}")]
    }

    fn render_drop_foreign_key(&self, _namespace: Option<&str>, foreign_key: ForeignKeyWalker<'_>) -> String {
        format!(
            "ALTER TABLE {} DROP CONSTRAINT {}",
            quoted_table(foreign_key.table()),
            self.quote(foreign_key.constraint_name().expect("foreign key name is required"))
        )
    }

    fn render_drop_index(&self, namespace: Option<&str>, index: IndexWalker<'_>) -> String {
        let index_name = QuotedWithPrefix::pg_new(namespace, index.name());
        format!("DROP INDEX {index_name}")
    }

    fn render_redefine_tables(&self, tables: &[RedefineTable], schemas: MigrationPair<&SqlSchema>) -> Vec<String> {
        let mut statements = Vec::new();

        for redefine_table in tables {
            let tables = schemas.walk(redefine_table.table_ids);
            let temporary_name = format!("_prisma_new_{}", tables.next.name());
            let temporary_table = QuotedWithPrefix::pg_new(tables.next.explicit_namespace(), temporary_name.as_str());
            statements.push(self.render_create_table_as(tables.next, temporary_table));

            let columns = redefine_table
                .column_pairs
                .iter()
                .map(|(column_ids, _, _)| self.quote(schemas.walk(*column_ids).next.name()).to_string())
                .join(", ");
            if !columns.is_empty() {
                statements.push(format!(
                    "INSERT INTO {temporary_table} ({columns}) SELECT {columns} FROM {}",
                    quoted_table(tables.previous)
                ));
            }

            statements.push(format!("DROP TABLE {} CASCADE", quoted_table(tables.previous)));
            statements.push(self.render_rename_table(
                tables.next.explicit_namespace(),
                &temporary_name,
                tables.next.name(),
            ));

            for index in tables.next.indexes().filter(|index| !index.is_primary_key()) {
                statements.push(self.render_create_index(index));
            }
            for foreign_key in tables.next.foreign_keys() {
                statements.push(self.render_add_foreign_key(foreign_key));
            }
        }

        statements
    }

    fn render_rename_table(&self, namespace: Option<&str>, name: &str, new_name: &str) -> String {
        format!(
            "ALTER TABLE {} RENAME TO {}",
            QuotedWithPrefix::pg_new(namespace, name),
            self.quote(new_name)
        )
    }

    fn render_drop_view(&self, namespace: Option<&str>, view: ViewWalker<'_>) -> String {
        format!("DROP VIEW {}", QuotedWithPrefix::pg_new(namespace, view.name()))
    }

    fn render_drop_user_defined_type(&self, _namespace: Option<&str>, _udt: &UserDefinedTypeWalker<'_>) -> String {
        unreachable!("Kingbase Oracle does not support Prisma extension types")
    }

    fn render_rename_foreign_key(&self, fks: MigrationPair<ForeignKeyWalker<'_>>) -> String {
        format!(
            "ALTER TABLE {} RENAME CONSTRAINT {} TO {}",
            quoted_table(fks.next.table()),
            self.quote(fks.previous.constraint_name().expect("foreign key name is required")),
            self.quote(fks.next.constraint_name().expect("foreign key name is required")),
        )
    }

    fn render_create_namespace(&self, namespace: sql::NamespaceWalker<'_>) -> Vec<String> {
        vec![format!("CREATE SCHEMA IF NOT EXISTS {}", self.quote(namespace.name()))]
    }
}

fn render_alter_column(
    table: &QuotedWithPrefix<&str>,
    columns: MigrationPair<TableColumnWalker<'_>>,
    changes: &crate::sql_schema_differ::ColumnChanges,
) -> Vec<String> {
    let column_name = Quoted::postgres_ident(columns.previous.name());
    let mut statements = Vec::new();

    if changes.type_changed() {
        statements.push(format!(
            "ALTER TABLE {table} MODIFY ({column_name} {})",
            render_column_type(columns.next)
        ));
    }

    if changes.default_changed() {
        match columns.next.default() {
            Some(default) if !matches!(default.kind(), DefaultKind::DbGenerated(None)) => statements.push(format!(
                "ALTER TABLE {table} ALTER COLUMN {column_name} SET DEFAULT {}",
                render_default(default.inner())
            )),
            _ => statements.push(format!("ALTER TABLE {table} ALTER COLUMN {column_name} DROP DEFAULT")),
        }
    }

    if changes.arity_changed() {
        let nullability = if columns.next.arity().is_required() {
            "SET NOT NULL"
        } else {
            "DROP NOT NULL"
        };
        statements.push(format!("ALTER TABLE {table} ALTER COLUMN {column_name} {nullability}"));
    }

    if changes.autoincrement_changed() {
        if columns.next.is_autoincrement() {
            let sequence_name = sequence_name(columns.next.table().name(), columns.next.name());
            statements.push(format!("CREATE SEQUENCE {}", Quoted::postgres_ident(&sequence_name)));
            statements.push(format!(
                "ALTER TABLE {table} ALTER COLUMN {column_name} SET DEFAULT nextval({})",
                Quoted::postgres_string(&sequence_name)
            ));
            statements.push(format!(
                "ALTER SEQUENCE {} OWNED BY {table}.{column_name}",
                Quoted::postgres_ident(&sequence_name)
            ));
        } else {
            let previous_sequence = columns.previous.default().and_then(|default| default.as_sequence());
            statements.push(format!("ALTER TABLE {table} ALTER COLUMN {column_name} DROP DEFAULT"));
            if let Some(sequence) = previous_sequence.filter(|sequence| !sequence.is_empty()) {
                statements.push(format!("DROP SEQUENCE {}", Quoted::postgres_ident(sequence)));
            }
        }
    }

    statements
}

fn render_column(column: TableColumnWalker<'_>) -> String {
    let mut output = format!(
        "{} {}",
        Quoted::postgres_ident(column.name()),
        render_column_type(column)
    );

    if column.arity().is_required() {
        output.push_str(" NOT NULL");
    }

    if let Some(default) = column
        .default()
        .filter(|default| !matches!(default.kind(), DefaultKind::DbGenerated(None)))
        && !matches!(default.kind(), DefaultKind::Sequence(_))
    {
        output.push_str(" DEFAULT ");
        output.push_str(&render_default(default.inner()));
    }

    output
}

fn render_column_type(column: TableColumnWalker<'_>) -> Cow<'static, str> {
    if let Some(enm) = column.column_type_family_as_enum() {
        return QuotedWithPrefix::pg_new(enm.explicit_namespace(), enm.name())
            .to_string()
            .into();
    }

    if let ColumnTypeFamily::Unsupported(description) = &column.column_type().family {
        return description.to_owned().into();
    }

    let native_type = column
        .column_native_type::<KingbaseOracleType>()
        .expect("missing Kingbase Oracle native type in renderer");

    if column.is_autoincrement() {
        return match native_type {
            KingbaseOracleType::Number(KingbaseOracleNumberArguments::Precision(19))
            | KingbaseOracleType::Number(KingbaseOracleNumberArguments::PrecisionAndScale(19, 0)) => "BIGSERIAL".into(),
            _ => "SERIAL".into(),
        };
    }

    optional_argument_type(native_type)
}

fn optional_argument_type(native_type: &KingbaseOracleType) -> Cow<'static, str> {
    fn optional_argument(prefix: &str, argument: Option<u32>) -> Cow<'static, str> {
        match argument {
            Some(argument) => format!("{prefix}({argument})").into(),
            None => prefix.to_owned().into(),
        }
    }

    match native_type {
        KingbaseOracleType::Number(KingbaseOracleNumberArguments::Unspecified) => "NUMBER".into(),
        KingbaseOracleType::Number(KingbaseOracleNumberArguments::Precision(precision)) => {
            format!("NUMBER({precision})").into()
        }
        KingbaseOracleType::Number(KingbaseOracleNumberArguments::PrecisionAndScale(precision, scale)) => {
            format!("NUMBER({precision},{scale})").into()
        }
        KingbaseOracleType::Float(precision) => optional_argument("FLOAT", *precision),
        KingbaseOracleType::BinaryFloat => "BINARY_FLOAT".into(),
        KingbaseOracleType::BinaryDouble => "BINARY_DOUBLE".into(),
        KingbaseOracleType::Char(length) => optional_argument("CHAR", *length),
        KingbaseOracleType::VarChar2(length) => optional_argument("VARCHAR2", *length),
        KingbaseOracleType::NChar(length) => optional_argument("NCHAR", *length),
        KingbaseOracleType::NVarChar2(length) => optional_argument("NVARCHAR2", *length),
        KingbaseOracleType::Clob => "CLOB".into(),
        KingbaseOracleType::NClob => "NCLOB".into(),
        KingbaseOracleType::Blob => "BLOB".into(),
        KingbaseOracleType::Date => "DATE".into(),
        KingbaseOracleType::Timestamp(precision) => optional_argument("TIMESTAMP", *precision),
        KingbaseOracleType::TimestampTz(precision) => match precision {
            Some(precision) => format!("TIMESTAMP({precision}) WITH TIME ZONE").into(),
            None => "TIMESTAMP WITH TIME ZONE".into(),
        },
        KingbaseOracleType::TimestampLocalTz(precision) => match precision {
            Some(precision) => format!("TIMESTAMP({precision}) WITH LOCAL TIME ZONE").into(),
            None => "TIMESTAMP WITH LOCAL TIME ZONE".into(),
        },
        KingbaseOracleType::Boolean => "BOOLEAN".into(),
        KingbaseOracleType::Json => "JSON".into(),
        KingbaseOracleType::Uuid => "UUID".into(),
        KingbaseOracleType::Xml => "XML".into(),
    }
}

fn render_default(default: &DefaultValue) -> Cow<'_, str> {
    match default.kind() {
        DefaultKind::DbGenerated(Some(value)) => value.as_str().into(),
        DefaultKind::DbGenerated(None) | DefaultKind::Sequence(_) => Cow::Borrowed(""),
        DefaultKind::Now => Cow::Borrowed("CURRENT_TIMESTAMP"),
        DefaultKind::UniqueRowid => unreachable!("Kingbase Oracle does not support unique_rowid()"),
        DefaultKind::Value(PrismaValue::String(value) | PrismaValue::Enum(value) | PrismaValue::Json(value)) => {
            Quoted::postgres_string(value).to_string().into()
        }
        DefaultKind::Value(PrismaValue::DateTime(value)) => Quoted::postgres_string(value).to_string().into(),
        DefaultKind::Value(PrismaValue::Bytes(value)) => {
            let mut encoded = String::with_capacity(value.len() * 2);
            for byte in value {
                write!(encoded, "{byte:02x}").unwrap();
            }
            format!("decode({}, 'hex')", Quoted::postgres_string(&encoded)).into()
        }
        DefaultKind::Value(value) => value.to_string().into(),
    }
}

fn render_foreign_key_action(action: ForeignKeyAction) -> &'static str {
    match action {
        ForeignKeyAction::Cascade => "CASCADE",
        ForeignKeyAction::NoAction => "NO ACTION",
        ForeignKeyAction::Restrict => "RESTRICT",
        ForeignKeyAction::SetDefault => "SET DEFAULT",
        ForeignKeyAction::SetNull => "SET NULL",
    }
}

fn quoted_table(table: TableWalker<'_>) -> QuotedWithPrefix<&str> {
    QuotedWithPrefix::pg_from_table_walker(table)
}

fn quoted_index(index: IndexWalker<'_>) -> QuotedWithPrefix<&str> {
    QuotedWithPrefix::pg_new(index.table().explicit_namespace(), index.name())
}

fn sequence_name(table_name: &str, column_name: &str) -> String {
    format!("{table_name}_{column_name}_seq").to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_oracle_native_type_names() {
        assert_eq!(
            optional_argument_type(&KingbaseOracleType::TimestampTz(Some(3))),
            "TIMESTAMP(3) WITH TIME ZONE"
        );
        assert_eq!(
            optional_argument_type(&KingbaseOracleType::Number(
                KingbaseOracleNumberArguments::PrecisionAndScale(10, 2)
            )),
            "NUMBER(10,2)"
        );
    }

    #[test]
    fn renders_binary_defaults_as_decode_calls() {
        assert_eq!(
            render_default(&DefaultValue::value(PrismaValue::Bytes(vec![0xab, 0xcd]))),
            "decode('abcd', 'hex')"
        );
    }
}
