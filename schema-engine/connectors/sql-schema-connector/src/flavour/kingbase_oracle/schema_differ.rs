use crate::{
    migration_pair::MigrationPair,
    sql_migration::{AlterEnum, SqlMigrationStep},
    sql_schema_differ::{SqlSchemaDifferFlavour, all_match, column::ColumnTypeChange, differ_database::DifferDatabase},
};
use psl::builtin_connectors::{KingbaseOracleNumberArguments, KingbaseOracleType};
use sql_schema_describer::walkers::{IndexWalker, TableColumnWalker};

/// Oracle-compatible migration-diff rules for KingbaseES.
#[derive(Debug, Default)]
pub(crate) struct KingbaseOracleSchemaDifferFlavour;

impl SqlSchemaDifferFlavour for KingbaseOracleSchemaDifferFlavour {
    fn can_rename_foreign_key(&self) -> bool {
        true
    }

    fn column_type_change(&self, columns: MigrationPair<TableColumnWalker<'_>>) -> Option<ColumnTypeChange> {
        match (
            columns.previous.column_type_family_as_enum(),
            columns.next.column_type_family_as_enum(),
        ) {
            (Some(previous), Some(next)) => {
                if all_match(previous.values(), next.values()) {
                    return None;
                }

                return Some(ColumnTypeChange::RiskyCast);
            }
            (Some(_), None) | (None, Some(_)) => return Some(ColumnTypeChange::NotCastable),
            (None, None) => (),
        }

        if columns.previous.column_type().native_type == columns.next.column_type().native_type
            && columns.previous.column_type().family == columns.next.column_type().family
        {
            return None;
        }

        match (
            columns.previous.column_native_type::<KingbaseOracleType>(),
            columns.next.column_native_type::<KingbaseOracleType>(),
        ) {
            (Some(previous), Some(next)) => oracle_type_change(*previous, *next),
            (None, None)
                if columns.previous.column_type().full_data_type == columns.next.column_type().full_data_type =>
            {
                None
            }
            _ => Some(ColumnTypeChange::RiskyCast),
        }
    }

    fn push_enum_steps(&self, steps: &mut Vec<SqlMigrationStep>, db: &DifferDatabase<'_>) {
        for enum_differ in db.enum_pairs() {
            let mut alter_enum = AlterEnum {
                id: enum_differ.enums.map(|enm| enm.id),
                created_variants: enum_differ.created_values().map(str::to_owned).collect(),
                dropped_variants: enum_differ.dropped_values().map(str::to_owned).collect(),
                previous_usages_as_default: Vec::new(),
            };

            if alter_enum.is_empty() {
                continue;
            }

            push_alter_enum_previous_usages_as_default(db, &mut alter_enum);
            steps.push(SqlMigrationStep::AlterEnum(alter_enum));
        }

        steps.extend(db.created_enums().map(|enm| SqlMigrationStep::CreateEnum(enm.id)));
        steps.extend(db.dropped_enums().map(|enm| SqlMigrationStep::DropEnum(enm.id)));
    }

    fn indexes_should_be_recreated_after_column_drop(&self) -> bool {
        true
    }

    fn index_should_be_renamed(&self, indexes: MigrationPair<IndexWalker<'_>>) -> bool {
        indexes.previous.name() != indexes.next.name()
    }

    fn should_ignore_json_defaults(&self) -> bool {
        true
    }

    fn should_recreate_fks_covered_by_deleted_indexes(&self) -> bool {
        true
    }
}

/// Removing or recreating an enum cannot keep defaults that reference the
/// previous enum type. Collect those columns so the renderer can drop the
/// defaults before the enum change and restore the still-valid ones after it.
fn push_alter_enum_previous_usages_as_default(db: &DifferDatabase<'_>, alter_enum: &mut AlterEnum) {
    let mut previous_usages_as_default: Vec<(_, Option<_>)> = Vec::new();
    let enum_names = db.schemas.walk(alter_enum.id).map(|enm| enm.name());

    for table in db.dropped_tables() {
        for column in table
            .columns()
            .filter(|column| column.column_type_is_enum(enum_names.previous) && column.default().is_some())
        {
            previous_usages_as_default.push((column.id, None));
        }
    }

    for tables in db.table_pairs() {
        for column in tables
            .dropped_columns()
            .filter(|column| column.column_type_is_enum(enum_names.previous) && column.default().is_some())
        {
            previous_usages_as_default.push((column.id, None));
        }

        for columns in tables.column_pairs().filter(|columns| {
            columns.previous.column_type_is_enum(enum_names.previous) && columns.previous.default().is_some()
        }) {
            let next_usage_as_default = Some(&columns.next)
                .filter(|column| column.column_type_is_enum(enum_names.next) && column.default().is_some())
                .map(|column| column.id);

            previous_usages_as_default.push((columns.previous.id, next_usage_as_default));
        }
    }

    alter_enum.previous_usages_as_default = previous_usages_as_default;
}

fn oracle_type_change(previous: KingbaseOracleType, next: KingbaseOracleType) -> Option<ColumnTypeChange> {
    use ColumnTypeChange::*;

    if previous == next
        || timestamp_precision_is_equivalent(previous, next)
        || catalog_types_are_equivalent(previous, next)
    {
        return None;
    }

    if is_lob_or_json(previous) && !is_character_or_lob(next) || is_lob_or_json(next) && !is_character_or_lob(previous)
    {
        return Some(NotCastable);
    }

    match (previous, next) {
        (KingbaseOracleType::Number(previous), KingbaseOracleType::Number(next)) => {
            Some(if number_can_represent(next, previous) {
                SafeCast
            } else {
                RiskyCast
            })
        }
        (
            KingbaseOracleType::Char(previous) | KingbaseOracleType::VarChar2(previous),
            KingbaseOracleType::Char(next) | KingbaseOracleType::VarChar2(next),
        ) => Some(if length_can_represent(next, previous) {
            SafeCast
        } else {
            RiskyCast
        }),
        (
            KingbaseOracleType::NChar(previous) | KingbaseOracleType::NVarChar2(previous),
            KingbaseOracleType::NChar(next) | KingbaseOracleType::NVarChar2(next),
        ) => Some(if length_can_represent(next, previous) {
            SafeCast
        } else {
            RiskyCast
        }),
        (KingbaseOracleType::Date, KingbaseOracleType::Timestamp(_) | KingbaseOracleType::TimestampTz(_)) => {
            Some(SafeCast)
        }
        (KingbaseOracleType::Timestamp(_) | KingbaseOracleType::TimestampTz(_), KingbaseOracleType::Date) => {
            Some(RiskyCast)
        }
        _ => Some(RiskyCast),
    }
}

/// Kingbase's PostgreSQL-compatible catalog canonicalizes several Oracle-mode
/// declarations. Treat those representations as equal during schema diffing so
/// a schema push does not emit a MODIFY on every subsequent run.
fn catalog_types_are_equivalent(previous: KingbaseOracleType, next: KingbaseOracleType) -> bool {
    use KingbaseOracleType::*;

    match (previous, next) {
        (Float(_), BinaryDouble) | (BinaryDouble, Float(_)) => true,
        (Char(previous), Char(next))
        | (Char(previous), NChar(next))
        | (NChar(previous), Char(next))
        | (NChar(previous), NChar(next)) => previous == next,
        (VarChar2(previous), VarChar2(next))
        | (VarChar2(previous), NVarChar2(next))
        | (NVarChar2(previous), VarChar2(next))
        | (NVarChar2(previous), NVarChar2(next)) => previous == next,
        (Clob, NClob) | (NClob, Clob) => true,
        (Date, Timestamp(None) | Timestamp(Some(0))) | (Timestamp(None) | Timestamp(Some(0)), Date) => true,
        (TimestampLocalTz(previous), Timestamp(next)) | (Timestamp(next), TimestampLocalTz(previous)) => {
            previous == next
        }
        _ => false,
    }
}

fn timestamp_precision_is_equivalent(previous: KingbaseOracleType, next: KingbaseOracleType) -> bool {
    matches!(
        (previous, next),
        (
            KingbaseOracleType::Timestamp(None),
            KingbaseOracleType::Timestamp(Some(6))
        ) | (
            KingbaseOracleType::Timestamp(Some(6)),
            KingbaseOracleType::Timestamp(None)
        ) | (
            KingbaseOracleType::TimestampTz(None),
            KingbaseOracleType::TimestampTz(Some(6))
        ) | (
            KingbaseOracleType::TimestampTz(Some(6)),
            KingbaseOracleType::TimestampTz(None)
        ) | (
            KingbaseOracleType::TimestampLocalTz(None),
            KingbaseOracleType::TimestampLocalTz(Some(6))
        ) | (
            KingbaseOracleType::TimestampLocalTz(Some(6)),
            KingbaseOracleType::TimestampLocalTz(None)
        )
    )
}

fn number_can_represent(target: KingbaseOracleNumberArguments, source: KingbaseOracleNumberArguments) -> bool {
    use KingbaseOracleNumberArguments::*;

    match (source, target) {
        (_, Unspecified) => true,
        (Unspecified, _) => false,
        (Precision(source_precision), Precision(target_precision)) => target_precision >= source_precision,
        (Precision(source_precision), PrecisionAndScale(target_precision, 0)) => target_precision >= source_precision,
        (PrecisionAndScale(source_precision, source_scale), Precision(target_precision)) => {
            source_scale == 0 && target_precision >= source_precision
        }
        (PrecisionAndScale(source_precision, source_scale), PrecisionAndScale(target_precision, target_scale)) => {
            target_precision - target_scale >= source_precision - source_scale && target_scale >= source_scale
        }
        (Precision(_), PrecisionAndScale(_, _)) => false,
    }
}

fn length_can_represent(next: Option<u32>, previous: Option<u32>) -> bool {
    match (previous, next) {
        (_, None) => true,
        (None, Some(_)) => false,
        (Some(previous), Some(next)) => next >= previous,
    }
}

fn is_lob_or_json(native_type: KingbaseOracleType) -> bool {
    matches!(
        native_type,
        KingbaseOracleType::Blob | KingbaseOracleType::Clob | KingbaseOracleType::NClob | KingbaseOracleType::Json
    )
}

fn is_character_or_lob(native_type: KingbaseOracleType) -> bool {
    matches!(
        native_type,
        KingbaseOracleType::Char(_)
            | KingbaseOracleType::VarChar2(_)
            | KingbaseOracleType::NChar(_)
            | KingbaseOracleType::NVarChar2(_)
            | KingbaseOracleType::Clob
            | KingbaseOracleType::NClob
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_widening_number_and_character_changes_are_safe() {
        assert_eq!(
            oracle_type_change(
                KingbaseOracleType::Number(KingbaseOracleNumberArguments::PrecisionAndScale(10, 2)),
                KingbaseOracleType::Number(KingbaseOracleNumberArguments::PrecisionAndScale(12, 2))
            ),
            Some(ColumnTypeChange::SafeCast)
        );
        assert_eq!(
            oracle_type_change(
                KingbaseOracleType::VarChar2(Some(40)),
                KingbaseOracleType::VarChar2(Some(30))
            ),
            Some(ColumnTypeChange::RiskyCast)
        );

        assert_eq!(
            oracle_type_change(
                KingbaseOracleType::Number(KingbaseOracleNumberArguments::PrecisionAndScale(12, 4)),
                KingbaseOracleType::Number(KingbaseOracleNumberArguments::Precision(12))
            ),
            Some(ColumnTypeChange::RiskyCast)
        );

        assert_eq!(
            oracle_type_change(
                KingbaseOracleType::Number(KingbaseOracleNumberArguments::PrecisionAndScale(12, 0)),
                KingbaseOracleType::Number(KingbaseOracleNumberArguments::Precision(12))
            ),
            Some(ColumnTypeChange::SafeCast)
        );
    }

    #[test]
    fn lob_to_non_character_types_are_not_castable() {
        assert_eq!(
            oracle_type_change(
                KingbaseOracleType::Blob,
                KingbaseOracleType::Number(KingbaseOracleNumberArguments::Precision(10))
            ),
            Some(ColumnTypeChange::NotCastable)
        );
    }

    #[test]
    fn catalog_canonicalized_oracle_types_are_idempotent() {
        assert_eq!(
            oracle_type_change(KingbaseOracleType::NChar(Some(10)), KingbaseOracleType::Char(Some(10))),
            None
        );
        assert_eq!(
            oracle_type_change(
                KingbaseOracleType::NVarChar2(Some(32)),
                KingbaseOracleType::VarChar2(Some(32))
            ),
            None
        );
        assert_eq!(
            oracle_type_change(KingbaseOracleType::NClob, KingbaseOracleType::Clob),
            None
        );
        assert_eq!(
            oracle_type_change(KingbaseOracleType::Timestamp(Some(0)), KingbaseOracleType::Date),
            None
        );
        assert_eq!(
            oracle_type_change(
                KingbaseOracleType::Timestamp(Some(3)),
                KingbaseOracleType::TimestampLocalTz(Some(3))
            ),
            None
        );
        assert_eq!(
            oracle_type_change(KingbaseOracleType::BinaryDouble, KingbaseOracleType::Float(None)),
            None
        );
    }
}
