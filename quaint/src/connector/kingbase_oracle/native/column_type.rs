use crate::connector::ColumnType;
use kingbase_tokio_postgres::types::{Kind, Type};

/// Maps Kingbase Oracle-mode PostgreSQL-wire OIDs to Quaint result types.
pub(crate) fn column_type_from_type(typ: &Type) -> ColumnType {
    if matches!(typ.kind(), Kind::Enum(_)) {
        ColumnType::Enum
    } else if typ == &Type::BOOL {
        ColumnType::Boolean
    } else if matches!(typ, &Type::INT2 | &Type::INT4 | &Type::ORACLE_TINYINT) {
        ColumnType::Int32
    } else if matches!(typ, &Type::INT8 | &Type::OID) {
        ColumnType::Int64
    } else if typ == &Type::FLOAT4 {
        ColumnType::Float
    } else if typ == &Type::FLOAT8 {
        ColumnType::Double
    } else if typ == &Type::NUMERIC {
        ColumnType::Numeric
    } else if is_oracle_text_type(typ) || typ == &Type::ORACLE_ROWID {
        // ROWID is exposed as text by the SQL text protocol (JS TypedSQL).
        // Its native binary payload is opaque, unlike UROWID/String.
        ColumnType::Text
    } else if matches!(typ, &Type::BYTEA | &Type::ORACLE_BLOB) {
        ColumnType::Bytes
    } else if matches!(typ, &Type::JSON | &Type::JSONB) {
        ColumnType::Json
    } else if typ == &Type::XML {
        ColumnType::Xml
    } else if typ == &Type::UUID {
        ColumnType::Uuid
    } else if typ == &Type::CHAR {
        // `pg_catalog."char"` appears in the PostgreSQL-compatible catalog
        // for enum and foreign-key metadata.
        ColumnType::Char
    } else if is_catalog_text_array(typ) {
        // PostgreSQL-compatible catalogs expose attributes such as reloptions
        // as text[]. This only supports decoding metadata returned by the
        // server; Oracle-mode bind parameters and Prisma scalar lists remain
        // unsupported.
        ColumnType::TextArray
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

pub(crate) fn is_catalog_text_array(typ: &Type) -> bool {
    matches!(typ, &Type::TEXT_ARRAY | &Type::NAME_ARRAY | &Type::VARCHAR_ARRAY)
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
    use super::{column_type_from_type, is_catalog_text_array, is_oracle_text_type};
    use crate::connector::ColumnType;
    use kingbase_tokio_postgres::types::Type;

    #[tokio::test]
    async fn describe_rowid_reports_text_for_typed_sql() {
        use crate::{prelude::Queryable, single::Quaint, tests::test_api::kingbase_oracle::CONN_STR};
        let connection = Quaint::new(CONN_STR.as_str()).await.unwrap();
        let described = connection
            .describe_query("SELECT nextrowid() AS \"rowId\", CAST(-128 AS TINYINT) AS \"tiny\"")
            .await
            .unwrap();
        assert_eq!(described.columns.len(), 2);
        assert_eq!(described.columns[0].typ, ColumnType::Text);
        assert_eq!(described.columns[0].name, "rowId");
        assert_eq!(described.columns[1].typ, ColumnType::Int32);
    }

    #[test]
    fn maps_oracle_mode_scalar_oids() {
        assert_eq!(column_type_from_type(&Type::NUMERIC), ColumnType::Numeric);
        assert_eq!(column_type_from_type(&Type::ORACLE_TINYINT), ColumnType::Int32);
        assert_eq!(column_type_from_type(&Type::ORACLE_BLOB), ColumnType::Bytes);
        assert_eq!(column_type_from_type(&Type::ORACLE_CLOB), ColumnType::Text);
        assert_eq!(column_type_from_type(&Type::ORACLE_NCLOB), ColumnType::Text);
        assert_eq!(column_type_from_type(&Type::ORACLE_SYS_DATE), ColumnType::DateTime);
        assert_eq!(column_type_from_type(&Type::ORACLE_ROWID), ColumnType::Text);
        assert_eq!(column_type_from_type(&Type::TEXT_ARRAY), ColumnType::TextArray);
    }

    #[test]
    fn recognises_oracle_character_types() {
        assert!(is_oracle_text_type(&Type::ORACLE_UROWID));
        assert!(is_oracle_text_type(&Type::ORACLE_BPCHARBYTE));
        assert!(is_oracle_text_type(&Type::ORACLE_VARCHARBYTE));
        assert!(!is_oracle_text_type(&Type::ORACLE_BLOB));
        assert!(is_catalog_text_array(&Type::TEXT_ARRAY));
        assert!(!is_catalog_text_array(&Type::BOOL_ARRAY));
    }
}
