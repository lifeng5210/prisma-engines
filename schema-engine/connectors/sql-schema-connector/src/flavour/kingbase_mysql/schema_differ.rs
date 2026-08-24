use crate::migration_pair::MigrationPair;
use psl::builtin_connectors::KingbaseMySqlType;
use sql_schema_describer::walkers::{IndexWalker, TableColumnWalker};

use crate::sql_schema_differ::{SqlSchemaDifferFlavour, all_match, column::ColumnTypeChange};

/// Whether a type change can be represented by an explicit Kingbase `USING`
/// expression. This is deliberately separate from whether Kingbase performs
/// the conversion implicitly: the renderer uses this information to keep
/// executable conversions as ALTER COLUMN operations instead of planning a
/// destructive drop/recreate.
pub(crate) fn explicit_cast_supported(previous: Option<&KingbaseMySqlType>, next: Option<&KingbaseMySqlType>) -> bool {
    let (Some(previous), Some(next)) = (previous, next) else {
        return false;
    };

    if is_blob(previous) && !is_blob(next) {
        // BLOB values can be decoded as UTF-8 text, but there is no reliable
        // type-level conversion for arbitrary binary data to numeric, temporal,
        // or JSON values. Those changes must retain the destructive path.
        return is_string(next) || is_binary(next);
    }

    if is_json(previous) && (is_numeric(next) || is_datetime(next) || is_bit(next)) {
        return false;
    }

    if is_binary(previous) && (is_datetime(next) || is_json(next) || is_numeric(next) || is_bit(next)) {
        return false;
    }

    if is_numeric(previous) && is_datetime(next) {
        return false;
    }

    if is_numeric(previous) && matches!(next, KingbaseMySqlType::Year) {
        return false;
    }

    if is_datetime(previous) && (is_json(next) || is_bit(next)) {
        return false;
    }

    if is_datetime(previous) && is_numeric(next) && !datetime_numeric_cast_supported(previous, next) {
        return false;
    }

    // Numeric-to-JSON uses to_json(), and datetime-to-numeric uses to_char()
    // in the renderer; all remaining native families have an explicit cast
    // representation in the Kingbase type system.
    true
}

#[derive(Debug, Default)]
pub(crate) struct KingbaseMysqlSchemaDifferFlavour;

impl SqlSchemaDifferFlavour for KingbaseMysqlSchemaDifferFlavour {
    fn can_rename_foreign_key(&self) -> bool {
        false
    }

    fn can_rename_index(&self) -> bool {
        true
    }

    fn can_cope_with_foreign_key_column_becoming_non_nullable(&self) -> bool {
        false
    }

    fn column_type_change(&self, differ: MigrationPair<TableColumnWalker<'_>>) -> Option<ColumnTypeChange> {
        match (
            differ.previous.column_type_family_as_enum(),
            differ.next.column_type_family_as_enum(),
        ) {
            (Some(previous_enum), Some(next_enum)) => {
                if all_match(previous_enum.values(), next_enum.values()) {
                    return None;
                }

                return Some(
                    if previous_enum
                        .values()
                        .all(|previous_value| next_enum.values().any(|next_value| previous_value == next_value))
                    {
                        ColumnTypeChange::SafeCast
                    } else {
                        ColumnTypeChange::RiskyCast
                    },
                );
            }
            (Some(_), None) | (None, Some(_)) => return Some(ColumnTypeChange::RiskyCast),
            (None, None) => (),
        }

        if differ.previous.column_type().native_type == differ.next.column_type().native_type
            && differ.previous.column_type().family == differ.next.column_type().family
        {
            return None;
        }

        match (
            differ.previous.column_native_type::<KingbaseMySqlType>(),
            differ.next.column_native_type::<KingbaseMySqlType>(),
        ) {
            (Some(previous), Some(next)) if datetime_precision_zero_is_unspecified(previous, next) => return None,
            (Some(previous), Some(next))
                if matches!(
                    (previous, next),
                    (KingbaseMySqlType::Int, KingbaseMySqlType::VarChar(_))
                ) =>
            {
                return Some(ColumnTypeChange::SafeCast);
            }
            (Some(previous), Some(next)) if !explicit_cast_supported(Some(previous), Some(next)) => {
                return Some(ColumnTypeChange::NotCastable);
            }
            (Some(previous), Some(next)) if integer_signedness_is_only_difference(previous, next) => return None,
            (None, Some(KingbaseMySqlType::TinyInt)) | (Some(KingbaseMySqlType::TinyInt), None)
                if differ.previous.column_type_family().is_boolean()
                    && differ.next.column_type_family().is_boolean() =>
            {
                return None;
            }
            _ => (),
        }

        Some(ColumnTypeChange::RiskyCast)
    }

    fn index_should_be_renamed(&self, indexes: MigrationPair<IndexWalker<'_>>) -> bool {
        indexes.previous.name() != indexes.next.name()
    }

    fn lower_cases_table_names(&self) -> bool {
        false
    }

    fn should_create_indexes_from_created_tables(&self) -> bool {
        false
    }

    fn should_ignore_json_defaults(&self) -> bool {
        true
    }

    fn should_recreate_fks_covered_by_deleted_indexes(&self) -> bool {
        true
    }

    fn table_names_match(&self, names: MigrationPair<&str>) -> bool {
        names.previous == names.next
    }
}

fn integer_signedness_is_only_difference(previous: &KingbaseMySqlType, next: &KingbaseMySqlType) -> bool {
    matches!(
        (previous, next),
        (KingbaseMySqlType::Int, KingbaseMySqlType::UnsignedInt)
            | (KingbaseMySqlType::UnsignedInt, KingbaseMySqlType::Int)
            | (KingbaseMySqlType::SmallInt, KingbaseMySqlType::UnsignedSmallInt)
            | (KingbaseMySqlType::UnsignedSmallInt, KingbaseMySqlType::SmallInt)
            | (KingbaseMySqlType::TinyInt, KingbaseMySqlType::UnsignedTinyInt)
            | (KingbaseMySqlType::UnsignedTinyInt, KingbaseMySqlType::TinyInt)
            | (KingbaseMySqlType::MediumInt, KingbaseMySqlType::UnsignedMediumInt)
            | (KingbaseMySqlType::UnsignedMediumInt, KingbaseMySqlType::MediumInt)
            | (KingbaseMySqlType::BigInt, KingbaseMySqlType::UnsignedBigInt)
            | (KingbaseMySqlType::UnsignedBigInt, KingbaseMySqlType::BigInt)
    )
}

fn datetime_precision_zero_is_unspecified(previous: &KingbaseMySqlType, next: &KingbaseMySqlType) -> bool {
    matches!(
        (previous, next),
        (KingbaseMySqlType::Time(None), KingbaseMySqlType::Time(Some(0)))
            | (KingbaseMySqlType::Time(Some(0)), KingbaseMySqlType::Time(None))
            | (KingbaseMySqlType::DateTime(None), KingbaseMySqlType::DateTime(Some(0)))
            | (KingbaseMySqlType::DateTime(Some(0)), KingbaseMySqlType::DateTime(None))
            | (
                KingbaseMySqlType::Timestamp(None),
                KingbaseMySqlType::Timestamp(Some(0))
            )
            | (
                KingbaseMySqlType::Timestamp(Some(0)),
                KingbaseMySqlType::Timestamp(None)
            )
    )
}

fn is_blob(native_type: &KingbaseMySqlType) -> bool {
    matches!(
        native_type,
        KingbaseMySqlType::TinyBlob
            | KingbaseMySqlType::Blob
            | KingbaseMySqlType::MediumBlob
            | KingbaseMySqlType::LongBlob
    )
}

fn is_binary(native_type: &KingbaseMySqlType) -> bool {
    matches!(
        native_type,
        KingbaseMySqlType::Bit(_) | KingbaseMySqlType::Binary(_) | KingbaseMySqlType::VarBinary(_)
    ) || is_blob(native_type)
}

fn is_bit(native_type: &KingbaseMySqlType) -> bool {
    matches!(native_type, KingbaseMySqlType::Bit(_))
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

fn datetime_numeric_cast_supported(previous: &KingbaseMySqlType, next: &KingbaseMySqlType) -> bool {
    let required_digits = match previous {
        KingbaseMySqlType::Date => 8,
        KingbaseMySqlType::Time(_) => 6,
        KingbaseMySqlType::DateTime(_) | KingbaseMySqlType::Timestamp(_) => 14,
        _ => return false,
    };

    let capacity = match next {
        KingbaseMySqlType::TinyInt | KingbaseMySqlType::UnsignedTinyInt => 3,
        KingbaseMySqlType::SmallInt | KingbaseMySqlType::UnsignedSmallInt => 5,
        KingbaseMySqlType::MediumInt | KingbaseMySqlType::UnsignedMediumInt => 7,
        KingbaseMySqlType::Int | KingbaseMySqlType::UnsignedInt => 10,
        KingbaseMySqlType::BigInt | KingbaseMySqlType::UnsignedBigInt => 19,
        KingbaseMySqlType::Decimal(Some((precision, _))) => *precision,
        KingbaseMySqlType::Decimal(None) => 10,
        KingbaseMySqlType::Float | KingbaseMySqlType::Double => 38,
        KingbaseMySqlType::Year => 4,
        _ => return false,
    };

    capacity >= required_digits
}

fn is_json(native_type: &KingbaseMySqlType) -> bool {
    matches!(native_type, KingbaseMySqlType::Json)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_using_support_is_based_on_the_source_and_target_types() {
        assert!(explicit_cast_supported(
            Some(&KingbaseMySqlType::DateTime(Some(0))),
            Some(&KingbaseMySqlType::Double),
        ));
        assert!(explicit_cast_supported(
            Some(&KingbaseMySqlType::VarChar(32)),
            Some(&KingbaseMySqlType::Json),
        ));
        assert!(explicit_cast_supported(
            Some(&KingbaseMySqlType::Json),
            Some(&KingbaseMySqlType::Blob),
        ));
        assert!(explicit_cast_supported(
            Some(&KingbaseMySqlType::Blob),
            Some(&KingbaseMySqlType::VarChar(32)),
        ));
    }

    #[test]
    fn unsupported_value_family_changes_stay_destructive() {
        assert!(!explicit_cast_supported(
            Some(&KingbaseMySqlType::Blob),
            Some(&KingbaseMySqlType::Int),
        ));
        assert!(!explicit_cast_supported(
            Some(&KingbaseMySqlType::Json),
            Some(&KingbaseMySqlType::Double),
        ));
        assert!(!explicit_cast_supported(
            Some(&KingbaseMySqlType::Int),
            Some(&KingbaseMySqlType::Date),
        ));
        assert!(!explicit_cast_supported(
            Some(&KingbaseMySqlType::Binary(8)),
            Some(&KingbaseMySqlType::Int),
        ));
        assert!(!explicit_cast_supported(
            Some(&KingbaseMySqlType::Json),
            Some(&KingbaseMySqlType::Bit(64)),
        ));
        assert!(!explicit_cast_supported(
            Some(&KingbaseMySqlType::Date),
            Some(&KingbaseMySqlType::TinyInt),
        ));
        assert!(explicit_cast_supported(
            Some(&KingbaseMySqlType::Date),
            Some(&KingbaseMySqlType::Int),
        ));
    }
}
