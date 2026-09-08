use crate::connector::ColumnType;
use kingbase_tokio_postgres::types::Type;

/// Maps Kingbase Oracle-mode PostgreSQL-wire OIDs to Quaint result types.
pub(crate) fn column_type_from_type(typ: &Type) -> ColumnType {
    if typ == &Type::BOOL {
        ColumnType::Boolean
    } else if matches!(typ, &Type::INT2 | &Type::INT4) {
        ColumnType::Int32
    } else if matches!(typ, &Type::INT8 | &Type::OID) {
        ColumnType::Int64
    } else if typ == &Type::FLOAT4 {
        ColumnType::Float
    } else if typ == &Type::FLOAT8 {
        ColumnType::Double
    } else if typ == &Type::NUMERIC {
        ColumnType::Numeric
    } else if is_oracle_text_type(typ) {
        ColumnType::Text
    } else if matches!(typ, &Type::BYTEA | &Type::ORACLE_BLOB) {
        ColumnType::Bytes
    } else if matches!(typ, &Type::JSON | &Type::JSONB) {
        ColumnType::Json
    } else if typ == &Type::XML {
        ColumnType::Xml
    } else if typ == &Type::UUID {
        ColumnType::Uuid
    } else if matches!(typ, &Type::TIMESTAMP | &Type::TIMESTAMPTZ | &Type::ORACLE_SYS_DATE) {
        ColumnType::DateTime
    } else if typ == &Type::DATE {
        ColumnType::Date
    } else if matches!(typ, &Type::TIME | &Type::TIMETZ) {
        ColumnType::Time
    } else {
        ColumnType::Unknown
    }
}

pub(crate) fn is_oracle_text_type(typ: &Type) -> bool {
    matches!(
        typ,
        &Type::TEXT
            | &Type::VARCHAR
            | &Type::BPCHAR
            | &Type::NAME
            | &Type::UNKNOWN
            | &Type::ORACLE_UROWID
            | &Type::ORACLE_CLOB
            | &Type::ORACLE_NCLOB
            | &Type::ORACLE_BPCHARBYTE
            | &Type::ORACLE_VARCHARBYTE
            | &Type::ORACLE_BFILE
    )
}

#[cfg(test)]
mod tests {
    use super::{column_type_from_type, is_oracle_text_type};
    use crate::connector::ColumnType;
    use kingbase_tokio_postgres::types::Type;

    #[test]
    fn maps_oracle_mode_scalar_oids() {
        assert_eq!(column_type_from_type(&Type::NUMERIC), ColumnType::Numeric);
        assert_eq!(column_type_from_type(&Type::ORACLE_BLOB), ColumnType::Bytes);
        assert_eq!(column_type_from_type(&Type::ORACLE_CLOB), ColumnType::Text);
        assert_eq!(column_type_from_type(&Type::ORACLE_NCLOB), ColumnType::Text);
        assert_eq!(column_type_from_type(&Type::ORACLE_SYS_DATE), ColumnType::DateTime);
    }

    #[test]
    fn recognises_oracle_character_types() {
        assert!(is_oracle_text_type(&Type::ORACLE_UROWID));
        assert!(is_oracle_text_type(&Type::ORACLE_BPCHARBYTE));
        assert!(is_oracle_text_type(&Type::ORACLE_VARCHARBYTE));
        assert!(!is_oracle_text_type(&Type::ORACLE_BLOB));
    }
}
