use crate::common::*;
use psl::{
    builtin_connectors::{KINGBASE_MYSQL, KingbaseMySqlType},
    datamodel_connector::NativeTypeInstance,
    parser_database::ScalarType,
};

#[test]
fn text_type_should_fail_on_unique() {
    let schema = indoc! {r#"
        datasource db {
          provider = "kingbase-mysql"
        }

        model User {
          id        Int    @id
          firstName String @db.Text
          lastName  String @db.Text

          @@unique([firstName, lastName])
        }
    "#};

    let expectation = expect![[r#"
        error: Native type `Text` cannot be unique in Kingbase MySQL. Please use the `length` argument to the field in the index definition to allow this.
          -->  schema.prisma:10
           | 
         9 | 
        10 |   @@unique([firstName, lastName])
           | 
    "#]];

    expect_error(schema, &expectation);
}

#[test]
fn longtext_type_should_fail_on_unique() {
    let schema = indoc! {r#"
        datasource db {
          provider = "kingbase-mysql"
        }

        model User {
          id        Int    @id
          firstName String @db.LongText
          lastName  String @db.LongText

          @@unique([firstName, lastName])
        }
    "#};

    let expectation = expect![[r#"
        error: Native type `LongText` cannot be unique in Kingbase MySQL. Please use the `length` argument to the field in the index definition to allow this.
          -->  schema.prisma:10
           | 
         9 | 
        10 |   @@unique([firstName, lastName])
           | 
    "#]];

    expect_error(schema, &expectation);
}

#[test]
fn mediumtext_type_should_fail_on_unique() {
    let schema = indoc! {r#"
        datasource db {
          provider = "kingbase-mysql"
        }

        model User {
          id        Int    @id
          firstName String @db.MediumText
          lastName  String @db.MediumText

          @@unique([firstName, lastName])
        }
    "#};

    let expectation = expect![[r#"
        error: Native type `MediumText` cannot be unique in Kingbase MySQL. Please use the `length` argument to the field in the index definition to allow this.
          -->  schema.prisma:10
           | 
         9 | 
        10 |   @@unique([firstName, lastName])
           | 
    "#]];

    expect_error(schema, &expectation);
}

#[test]
fn tinytext_type_should_fail_on_unique() {
    let schema = indoc! {r#"
        datasource db {
          provider = "kingbase-mysql"
        }

        model User {
          id        Int    @id
          firstName String @db.TinyText
          lastName  String @db.TinyText

          @@unique([firstName, lastName])
        }
    "#};

    let expectation = expect![[r#"
        error: Native type `TinyText` cannot be unique in Kingbase MySQL. Please use the `length` argument to the field in the index definition to allow this.
          -->  schema.prisma:10
           | 
         9 | 
        10 |   @@unique([firstName, lastName])
           | 
    "#]];

    expect_error(schema, &expectation);
}

#[test]
fn blob_type_should_fail_on_unique() {
    let schema = indoc! {r#"
        datasource db {
          provider = "kingbase-mysql"
        }

        model User {
          id        Int   @id
          firstName Bytes @db.Blob
          lastName  Bytes @db.Blob

          @@unique([firstName, lastName])
        }
    "#};

    let expectation = expect![[r#"
        error: Native type `Blob` cannot be unique in Kingbase MySQL. Please use the `length` argument to the field in the index definition to allow this.
          -->  schema.prisma:10
           | 
         9 | 
        10 |   @@unique([firstName, lastName])
           | 
    "#]];

    expect_error(schema, &expectation);
}

#[test]
fn longblob_type_should_fail_on_unique() {
    let schema = indoc! {r#"
        datasource db {
          provider = "kingbase-mysql"
        }

        model User {
          id        Int   @id
          firstName Bytes @db.LongBlob
          lastName  Bytes @db.LongBlob

          @@unique([firstName, lastName])
        }
    "#};

    let expectation = expect![[r#"
        error: Native type `LongBlob` cannot be unique in Kingbase MySQL. Please use the `length` argument to the field in the index definition to allow this.
          -->  schema.prisma:10
           | 
         9 | 
        10 |   @@unique([firstName, lastName])
           | 
    "#]];

    expect_error(schema, &expectation);
}

#[test]
fn mediumblob_type_should_fail_on_unique() {
    let schema = indoc! {r#"
        datasource db {
          provider = "kingbase-mysql"
        }

        model User {
          id        Int   @id
          firstName Bytes @db.MediumBlob
          lastName  Bytes @db.MediumBlob

          @@unique([firstName, lastName])
        }
    "#};

    let expectation = expect![[r#"
        error: Native type `MediumBlob` cannot be unique in Kingbase MySQL. Please use the `length` argument to the field in the index definition to allow this.
          -->  schema.prisma:10
           | 
         9 | 
        10 |   @@unique([firstName, lastName])
           | 
    "#]];

    expect_error(schema, &expectation);
}

#[test]
fn tinyblob_type_should_fail_on_unique() {
    let schema = indoc! {r#"
        datasource db {
          provider = "kingbase-mysql"
        }

        model User {
          id        Int   @id
          firstName Bytes @db.TinyBlob
          lastName  Bytes @db.TinyBlob

          @@unique([firstName, lastName])
        }
    "#};

    let expectation = expect![[r#"
        error: Native type `TinyBlob` cannot be unique in Kingbase MySQL. Please use the `length` argument to the field in the index definition to allow this.
          -->  schema.prisma:10
           | 
         9 | 
        10 |   @@unique([firstName, lastName])
           | 
    "#]];

    expect_error(schema, &expectation);
}

#[test]
fn text_type_should_fail_on_index() {
    let schema = indoc! {r#"
        datasource db {
          provider = "kingbase-mysql"
        }

        model User {
          id        Int    @id
          firstName String @db.Text
          lastName  String @db.Text

          @@index([firstName, lastName])
        }
    "#};

    let expectation = expect![[r#"
        error: You cannot define an index on fields with native type `Text` of Kingbase MySQL. Please use the `length` argument to the field in the index definition to allow this.
          -->  schema.prisma:10
           | 
         9 | 
        10 |   @@index([firstName, lastName])
           | 
    "#]];

    expect_error(schema, &expectation);
}

#[test]
fn longtext_type_should_fail_on_index() {
    let schema = indoc! {r#"
        datasource db {
          provider = "kingbase-mysql"
        }

        model User {
          id        Int    @id
          firstName String @db.LongText
          lastName  String @db.LongText

          @@index([firstName, lastName])
        }
    "#};

    let expectation = expect![[r#"
        error: You cannot define an index on fields with native type `LongText` of Kingbase MySQL. Please use the `length` argument to the field in the index definition to allow this.
          -->  schema.prisma:10
           | 
         9 | 
        10 |   @@index([firstName, lastName])
           | 
    "#]];

    expect_error(schema, &expectation);
}

#[test]
fn mediumtext_type_should_fail_on_index() {
    let schema = indoc! {r#"
        datasource db {
          provider = "kingbase-mysql"
        }

        model User {
          id        Int    @id
          firstName String @db.MediumText
          lastName  String @db.MediumText

          @@index([firstName, lastName])
        }
    "#};

    let expectation = expect![[r#"
        error: You cannot define an index on fields with native type `MediumText` of Kingbase MySQL. Please use the `length` argument to the field in the index definition to allow this.
          -->  schema.prisma:10
           | 
         9 | 
        10 |   @@index([firstName, lastName])
           | 
    "#]];

    expect_error(schema, &expectation);
}

#[test]
fn tinytext_type_should_fail_on_index() {
    let schema = indoc! {r#"
        datasource db {
          provider = "kingbase-mysql"
        }

        model User {
          id        Int    @id
          firstName String @db.TinyText
          lastName  String @db.TinyText

          @@index([firstName, lastName])
        }
    "#};

    let expectation = expect![[r#"
        error: You cannot define an index on fields with native type `TinyText` of Kingbase MySQL. Please use the `length` argument to the field in the index definition to allow this.
          -->  schema.prisma:10
           | 
         9 | 
        10 |   @@index([firstName, lastName])
           | 
    "#]];

    expect_error(schema, &expectation);
}

#[test]
fn blob_type_should_fail_on_index() {
    let schema = indoc! {r#"
        datasource db {
          provider = "kingbase-mysql"
        }

        model User {
          id        Int   @id
          firstName Bytes @db.Blob
          lastName  Bytes @db.Blob

          @@index([firstName, lastName])
        }
    "#};

    let expectation = expect![[r#"
        error: You cannot define an index on fields with native type `Blob` of Kingbase MySQL. Please use the `length` argument to the field in the index definition to allow this.
          -->  schema.prisma:10
           | 
         9 | 
        10 |   @@index([firstName, lastName])
           | 
    "#]];

    expect_error(schema, &expectation);
}

#[test]
fn longblob_type_should_fail_on_index() {
    let schema = indoc! {r#"
        datasource db {
          provider = "kingbase-mysql"
        }

        model User {
          id        Int   @id
          firstName Bytes @db.LongBlob
          lastName  Bytes @db.LongBlob

          @@index([firstName, lastName])
        }
    "#};

    let expectation = expect![[r#"
        error: You cannot define an index on fields with native type `LongBlob` of Kingbase MySQL. Please use the `length` argument to the field in the index definition to allow this.
          -->  schema.prisma:10
           | 
         9 | 
        10 |   @@index([firstName, lastName])
           | 
    "#]];

    expect_error(schema, &expectation);
}

#[test]
fn mediumblob_type_should_fail_on_index() {
    let schema = indoc! {r#"
        datasource db {
          provider = "kingbase-mysql"
        }

        model User {
          id        Int   @id
          firstName Bytes @db.MediumBlob
          lastName  Bytes @db.MediumBlob

          @@index([firstName, lastName])
        }
    "#};

    let expectation = expect![[r#"
        error: You cannot define an index on fields with native type `MediumBlob` of Kingbase MySQL. Please use the `length` argument to the field in the index definition to allow this.
          -->  schema.prisma:10
           | 
         9 | 
        10 |   @@index([firstName, lastName])
           | 
    "#]];

    expect_error(schema, &expectation);
}

#[test]
fn tinyblob_type_should_fail_on_index() {
    let schema = indoc! {r#"
        datasource db {
          provider = "kingbase-mysql"
        }

        model User {
          id        Int   @id
          firstName Bytes @db.TinyBlob
          lastName  Bytes @db.TinyBlob

          @@index([firstName, lastName])
        }
    "#};

    let expectation = expect![[r#"
        error: You cannot define an index on fields with native type `TinyBlob` of Kingbase MySQL. Please use the `length` argument to the field in the index definition to allow this.
          -->  schema.prisma:10
           | 
         9 | 
        10 |   @@index([firstName, lastName])
           | 
    "#]];

    expect_error(schema, &expectation);
}

#[test]
fn text_type_should_fail_on_id() {
    let schema = indoc! {r#"
        datasource db {
          provider = "kingbase-mysql"
        }

        model User {
          firstName String @db.Text
          lastName  String @db.Text

          @@id([firstName, lastName])
        }
    "#};

    let expectation = expect![[r#"
        error: Native type `Text` of Kingbase MySQL cannot be used on a field that is `@id` or `@@id`. Please use the `length` argument to the field in the index definition to allow this.
          -->  schema.prisma:9
           | 
         8 | 
         9 |   @@id([firstName, lastName])
           | 
    "#]];

    expect_error(schema, &expectation);
}

#[test]
fn longtext_type_should_fail_on_id() {
    let schema = indoc! {r#"
        datasource db {
          provider = "kingbase-mysql"
        }

        model User {
          firstName String @db.LongText
          lastName  String @db.LongText

          @@id([firstName, lastName])
        }
    "#};

    let expectation = expect![[r#"
        error: Native type `LongText` of Kingbase MySQL cannot be used on a field that is `@id` or `@@id`. Please use the `length` argument to the field in the index definition to allow this.
          -->  schema.prisma:9
           | 
         8 | 
         9 |   @@id([firstName, lastName])
           | 
    "#]];

    expect_error(schema, &expectation);
}

#[test]
fn mediumtext_type_should_fail_on_id() {
    let schema = indoc! {r#"
        datasource db {
          provider = "kingbase-mysql"
        }

        model User {
          firstName String @db.MediumText
          lastName  String @db.MediumText

          @@id([firstName, lastName])
        }
    "#};

    let expectation = expect![[r#"
        error: Native type `MediumText` of Kingbase MySQL cannot be used on a field that is `@id` or `@@id`. Please use the `length` argument to the field in the index definition to allow this.
          -->  schema.prisma:9
           | 
         8 | 
         9 |   @@id([firstName, lastName])
           | 
    "#]];

    expect_error(schema, &expectation);
}

#[test]
fn tinytext_type_should_fail_on_id() {
    let schema = indoc! {r#"
        datasource db {
          provider = "kingbase-mysql"
        }

        model User {
          firstName String @db.TinyText
          lastName  String @db.TinyText

          @@id([firstName, lastName])
        }
    "#};

    let expectation = expect![[r#"
        error: Native type `TinyText` of Kingbase MySQL cannot be used on a field that is `@id` or `@@id`. Please use the `length` argument to the field in the index definition to allow this.
          -->  schema.prisma:9
           | 
         8 | 
         9 |   @@id([firstName, lastName])
           | 
    "#]];

    expect_error(schema, &expectation);
}

#[test]
fn blob_type_should_fail_on_id() {
    let schema = indoc! {r#"
        datasource db {
          provider = "kingbase-mysql"
        }

        model User {
          firstName Bytes @db.Blob
          lastName  Bytes @db.Blob

          @@id([firstName, lastName])
        }
    "#};

    let expectation = expect![[r#"
        error: Native type `Blob` of Kingbase MySQL cannot be used on a field that is `@id` or `@@id`. Please use the `length` argument to the field in the index definition to allow this.
          -->  schema.prisma:9
           | 
         8 | 
         9 |   @@id([firstName, lastName])
           | 
    "#]];

    expect_error(schema, &expectation);
}

#[test]
fn longblob_type_should_fail_on_id() {
    let schema = indoc! {r#"
        datasource db {
          provider = "kingbase-mysql"
        }

        model User {
          firstName Bytes @db.LongBlob
          lastName  Bytes @db.LongBlob

          @@id([firstName, lastName])
        }
    "#};

    let expectation = expect![[r#"
        error: Native type `LongBlob` of Kingbase MySQL cannot be used on a field that is `@id` or `@@id`. Please use the `length` argument to the field in the index definition to allow this.
          -->  schema.prisma:9
           | 
         8 | 
         9 |   @@id([firstName, lastName])
           | 
    "#]];

    expect_error(schema, &expectation);
}

#[test]
fn mediumblob_type_should_fail_on_id() {
    let schema = indoc! {r#"
        datasource db {
          provider = "kingbase-mysql"
        }

        model User {
          firstName Bytes @db.MediumBlob
          lastName  Bytes @db.MediumBlob

          @@id([firstName, lastName])
        }
    "#};

    let expectation = expect![[r#"
        error: Native type `MediumBlob` of Kingbase MySQL cannot be used on a field that is `@id` or `@@id`. Please use the `length` argument to the field in the index definition to allow this.
          -->  schema.prisma:9
           | 
         8 | 
         9 |   @@id([firstName, lastName])
           | 
    "#]];

    expect_error(schema, &expectation);
}

#[test]
fn tinyblob_type_should_fail_on_id() {
    let schema = indoc! {r#"
        datasource db {
          provider = "kingbase-mysql"
        }

        model User {
          firstName Bytes @db.TinyBlob
          lastName  Bytes @db.TinyBlob

          @@id([firstName, lastName])
        }
    "#};

    let expectation = expect![[r#"
        error: Native type `TinyBlob` of Kingbase MySQL cannot be used on a field that is `@id` or `@@id`. Please use the `length` argument to the field in the index definition to allow this.
          -->  schema.prisma:9
           | 
         8 | 
         9 |   @@id([firstName, lastName])
           | 
    "#]];

    expect_error(schema, &expectation);
}

#[test]
fn text_should_not_fail_on_length_prefixed_index() {
    let dml = indoc! {r#"
        datasource db {
          provider = "kingbase-mysql"
        }

        model A {
          id Int    @id
          a  String @db.Text

          @@index([a(length: 30)])
        }
    "#};

    assert_valid(dml)
}

#[test]
fn text_should_not_fail_on_length_prefixed_unique() {
    let dml = indoc! {r#"
        datasource db {
          provider = "kingbase-mysql"
        }

        model A {
          id Int    @id
          a  String @db.Text @unique(length: 30)
        }
    "#};

    assert_valid(dml)
}

#[test]
fn text_should_not_fail_on_length_prefixed_pk() {
    let dml = indoc! {r#"
        datasource db {
          provider = "kingbase-mysql"
        }

        model A {
          id String @id(length: 30) @db.Text
        }
    "#};

    assert_valid(dml)
}

#[test]
fn bytes_should_not_fail_on_length_prefixed_index() {
    let dml = indoc! {r#"
        datasource db {
          provider = "kingbase-mysql"
        }

        model A {
          id Int   @id
          a  Bytes @db.Blob

          @@index([a(length: 30)])
        }
    "#};

    assert_valid(dml)
}

#[test]
fn bytes_should_not_fail_on_length_prefixed_unique() {
    let dml = indoc! {r#"
        datasource db {
          provider = "kingbase-mysql"
        }

        model A {
          id Int   @id
          a  Bytes @db.Blob @unique(length: 30)
        }
    "#};

    assert_valid(dml)
}

#[test]
fn bytes_should_not_fail_on_length_prefixed_pk() {
    let dml = indoc! {r#"
        datasource db {
          provider = "kingbase-mysql"
        }

        model A {
          id Bytes @id(length: 30) @db.Blob
        }
    "#};

    assert_valid(dml)
}

#[test]
fn should_fail_on_argument_for_bit_0_type() {
    let schema = indoc! {r#"
        datasource db {
          provider = "kingbase-mysql"
        }

        model User {
          id        Int   @id
          firstName Bytes @db.Bit(0)
        }
    "#};

    let expectation = expect![[r#"
        error: Argument M is out of range for native type `Bit(0)` of Kingbase MySQL: M can range from 1 to 64.
          -->  schema.prisma:7
           | 
         6 |   id        Int   @id
         7 |   firstName Bytes @db.Bit(0)
           | 
    "#]];

    expect_error(schema, &expectation);
}

#[test]
fn should_fail_on_argument_for_bit_65_type() {
    let schema = indoc! {r#"
        datasource db {
          provider = "kingbase-mysql"
        }

        model User {
          id        Int   @id
          firstName Bytes @db.Bit(65)
        }
    "#};

    let expectation = expect![[r#"
        error: Argument M is out of range for native type `Bit(65)` of Kingbase MySQL: M can range from 1 to 64.
          -->  schema.prisma:7
           | 
         6 |   id        Int   @id
         7 |   firstName Bytes @db.Bit(65)
           | 
    "#]];

    expect_error(schema, &expectation);
}

#[test]
fn should_only_allow_bit_one_for_booleans() {
    let schema = indoc! {r#"
        datasource db {
          provider = "kingbase-mysql"
        }

        model User {
          id        Int     @id
          firstName Boolean @db.Bit(2)
        }
    "#};

    let expectation = expect![[r#"
        error: Argument M is out of range for native type `Bit(2)` of Kingbase MySQL: only Bit(1) can be used as Boolean.
          -->  schema.prisma:7
           | 
         6 |   id        Int     @id
         7 |   firstName Boolean @db.Bit(2)
           | 
    "#]];

    expect_error(schema, &expectation);
}

#[test]
fn should_fail_on_argument_out_of_range_for_char_type() {
    let schema = indoc! {r#"
        datasource db {
          provider = "kingbase-mysql"
        }

        model User {
          id        Int    @id
          firstName String @db.Char(256)
        }
    "#};

    let expectation = expect![[r#"
        error: Argument M is out of range for native type `Char(256)` of Kingbase MySQL: M can range from 0 to 255.
          -->  schema.prisma:7
           | 
         6 |   id        Int    @id
         7 |   firstName String @db.Char(256)
           | 
    "#]];

    expect_error(schema, &expectation);
}

#[test]
fn should_fail_on_argument_out_of_range_for_varchar_type() {
    let schema = indoc! {r#"
        datasource db {
          provider = "kingbase-mysql"
        }

        model User {
          id        Int    @id
          firstName String @db.Char(655350)
        }
    "#};

    let expectation = expect![[r#"
        error: Argument M is out of range for native type `Char(655350)` of Kingbase MySQL: M can range from 0 to 255.
          -->  schema.prisma:7
           | 
         6 |   id        Int    @id
         7 |   firstName String @db.Char(655350)
           | 
    "#]];

    expect_error(schema, &expectation);
}

#[test]
fn should_fail_on_argument_out_of_range_for_decimal_type() {
    let schema = indoc! {r#"
        datasource db {
          provider = "kingbase-mysql"
        }

        model User {
          id        Int     @id
          firstName Decimal @db.Decimal(66,20)
        }
    "#};

    let expectation = expect![[r#"
        error: Argument M is out of range for native type `Decimal(66,20)` of Kingbase MySQL: Precision can range from 1 to 65.
          -->  schema.prisma:7
           | 
         6 |   id        Int     @id
         7 |   firstName Decimal @db.Decimal(66,20)
           | 
    "#]];

    expect_error(schema, &expectation);

    let schema = indoc! {r#"
        datasource db {
          provider = "kingbase-mysql"
        }

        model User {
          id        Int     @id
          firstName Decimal @db.Decimal(44,33)
        }
    "#};

    let expectation = expect![[r#"
        error: Argument M is out of range for native type `Decimal(44,33)` of Kingbase MySQL: Scale can range from 0 to 30.
          -->  schema.prisma:7
           | 
         6 |   id        Int     @id
         7 |   firstName Decimal @db.Decimal(44,33)
           | 
    "#]];

    expect_error(schema, &expectation);
}

#[test]
fn should_fail_on_native_type_decimal_when_scale_is_bigger_than_precision() {
    let dml = indoc! {r#"
        datasource db {
          provider = "kingbase-mysql"
        }

        model Blog {
            id     Int  @id
            dec Decimal @db.Decimal(2, 4)
        }
    "#};

    let expectation = expect![[r#"
        error: The scale must not be larger than the precision for the Decimal(2,4) native type in Kingbase MySQL.
          -->  schema.prisma:7
           | 
         6 |     id     Int  @id
         7 |     dec Decimal @db.Decimal(2, 4)
           | 
    "#]];

    expect_error(dml, &expectation);
}

#[test]
fn should_fail_on_incompatible_scalar_type_with_tiny_int() {
    let dml = r#"
        datasource db {
          provider = "kingbase-mysql"
        }

        model Blog {
          id     Int      @id
          bigInt DateTime @db.TinyInt
        }
    "#;

    let expectation = expect![[r#"
        error: Native type TinyInt is not compatible with declared field type DateTime, expected field type Boolean or Int.
          -->  schema.prisma:8
           | 
         7 |           id     Int      @id
         8 |           bigInt DateTime @db.TinyInt
           | 
    "#]];

    expect_error(dml, &expectation);
}

#[test]
fn kingbase_mysql_provider_accepts_mysql_compatible_native_types() {
    let schema = indoc! {r#"
        datasource db {
          provider = "kingbase-mysql"
        }

        model User {
          id        Int      @id @db.Int
          email     String   @unique @db.VarChar(191)
          active    Boolean  @db.TinyInt
          score     Decimal  @db.Decimal(10, 2)
          createdAt DateTime @db.DateTime(3)
          payload   Bytes    @db.LongBlob
          metadata  Json     @db.Json
        }
    "#};

    assert_valid(schema);
}

#[test]
fn kingbase_mysql_uses_independent_native_type_defaults() {
    let schema = indoc! {r#"
        datasource db {
          provider = "kingbase-mysql"
        }

        model User {
          id Int @id
        }
    "#};
    let validated_schema = parse_schema(schema);
    let native_type = KINGBASE_MYSQL
        .default_native_type_for_scalar_type(
            &psl::parser_database::ScalarFieldType::BuiltInScalar(ScalarType::String),
            &validated_schema,
        )
        .unwrap();

    assert_eq!(
        native_type,
        NativeTypeInstance::new::<KingbaseMySqlType>(KingbaseMySqlType::VarChar(191)),
    );
}
