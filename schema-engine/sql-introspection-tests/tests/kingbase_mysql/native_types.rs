use sql_introspection_tests::test_api::*;
use test_macros::test_connector;

#[test_connector(tags(KingbaseMysql))]
async fn native_type_columns_are_introspected(api: &mut TestApi) -> TestResult {
    api.raw_cmd(
        r#"
        CREATE TABLE native_types (
            id INT NOT NULL AUTO_INCREMENT PRIMARY KEY,
            small_int SMALLINT NOT NULL,
            tiny_bool TINYINT(1) NOT NULL,
            tiny_int TINYINT NOT NULL,
            medium_int MEDIUMINT NOT NULL,
            big_int BIGINT NOT NULL,
            decimal_value DECIMAL(5, 3) NOT NULL,
            numeric_value NUMERIC(4, 1) NOT NULL,
            float_value FLOAT NOT NULL,
            double_value DOUBLE NOT NULL,
            bits BIT(8) NOT NULL,
            char_value CHAR(10) NOT NULL,
            varchar_value VARCHAR(32) NOT NULL,
            binary_value BINARY(8) NOT NULL,
            varbinary_value VARBINARY(8) NOT NULL,
            tiny_blob TINYBLOB NOT NULL,
            blob_value BLOB NOT NULL,
            medium_blob MEDIUMBLOB NOT NULL,
            long_blob LONGBLOB NOT NULL,
            tiny_text TINYTEXT NOT NULL,
            text_value TEXT NOT NULL,
            medium_text MEDIUMTEXT NOT NULL,
            long_text LONGTEXT NOT NULL,
            date_value DATE NOT NULL,
            time_value TIME(3) NOT NULL,
            datetime_value DATETIME(3) NOT NULL,
            timestamp_value TIMESTAMP(3) NOT NULL,
            year_value YEAR NOT NULL,
            json_value JSON NOT NULL
        );
    "#,
    )
    .await;

    let dml = api.introspect_dml().await?;
    let normalized = dml
        .lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>()
        .join("\n");

    for expected in [
        "small_int Int @db.SmallInt",
        "tiny_bool Boolean",
        "tiny_int Int @db.TinyInt",
        "medium_int Int @db.MediumInt",
        "big_int BigInt",
        "decimal_value Decimal @db.Decimal(5, 3)",
        "numeric_value Decimal @db.Decimal(4, 1)",
        "float_value Float @db.Float",
        "double_value Float",
        "bits Bytes @db.Bit(8)",
        "char_value String @db.Char(10)",
        "varchar_value String @db.VarChar(32)",
        "binary_value Bytes @db.Binary(8)",
        "varbinary_value Bytes @db.VarBinary(8)",
        "tiny_blob Bytes @db.TinyBlob",
        "blob_value Bytes @db.Blob",
        "medium_blob Bytes @db.MediumBlob",
        "long_blob Bytes",
        "tiny_text String @db.TinyText",
        "text_value String @db.Text",
        "medium_text String @db.MediumText",
        "long_text String",
        "date_value DateTime @db.Date",
        "time_value DateTime @db.Time(3)",
        "datetime_value DateTime",
        "timestamp_value DateTime @db.Timestamp(3)",
        "year_value Int @db.Year",
        "json_value Json",
    ] {
        assert!(normalized.contains(expected), "missing expected type in:\\n{dml}");
    }

    Ok(())
}
