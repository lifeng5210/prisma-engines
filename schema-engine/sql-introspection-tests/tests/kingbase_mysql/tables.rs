use indoc::indoc;
use sql_introspection_tests::test_api::*;
use test_macros::test_connector;

#[test_connector(tags(KingbaseMysql))]
async fn non_id_autoincrement_is_introspected(api: &mut TestApi) -> TestResult {
    api.raw_cmd(
        r#"
        CREATE TABLE `Test` (
            `id` INTEGER PRIMARY KEY,
            `authorId` INTEGER AUTO_INCREMENT UNIQUE
        );
    "#,
    )
    .await;

    let expected = expect![[r#"
        model Test {
          id       Int @id
          authorId Int @unique @default(autoincrement())
        }
    "#]];

    expected.assert_eq(&api.introspect_dml().await?);
    Ok(())
}

#[test_connector(tags(KingbaseMysql))]
async fn descending_indexes_are_introspected(api: &mut TestApi) -> TestResult {
    api.raw_cmd(
        r#"
        CREATE TABLE A (
            id INT PRIMARY KEY,
            a VARCHAR(32) NOT NULL,
            b VARCHAR(32) NOT NULL
        );
        CREATE INDEX A_a_b_idx ON A (a ASC, b DESC);
        CREATE UNIQUE INDEX A_a_b_key ON A (a ASC, b DESC);
    "#,
    )
    .await;

    let dml = api.introspect_dml().await?;
    assert!(dml.contains("@@index([a, b])"), "{dml}");
    assert!(dml.contains("@@unique([a, b])"), "{dml}");

    Ok(())
}

#[test_connector(tags(KingbaseMysql))]
async fn date_time_defaults_are_introspected(api: &mut TestApi) -> TestResult {
    api.raw_cmd(
        r#"
        CREATE TABLE defaults_table (
            id INT NOT NULL AUTO_INCREMENT PRIMARY KEY,
            created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
            updated_at TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3)
        );
    "#,
    )
    .await;

    let dml = api.introspect_dml().await?;
    assert!(dml.contains("created_at DateTime @default(now())"), "{dml}");
    assert!(dml.contains("updated_at DateTime @default(now())"), "{dml}");

    Ok(())
}

#[test_connector(tags(KingbaseMysql))]
async fn table_columns_defaults_and_indexes_are_introspected(api: &mut TestApi) -> TestResult {
    api.raw_cmd(indoc! {r#"
        CREATE TABLE table_features (
            id INT NOT NULL AUTO_INCREMENT PRIMARY KEY,
            tenant_id INT NOT NULL,
            required_label VARCHAR(64) NOT NULL,
            optional_label VARCHAR(64),
            attempts INT NOT NULL DEFAULT 3,
            created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
            unique_code VARCHAR(32) NOT NULL,
            binary_data VARBINARY(16),
            CONSTRAINT table_features_tenant_code_key UNIQUE (tenant_id, unique_code)
        );

        CREATE INDEX table_features_required_label_idx ON table_features (required_label);
    "#})
        .await;

    let expected = expect![[r#"
        model table_features {
          id             Int      @id @default(autoincrement())
          tenant_id      Int
          required_label String   @db.VarChar(64)
          optional_label String?  @db.VarChar(64)
          attempts       Int      @default(3)
          created_at     DateTime @default(now()) @db.DateTime(0)
          unique_code    String   @db.VarChar(32)
          binary_data    Bytes?   @db.VarBinary(16)

          @@unique([tenant_id, unique_code], map: "table_features_tenant_code_key")
          @@index([required_label])
        }
    "#]];
    expected.assert_eq(&api.introspect_dml().await?);

    Ok(())
}

#[test_connector(tags(KingbaseMysql))]
async fn quoted_string_defaults_are_introspected(api: &mut TestApi) -> TestResult {
    api.raw_cmd(indoc! {r#"
        CREATE TABLE string_defaults (
            id INT NOT NULL AUTO_INCREMENT PRIMARY KEY,
            quote_text VARCHAR(64) NOT NULL DEFAULT 'O''Reilly',
            empty_text VARCHAR(64) NOT NULL DEFAULT '',
            current_label VARCHAR(64) NOT NULL DEFAULT 'pending'
        );
    "#})
        .await;

    let expected = expect![[r#"
        model string_defaults {
          id            Int    @id @default(autoincrement())
          quote_text    String @default("O'Reilly") @db.VarChar(64)
          empty_text    String @default("") @db.VarChar(64)
          current_label String @default("pending") @db.VarChar(64)
        }
    "#]];
    expected.assert_eq(&api.introspect_dml().await?);

    Ok(())
}
