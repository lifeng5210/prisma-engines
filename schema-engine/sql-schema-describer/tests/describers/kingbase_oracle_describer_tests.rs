use crate::test_api::*;
use pretty_assertions::assert_eq;
use prisma_value::PrismaValue;
use psl::builtin_connectors::{KingbaseOracleNumberArguments, KingbaseOracleType};
use sql_schema_describer::{ColumnTypeFamily, ForeignKeyAction, SQLSortOrder};

#[test_connector(tags(KingbaseOracle))]
fn oracle_mode_column_types_and_identity_are_described(api: TestApi) {
    api.raw_cmd(
        r#"
        CREATE TABLE type_samples (
            id SERIAL PRIMARY KEY,
            tinyint_value TINYINT,
            int_value NUMBER(10, 0),
            bigint_value NUMBER(19, 0),
            decimal_value NUMBER(12, 4),
            binary_float_value BINARY_FLOAT,
            binary_double_value BINARY_DOUBLE,
            char_value CHAR(10),
            varchar_value VARCHAR2(32),
            clob_value CLOB,
            nclob_value NCLOB,
            blob_value BLOB,
            date_value DATE,
            timestamp_value TIMESTAMP(3),
            timestamp_tz_value TIMESTAMP(3) WITH TIME ZONE,
            bool_value BOOLEAN,
            json_value JSON,
            uuid_value UUID,
            xml_value XML
        );
        "#,
    );

    let schema = api.describe();
    let table = schema.table_walker("type_samples").unwrap();

    assert_eq!(1, table.primary_key_columns_count());
    assert!(table.column("id").unwrap().is_autoincrement());

    for (column_name, expected_family, expected_native_type) in [
        (
            "id",
            ColumnTypeFamily::Int,
            Some(KingbaseOracleType::Number(
                KingbaseOracleNumberArguments::PrecisionAndScale(10, 0),
            )),
        ),
        (
            "tinyint_value",
            ColumnTypeFamily::Int,
            Some(KingbaseOracleType::TinyInt),
        ),
        (
            "int_value",
            ColumnTypeFamily::Int,
            Some(KingbaseOracleType::Number(
                KingbaseOracleNumberArguments::PrecisionAndScale(10, 0),
            )),
        ),
        (
            "bigint_value",
            ColumnTypeFamily::BigInt,
            Some(KingbaseOracleType::Number(
                KingbaseOracleNumberArguments::PrecisionAndScale(19, 0),
            )),
        ),
        (
            "decimal_value",
            ColumnTypeFamily::Decimal,
            Some(KingbaseOracleType::Number(
                KingbaseOracleNumberArguments::PrecisionAndScale(12, 4),
            )),
        ),
        (
            "binary_float_value",
            ColumnTypeFamily::Float,
            Some(KingbaseOracleType::BinaryFloat),
        ),
        (
            "binary_double_value",
            ColumnTypeFamily::Float,
            Some(KingbaseOracleType::BinaryDouble),
        ),
        (
            "char_value",
            ColumnTypeFamily::String,
            Some(KingbaseOracleType::Char(Some(10))),
        ),
        (
            "varchar_value",
            ColumnTypeFamily::String,
            Some(KingbaseOracleType::VarChar2(Some(32))),
        ),
        ("clob_value", ColumnTypeFamily::String, Some(KingbaseOracleType::Clob)),
        // Kingbase Oracle's PostgreSQL-compatible catalog normalizes NCLOB to
        // its CLOB storage type, so introspection must emit the canonical type.
        ("nclob_value", ColumnTypeFamily::String, Some(KingbaseOracleType::Clob)),
        ("blob_value", ColumnTypeFamily::Binary, Some(KingbaseOracleType::Blob)),
        // Oracle DATE includes a time-of-day. The Kingbase catalog represents
        // it as timestamp(0), which is the lossless canonical native type.
        (
            "date_value",
            ColumnTypeFamily::DateTime,
            Some(KingbaseOracleType::Timestamp(Some(0))),
        ),
        (
            "timestamp_value",
            ColumnTypeFamily::DateTime,
            Some(KingbaseOracleType::Timestamp(Some(3))),
        ),
        (
            "timestamp_tz_value",
            ColumnTypeFamily::DateTime,
            Some(KingbaseOracleType::TimestampTz(Some(3))),
        ),
        (
            "bool_value",
            ColumnTypeFamily::Boolean,
            Some(KingbaseOracleType::Boolean),
        ),
        ("json_value", ColumnTypeFamily::Json, Some(KingbaseOracleType::Json)),
        ("uuid_value", ColumnTypeFamily::String, Some(KingbaseOracleType::Uuid)),
        ("xml_value", ColumnTypeFamily::String, Some(KingbaseOracleType::Xml)),
    ] {
        let column = table.column(column_name).unwrap();
        assert_eq!(
            &expected_family,
            column.column_type_family(),
            "wrong type family for {column_name}"
        );
        let native_type = column
            .column_type()
            .native_type
            .as_ref()
            .map(|native_type| *native_type.downcast_ref::<KingbaseOracleType>());
        assert_eq!(expected_native_type, native_type, "wrong native type for {column_name}");
    }
}

#[test_connector(tags(KingbaseOracle))]
fn oracle_mode_native_type_boundaries_are_described(api: TestApi) {
    api.raw_cmd(
        r#"
        CREATE TABLE type_boundaries (
            number_unspecified NUMBER,
            number_boolean NUMBER(1, 0),
            float_value FLOAT(53),
            national_char NCHAR(10),
            national_varchar NVARCHAR2(32),
            local_timestamp TIMESTAMP(3) WITH LOCAL TIME ZONE
        );
        "#,
    );

    let schema = api.describe();
    let table = schema.table_walker("type_boundaries").unwrap();

    for (column_name, expected_family, expected_native_type) in [
        (
            "number_unspecified",
            ColumnTypeFamily::Decimal,
            Some(KingbaseOracleType::Number(KingbaseOracleNumberArguments::Unspecified)),
        ),
        (
            "number_boolean",
            ColumnTypeFamily::Int,
            Some(KingbaseOracleType::Number(
                KingbaseOracleNumberArguments::PrecisionAndScale(1, 0),
            )),
        ),
        (
            "float_value",
            ColumnTypeFamily::Float,
            Some(KingbaseOracleType::BinaryDouble),
        ),
        (
            "national_char",
            ColumnTypeFamily::String,
            // Kingbase Oracle exposes NCHAR as the same `bpchar` catalog type
            // as CHAR, so introspection uses the canonical character type.
            Some(KingbaseOracleType::Char(Some(10))),
        ),
        (
            "national_varchar",
            ColumnTypeFamily::String,
            // The catalog also normalizes NVARCHAR2 to `varchar`.
            Some(KingbaseOracleType::VarChar2(Some(32))),
        ),
        (
            "local_timestamp",
            ColumnTypeFamily::DateTime,
            // TIMESTAMP WITH LOCAL TIME ZONE is reported as PostgreSQL
            // `timestamp` by this Kingbase Oracle-mode catalog.
            Some(KingbaseOracleType::Timestamp(Some(3))),
        ),
    ] {
        let column = table.column(column_name).unwrap();
        assert_eq!(
            &expected_family,
            column.column_type_family(),
            "wrong type family for {column_name}"
        );
        let native_type = column
            .column_type()
            .native_type
            .as_ref()
            .map(|native_type| *native_type.downcast_ref::<KingbaseOracleType>());
        assert_eq!(expected_native_type, native_type, "wrong native type for {column_name}");
    }
}

#[test_connector(tags(KingbaseOracle))]
fn oracle_mode_postgres_array_columns_remain_unsupported(api: TestApi) {
    api.raw_cmd(
        r#"
        CREATE TABLE unsupported_array_column (
            values INTEGER[]
        );
        "#,
    );

    let schema = api.describe();
    let column = schema
        .table_walker("unsupported_array_column")
        .unwrap()
        .column("values")
        .unwrap();

    assert!(matches!(column.column_type_family(), ColumnTypeFamily::Unsupported(_)));
    assert!(column.column_type().native_type.is_none());
}

#[test_connector(tags(KingbaseOracle))]
fn oracle_mode_defaults_enums_and_foreign_keys_are_described(api: TestApi) {
    api.raw_cmd(
        r#"
        CREATE TYPE order_status AS ENUM ('draft', 'published');

        CREATE TABLE parent (
            id NUMBER(10, 0) PRIMARY KEY
        );

        CREATE TABLE child (
            id SERIAL PRIMARY KEY,
            parent_id NUMBER(10, 0) NOT NULL,
            retries NUMBER(10, 0) NOT NULL DEFAULT 3,
            label VARCHAR2(32) NOT NULL DEFAULT 'pending',
            created_at TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
            status order_status NOT NULL,
            CONSTRAINT child_parent_fk
                FOREIGN KEY (parent_id)
                REFERENCES parent (id)
                ON DELETE CASCADE
        );
        "#,
    );

    let schema = api.describe();
    let child = schema.table_walker("child").unwrap();

    assert!(child.column("id").unwrap().is_autoincrement());
    assert_eq!(
        Some(&PrismaValue::Int(3)),
        child.column("retries").unwrap().default().unwrap().as_value()
    );
    assert_eq!(
        Some(&PrismaValue::String("pending".to_owned())),
        child.column("label").unwrap().default().unwrap().as_value()
    );
    assert!(child.column("created_at").unwrap().default().unwrap().is_now());

    let status = child.column("status").unwrap().column_type_family_as_enum().unwrap();
    assert_eq!("order_status", status.name());
    assert_eq!(vec!["draft", "published"], status.values().collect::<Vec<_>>());

    schema.assert_table("child", |table| {
        table.assert_foreign_key_on_columns(&["parent_id"], |foreign_key| {
            foreign_key
                .assert_references("parent", &["id"])
                .assert_on_delete(ForeignKeyAction::Cascade)
        })
    });
}

#[test_connector(tags(KingbaseOracle))]
fn oracle_mode_composite_foreign_keys_preserve_column_order(api: TestApi) {
    api.raw_cmd(
        r#"
        CREATE TABLE city (
            id NUMBER(10, 0) NOT NULL,
            code VARCHAR2(16) NOT NULL,
            PRIMARY KEY (id, code)
        );

        CREATE TABLE user_city (
            city_id NUMBER(10, 0) NOT NULL,
            city_code VARCHAR2(16) NOT NULL,
            CONSTRAINT user_city_city_fk
                FOREIGN KEY (city_id, city_code)
                REFERENCES city (id, code)
                ON DELETE CASCADE
        );
        "#,
    );

    api.describe().assert_table("user_city", |table| {
        table.assert_foreign_key_on_columns(&["city_id", "city_code"], |foreign_key| {
            foreign_key
                .assert_references("city", &["id", "code"])
                .assert_on_delete(ForeignKeyAction::Cascade)
        })
    });
}

#[test_connector(tags(KingbaseOracle))]
fn oracle_mode_check_constraints_are_described(api: TestApi) {
    api.raw_cmd(
        r#"
        CREATE TABLE products (
            id NUMBER(10, 0) NOT NULL PRIMARY KEY,
            price NUMBER(10, 0) NOT NULL,
            CONSTRAINT products_price_positive CHECK (price > 0)
        );
        "#,
    );

    let schema = api.describe();
    let table = schema.table_walker("products").unwrap();

    assert!(table.has_check_constraints());
    assert_eq!(
        vec!["products_price_positive"],
        table.check_constraints().collect::<Vec<_>>()
    );
}

#[test_connector(tags(KingbaseOracle))]
fn oracle_mode_table_and_column_comments_are_described(api: TestApi) {
    api.raw_cmd(
        r#"
        CREATE TABLE comments (
            id NUMBER(10, 0) NOT NULL PRIMARY KEY
        );
        COMMENT ON TABLE comments IS 'commented table';
        COMMENT ON COLUMN comments.id IS 'identifier';
        "#,
    );

    let schema = api.describe();
    let table = schema.table_walker("comments").unwrap();

    assert_eq!(Some("commented table"), table.description());
    assert_eq!(Some("identifier"), table.column("id").unwrap().description());
}

#[test_connector(tags(KingbaseOracle))]
fn oracle_mode_views_are_described(api: TestApi) {
    api.raw_cmd(
        r#"
        CREATE TABLE a (a_id NUMBER(10, 0));
        CREATE TABLE b (b_id NUMBER(10, 0));
        CREATE VIEW ab AS SELECT a_id FROM a UNION ALL SELECT b_id FROM b;
        "#,
    );

    let schema = api.describe();
    let view = schema.get_view("ab").expect("couldn't get ab view");

    assert_eq!("ab", &view.name);
    assert!(
        view.definition
            .as_deref()
            .is_some_and(|definition| definition.contains("UNION ALL"))
    );
}

#[test_connector(tags(KingbaseOracle))]
fn oracle_mode_procedures_are_described(api: TestApi) {
    api.raw_cmd(
        r#"
        CREATE PROCEDURE describe_me (OUT result NUMBER)
        AS
        BEGIN
            SELECT 1 INTO result;
        END;
        "#,
    );

    let schema = api.describe();
    let procedure = schema
        .get_procedure("describe_me")
        .expect("couldn't get describe_me procedure");

    assert_eq!("describe_me", &procedure.name);
    assert!(procedure.definition.is_some());
}

#[test_connector(tags(KingbaseOracle))]
fn oracle_mode_compound_unique_indexes_are_described(api: TestApi) {
    api.raw_cmd(
        r#"
        CREATE TABLE relation_index (
            left_id NUMBER(10, 0) NOT NULL,
            right_id NUMBER(10, 0) NOT NULL
        );
        CREATE UNIQUE INDEX relation_index_unique ON relation_index (left_id, right_id);
        "#,
    );

    api.describe().assert_table("relation_index", |table| {
        table.assert_index_on_columns(&["left_id", "right_id"], |index| {
            index.assert_name("relation_index_unique").assert_is_unique()
        })
    });
}

#[test_connector(tags(KingbaseOracle))]
fn oracle_mode_function_defaults_are_described_as_dbgenerated(api: TestApi) {
    api.raw_cmd(
        r#"
        CREATE TABLE expression_defaults (
            value NUMBER(10, 0) DEFAULT (ABS(8) + ABS(8))
        );
        "#,
    );

    let schema = api.describe();
    let table = schema.table_walker("expression_defaults").unwrap();

    assert!(table.column("value").unwrap().default().unwrap().is_db_generated());
}

#[test_connector(tags(KingbaseOracle))]
fn oracle_mode_escaped_string_defaults_are_described(api: TestApi) {
    api.raw_cmd(
        r#"
        CREATE TABLE escaped_defaults (
            quote_value VARCHAR2(500) NOT NULL DEFAULT '"That''s a lot of fish!"\n - Godzilla, 1998',
            backslash_value VARCHAR2(255) NOT NULL DEFAULT 'xyz\Datasource\Model'
        );
        "#,
    );

    let schema = api.describe();
    let table = schema.table_walker("escaped_defaults").unwrap();

    assert_eq!(
        Some(&PrismaValue::String(
            "\"That's a lot of fish!\"\\n - Godzilla, 1998".to_owned()
        )),
        table.column("quote_value").unwrap().default().unwrap().as_value()
    );
    assert_eq!(
        Some(&PrismaValue::String("xyz\\Datasource\\Model".to_owned())),
        table.column("backslash_value").unwrap().default().unwrap().as_value()
    );
}

#[test_connector(tags(KingbaseOracle))]
fn oracle_mode_constraints_outside_the_described_schema_are_ignored(api: TestApi) {
    api.raw_cmd(
        r#"
        CREATE SCHEMA other_schema;

        CREATE TABLE other_schema.constraint_parent (
            id NUMBER(10, 0) PRIMARY KEY
        );
        CREATE TABLE other_schema.constraint_child (
            id NUMBER(10, 0) PRIMARY KEY,
            parent_id NUMBER(10, 0),
            CONSTRAINT other_schema_parent_fk
                FOREIGN KEY (parent_id)
                REFERENCES other_schema.constraint_parent (id)
        );

        CREATE TABLE constraint_parent (
            id NUMBER(10, 0) PRIMARY KEY
        );
        CREATE TABLE constraint_child (
            id NUMBER(10, 0) PRIMARY KEY,
            parent_id NUMBER(10, 0),
            CONSTRAINT current_schema_parent_fk
                FOREIGN KEY (parent_id)
                REFERENCES constraint_parent (id)
        );
        "#,
    );

    let schema = api.describe();
    assert_eq!(1, schema.walk_foreign_keys().count());
    schema.assert_table("constraint_child", |table| {
        table.assert_foreign_key_on_columns(&["parent_id"], |foreign_key| {
            foreign_key.assert_references("constraint_parent", &["id"])
        })
    });
}

#[test_connector(tags(KingbaseOracle))]
fn oracle_mode_multiple_schemas_keep_same_named_tables_and_relations(api: TestApi) {
    api.raw_cmd(
        r#"
        CREATE SCHEMA one;
        CREATE SCHEMA two;

        CREATE TABLE one.parent (
            id NUMBER(10, 0) PRIMARY KEY
        );
        CREATE TABLE one.child (
            parent_id NUMBER(10, 0),
            CONSTRAINT one_child_parent_fk
                FOREIGN KEY (parent_id) REFERENCES one.parent (id)
        );

        CREATE TABLE two.parent (
            id NUMBER(10, 0) PRIMARY KEY
        );
        CREATE TABLE two.child (
            parent_id NUMBER(10, 0),
            CONSTRAINT two_child_parent_fk
                FOREIGN KEY (parent_id) REFERENCES two.parent (id)
        );
        "#,
    );

    let schema = api.describe_with_schemas(&["one", "two"]);
    schema.assert_namespace("one").assert_namespace("two");
    assert_eq!(4, schema.table_walkers().len());
    assert_eq!(2, schema.walk_foreign_keys().count());

    for namespace in ["one", "two"] {
        let child = schema
            .table_walkers()
            .find(|table| table.name() == "child" && table.namespace() == Some(namespace))
            .unwrap();
        let foreign_key = child.foreign_keys().next().unwrap();

        assert_eq!("parent", foreign_key.referenced_table_name());
        assert_eq!(Some(namespace), foreign_key.referenced_table().namespace());
    }
}

#[test_connector(tags(KingbaseOracle))]
fn oracle_mode_foreign_key_delete_actions_are_described(api: TestApi) {
    api.raw_cmd(
        r#"
        CREATE TABLE city (
            id NUMBER(10, 0) PRIMARY KEY
        );
        CREATE TABLE user_city (
            id NUMBER(10, 0) PRIMARY KEY,
            city_no_action NUMBER(10, 0),
            city_cascade NUMBER(10, 0),
            city_restrict NUMBER(10, 0),
            city_set_null NUMBER(10, 0),
            city_set_default NUMBER(10, 0) DEFAULT 1,
            FOREIGN KEY (city_no_action) REFERENCES city (id) ON DELETE NO ACTION,
            FOREIGN KEY (city_cascade) REFERENCES city (id) ON DELETE CASCADE,
            FOREIGN KEY (city_restrict) REFERENCES city (id) ON DELETE RESTRICT,
            FOREIGN KEY (city_set_null) REFERENCES city (id) ON DELETE SET NULL,
            FOREIGN KEY (city_set_default) REFERENCES city (id) ON DELETE SET DEFAULT
        );
        "#,
    );

    api.describe().assert_table("user_city", |table| {
        table
            .assert_foreign_key_on_columns(&["city_no_action"], |foreign_key| {
                foreign_key
                    .assert_references("city", &["id"])
                    .assert_on_delete(ForeignKeyAction::NoAction)
            })
            .assert_foreign_key_on_columns(&["city_cascade"], |foreign_key| {
                foreign_key
                    .assert_references("city", &["id"])
                    .assert_on_delete(ForeignKeyAction::Cascade)
            })
            .assert_foreign_key_on_columns(&["city_restrict"], |foreign_key| {
                foreign_key
                    .assert_references("city", &["id"])
                    .assert_on_delete(ForeignKeyAction::Restrict)
            })
            .assert_foreign_key_on_columns(&["city_set_null"], |foreign_key| {
                foreign_key
                    .assert_references("city", &["id"])
                    .assert_on_delete(ForeignKeyAction::SetNull)
            })
            .assert_foreign_key_on_columns(&["city_set_default"], |foreign_key| {
                foreign_key
                    .assert_references("city", &["id"])
                    .assert_on_delete(ForeignKeyAction::SetDefault)
            })
    });
}

#[test_connector(tags(KingbaseOracle))]
fn oracle_mode_composite_index_sort_order_is_described(api: TestApi) {
    api.raw_cmd(
        r#"
        CREATE TABLE sorted_values (
            first_value NUMBER(10, 0) NOT NULL,
            second_value NUMBER(10, 0) NOT NULL
        );

        CREATE INDEX sorted_values_index ON sorted_values (first_value ASC, second_value DESC);
        "#,
    );

    let schema = api.describe();
    let table = schema.table_walker("sorted_values").unwrap();
    let index = table
        .indexes()
        .find(|index| index.name() == "sorted_values_index")
        .unwrap();
    let columns = index.columns().collect::<Vec<_>>();

    assert_eq!(2, columns.len());
    assert_eq!("first_value", columns[0].as_column().name());
    assert_eq!("second_value", columns[1].as_column().name());
    assert_eq!(Some(SQLSortOrder::Asc), columns[0].sort_order());
    assert_eq!(Some(SQLSortOrder::Desc), columns[1].sort_order());
}
