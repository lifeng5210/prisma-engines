use crate::connector::ColumnType;

use kingbase_tokio_postgres::{
    Column as KingbaseColumn,
    types::{Kind as KingbaseKind, Type as KingbaseType},
};
use std::borrow::Cow;

/// Kingbase's MySQL-compatible BIT type stores the declared bit width directly
/// in typmod. A negative typmod means that no fixed length was declared.
pub(crate) fn mysql_bit_length_from_type_modifier(type_modifier: i32) -> Option<u64> {
    u64::try_from(type_modifier).ok().filter(|length| *length > 0)
}

pub(crate) fn is_mysql_bit_one(column: &KingbaseColumn) -> bool {
    column.type_() == &KingbaseType::MYSQL_SYS_BIT
        && mysql_bit_length_from_type_modifier(column.type_modifier()) == Some(1)
}

pub(crate) fn column_type_from_column(column: &KingbaseColumn) -> ColumnType {
    if is_mysql_bit_one(column) {
        ColumnType::Boolean
    } else {
        ColumnType::from(column.type_())
    }
}

macro_rules! create_pg_mapping {
  (
    $($key:ident($typ: ty) => [$($value:ident),+]),* $(,)?
    $([$pg_only_key:ident => $column_type_mapping:ident]),*
  ) => {
      // Generate KingbaseColumnType<Type> enums
      $(
          concat_idents::concat_idents!(enum_name = KingbaseColumnType, $key {
            #[derive(Debug)]
            #[allow(non_camel_case_types)]
            #[allow(clippy::upper_case_acronyms)]
            pub(crate) enum enum_name {
                $($value,)*
            }
        });
      )*

      // Generate validators
      $(
        concat_idents::concat_idents!(struct_name = KingbaseColumnValidator, $key {
            #[derive(Debug)]
            #[allow(non_camel_case_types)]
            pub struct struct_name;

            impl struct_name {
                #[inline]
                #[allow(clippy::extra_unused_lifetimes)]
                pub fn read<'a>(&self, val: $typ) -> $typ {
                    val
                }
            }
        });
    )*

      pub(crate) enum KingbaseColumnType {
        $(
          $key(
            concat_idents::concat_idents!(variant = KingbaseColumnType, $key, { variant }),
            concat_idents::concat_idents!(enum_name = KingbaseColumnValidator, $key, { enum_name })
          ),
        )*
        $($pg_only_key(concat_idents::concat_idents!(enum_name = KingbaseColumnValidator, $column_type_mapping, { enum_name })),)*
      }

      impl KingbaseColumnType {
          /// Takes a Postgres type and returns the corresponding ColumnType
          #[deny(unreachable_patterns)]
          pub(crate) fn from_pg_type(ty: &KingbaseType) -> KingbaseColumnType {
              match ty {
                  $(
                      $(
                        &KingbaseType::$value => KingbaseColumnType::$key(
                          concat_idents::concat_idents!(variant = KingbaseColumnType, $key, { variant::$value }),
                          concat_idents::concat_idents!(enum_name = KingbaseColumnValidator, $key, { enum_name }),
                        ),
                      )*
                  )*
                  ref x => match x.kind() {
                      KingbaseKind::Enum(_) | KingbaseKind::MySqlEnum(_) => {
                          KingbaseColumnType::Enum(KingbaseColumnValidatorText)
                      }
                      KingbaseKind::MySqlSet => KingbaseColumnType::Set(KingbaseColumnValidatorText),
                      KingbaseKind::Array(inner) => match inner.kind() {
                          KingbaseKind::Enum(_) | KingbaseKind::MySqlEnum(_) => {
                              KingbaseColumnType::EnumArray(KingbaseColumnValidatorTextArray)
                          }
                          KingbaseKind::MySqlSet => {
                              KingbaseColumnType::SetArray(KingbaseColumnValidatorTextArray)
                          }
                          _ => KingbaseColumnType::UnknownArray(KingbaseColumnValidatorTextArray),
                      },
                      _ => KingbaseColumnType::Unknown(KingbaseColumnValidatorText),
                  },
              }
          }
      }

      impl From<KingbaseColumnType> for ColumnType {
          fn from(ty: KingbaseColumnType) -> ColumnType {
              match ty {
                  $(
                      KingbaseColumnType::$key(..) => ColumnType::$key,
                  )*
                  $(
                      KingbaseColumnType::$pg_only_key(..) => ColumnType::$column_type_mapping,
                  )*
              }
          }
      }

      impl From<&KingbaseType> for ColumnType {
          fn from(ty: &KingbaseType) -> ColumnType {
              // MySQL ENUMs are exposed as a first-class enum by the MySQL
              // connector. Keep PostgreSQL enums on their existing Text path.
              if matches!(ty.kind(), KingbaseKind::MySqlEnum(_)) {
                  ColumnType::Enum
              } else {
                  KingbaseColumnType::from_pg_type(&ty).into()
              }
          }
      }
  };
}

// Create a mapping between Postgres types and ColumnType and ensures there's a single source of truth.
// ColumnType(<accepted data>) => [KingbaseType(s)...]
create_pg_mapping! {
  Boolean(Option<bool>) => [BOOL],
  Int32(Option<i32>) => [INT2, INT4, MYSQL_TINYINT, MYSQL_INT1, MYSQL_INT3, MYSQL_MEDIUMINT, MYSQL_MIDDLEINT, MYSQL_YEAR],
  Int64(Option<i64>) => [INT8, OID, MYSQL_UINT4, MYSQL_UINT8],
  Float(Option<f32>) => [FLOAT4],
  Double(Option<f64>) => [FLOAT8],
  Bytes(Option<Cow<'a, [u8]>>) => [BYTEA, MYSQL_BINARY, MYSQL_VARBINARY, MYSQL_SYS_BIT, MYSQL_BLOB, MYSQL_LONGBLOB, MYSQL_MEDIUMBLOB, MYSQL_TINYBLOB],
  Numeric(Option<bigdecimal::BigDecimal>) => [NUMERIC, MONEY],
  DateTime(Option<chrono::DateTime<chrono::Utc>>) => [TIMESTAMP, TIMESTAMPTZ, MYSQL_DATETIME, MYSQL_SYS_TIMESTAMP],
  Date(Option<chrono::NaiveDate>) => [DATE, MYSQL_SYS_DATE],
  Time(Option<chrono::NaiveTime>) => [TIME, TIMETZ, MYSQL_SYS_TIME],
  Text(Option<Cow<'a, str>>) => [INET, CIDR, BIT, VARBIT, MYSQL_LONGTEXT, MYSQL_MEDIUMTEXT, MYSQL_TINYTEXT, MYSQL_BPCHARBYTE, MYSQL_VARCHARBYTE],
  Uuid(Option<uuid::Uuid>) => [UUID],
  Json(Option<serde_json::Value>) => [JSON, JSONB, MYSQL_SYS_JSON],
  Xml(Option<Cow<'a, str>>) => [XML],
  Char(Option<char>) => [CHAR],

  BooleanArray(impl Iterator<Item = Option<bool>>) => [BOOL_ARRAY],
  Int32Array(impl Iterator<Item = Option<i32>>) => [INT2_ARRAY, INT4_ARRAY],
  Int64Array(impl Iterator<Item =Option<i64>>) => [INT8_ARRAY, OID_ARRAY],
  FloatArray(impl Iterator<Item = Option<f32>>) => [FLOAT4_ARRAY],
  DoubleArray(impl Iterator<Item = Option<f64>>) => [FLOAT8_ARRAY],
  BytesArray(impl Iterator<Item = Option<Vec<u8>>>) => [BYTEA_ARRAY],
  NumericArray(impl Iterator<Item = Option<bigdecimal::BigDecimal>>) => [NUMERIC_ARRAY, MONEY_ARRAY],
  DateTimeArray(impl Iterator<Item = Option<chrono::DateTime<chrono::Utc>>>) => [TIMESTAMP_ARRAY, TIMESTAMPTZ_ARRAY],
  DateArray(impl Iterator<Item = Option<chrono::NaiveDate>>) => [DATE_ARRAY],
  TimeArray(impl Iterator<Item = Option<chrono::NaiveTime>>) => [TIME_ARRAY, TIMETZ_ARRAY],
  TextArray(impl Iterator<Item = Option<Cow<'a, str>>>) => [TEXT_ARRAY, NAME_ARRAY, VARCHAR_ARRAY, INET_ARRAY, CIDR_ARRAY, BIT_ARRAY, VARBIT_ARRAY, XML_ARRAY],
  UuidArray(impl Iterator<Item = Option<uuid::Uuid>>) => [UUID_ARRAY],
  JsonArray(impl Iterator<Item = Option<serde_json::Value>>) => [JSON_ARRAY, JSONB_ARRAY],

  // For the cases where the Postgres type is not directly mappable to ColumnType, use the following:
  // [KingbaseColumnType => ColumnType]
  [Enum => Text],
  [EnumArray => TextArray],
  [Set => Text],
  [SetArray => TextArray],
  [UnknownArray => TextArray],
  [Unknown => Text]
}

#[cfg(test)]
mod tests {
    use super::{ColumnType, KingbaseColumnType, mysql_bit_length_from_type_modifier};
    use kingbase_tokio_postgres::types::{Kind, Type};

    #[test]
    fn mysql_bit_typmod_is_the_declared_bit_width() {
        assert_eq!(mysql_bit_length_from_type_modifier(1), Some(1));
        assert_eq!(mysql_bit_length_from_type_modifier(8), Some(8));
        assert_eq!(mysql_bit_length_from_type_modifier(-1), None);
    }

    #[test]
    fn mysql_unsigned_types_are_int64() {
        assert_eq!(ColumnType::from(&Type::MYSQL_UINT4), ColumnType::Int64);
        assert_eq!(ColumnType::from(&Type::MYSQL_UINT8), ColumnType::Int64);
    }

    #[test]
    fn mysql_int1_is_int32() {
        assert_eq!(ColumnType::from(&Type::MYSQL_INT1), ColumnType::Int32);
    }

    #[test]
    fn mysql_set_is_text_and_mysql_enum_is_enum() {
        let set_type = Type::new("set_type".to_owned(), 100_001, Kind::MySqlSet, "sys".to_owned());
        let enum_type = Type::new(
            "enum_type".to_owned(),
            100_002,
            Kind::MySqlEnum(vec!["draft".to_owned(), "published".to_owned()]),
            "sys".to_owned(),
        );

        assert_eq!(ColumnType::from(&set_type), ColumnType::Text);
        assert_eq!(ColumnType::from(&enum_type), ColumnType::Enum);
        assert!(matches!(
            KingbaseColumnType::from_pg_type(&set_type),
            KingbaseColumnType::Set(_)
        ));
        assert!(matches!(
            KingbaseColumnType::from_pg_type(&enum_type),
            KingbaseColumnType::Enum(_)
        ));
    }
}
