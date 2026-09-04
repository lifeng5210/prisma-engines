use sql_migration_tests::test_api::*;
use sql_schema_describer::ColumnTypeFamily;

// Kingbase does not implicitly add a separate index for a foreign-key column. This
// still verifies the invariant that an explicitly declared covering index is emitted
// exactly once and remains stable after a round-trip through introspection.
#[test_connector(tags(KingbaseMysql))]
fn indexes_on_foreign_key_fields_are_not_created_twice(api: TestApi) {
    let schema = r#"
        model Human {
            id      String @id
            catname String
            cat_rel Cat    @relation(fields: [catname], references: [name])

            @@index([catname])
        }

        model Cat {
            id     String @id
            name   String @unique
            humans Human[]
        }
    "#;

    api.schema_push_w_datasource(schema).send().assert_green();
    api.assert_schema().assert_table("Human", |table| {
        table
            .assert_foreign_keys_count(1)
            .assert_fk_on_columns(&["catname"], |fk| fk.assert_references("Cat", &["name"]))
            .assert_indexes_count(1)
            .assert_index_on_columns(&["catname"], |idx| idx.assert_is_not_unique())
    });

    api.schema_push_w_datasource(schema)
        .send()
        .assert_green()
        .assert_no_steps();
}

#[test_connector(tags(KingbaseMysql))]
fn schema_push_creates_indexes_foreign_keys_and_is_idempotent(api: TestApi) {
    let schema = r#"
        model City {
            id    Int    @id
            code  String @unique @db.VarChar(32)
            users User[]
        }

        model User {
            id     Int    @id
            cityId Int
            email  String @unique @db.VarChar(64)
            city   City   @relation(fields: [cityId], references: [id], onDelete: Cascade)

            @@index([cityId], map: "User_cityId_idx")
        }
    "#;

    api.schema_push_w_datasource(schema)
        .send()
        .assert_green()
        .assert_has_executed_steps();

    api.assert_schema()
        .assert_table("City", |table| {
            table
                .assert_pk(|pk| pk.assert_columns(&["id"]))
                .assert_index_on_columns(&["code"], |index| index.assert_is_unique())
        })
        .assert_table("User", |table| {
            table
                .assert_pk(|pk| pk.assert_columns(&["id"]))
                .assert_foreign_keys_count(1)
                .assert_fk_on_columns(&["cityId"], |fk| fk.assert_references("City", &["id"]))
                .assert_index_on_columns(&["cityId"], |index| {
                    index.assert_is_not_unique().assert_name("User_cityId_idx")
                })
                .assert_index_on_columns(&["email"], |index| index.assert_is_unique())
        });

    api.schema_push_w_datasource(schema)
        .send()
        .assert_green()
        .assert_no_steps();
}

#[test_connector(tags(KingbaseMysql))]
fn fulltext_indexes_use_gin_and_are_idempotent(api: TestApi) {
    let schema = r#"
        model Article {
            id      Int    @id
            title   String @db.Text
            content String @db.Text

            @@fulltext([title, content])
        }
    "#;

    api.schema_push_w_datasource(schema)
        .send()
        .assert_green()
        .assert_has_executed_steps();

    api.assert_schema().assert_table("Article", |table| {
        table.assert_index_on_columns(&["title", "content"], |index| {
            index.assert_is_fulltext().assert_name("Article_title_content_idx")
        })
    });

    api.schema_push_w_datasource(schema)
        .send()
        .assert_green()
        .assert_no_steps();
}

#[test_connector(tags(KingbaseMysql))]
fn enum_columns_can_be_created_altered_and_reintrospected(api: TestApi) {
    let initial_schema = r#"
        model Post {
            id     Int    @id
            status Status @default(DRAFT)
        }

        enum Status {
            DRAFT
            PUBLISHED
        }
    "#;

    api.schema_push_w_datasource(initial_schema)
        .send()
        .assert_green()
        .assert_has_executed_steps();
    api.assert_schema().assert_table("Post", |table| {
        table.assert_column("status", |column| column.assert_type_is_enum())
    });

    let changed_schema = r#"
        model Post {
            id     Int     @id
            status Status?
        }

        enum Status {
            DRAFT
            PUBLISHED
        }
    "#;

    api.schema_push_w_datasource(changed_schema)
        .send()
        .assert_green()
        .assert_has_executed_steps();
    api.assert_schema().assert_table("Post", |table| {
        table.assert_column("status", |column| column.assert_is_nullable().assert_type_is_enum())
    });

    api.schema_push_w_datasource(changed_schema)
        .send()
        .assert_green()
        .assert_no_steps();
}

#[test_connector(tags(KingbaseMysql))]
fn migrations_can_be_created_applied_and_reapplied(api: TestApi) {
    let migrations_directory = api.create_migrations_directory();
    let schema = api.datamodel_with_provider(
        r#"
            model Account {
                id    Int    @id
                email String @unique @db.VarChar(64)
            }
        "#,
    );

    api.create_migration("init", &schema, &migrations_directory).send_sync();

    api.apply_migrations(&migrations_directory)
        .send_sync()
        .assert_applied_migrations(&["init"]);
    api.assert_schema().assert_table("Account", |table| {
        table
            .assert_pk(|pk| pk.assert_columns(&["id"]))
            .assert_index_on_columns(&["email"], |index| index.assert_is_unique())
    });

    api.apply_migrations(&migrations_directory)
        .send_sync()
        .assert_applied_migrations(&[]);
    assert!(
        api.diagnose_migration_history(&migrations_directory)
            .send_sync()
            .into_output()
            .is_empty()
    );
}

#[test_connector(tags(KingbaseMysql))]
fn enum_creation_is_idempotent(api: TestApi) {
    let schema = r#"
        model Cat {
            id   String @id
            mood Mood
        }

        model Human {
            id   String @id
            mood Mood
        }

        enum Mood {
            HAPPY
            HUNGRY
        }
    "#;

    api.schema_push_w_datasource(schema)
        .send()
        .assert_green()
        .assert_has_executed_steps();
    api.schema_push_w_datasource(schema)
        .send()
        .assert_green()
        .assert_no_steps();
}

#[test_connector(tags(KingbaseMysql))]
fn enums_work_when_table_name_is_remapped(api: TestApi) {
    let schema = r#"
        model User {
            id     String     @id
            status UserStatus @map("currentStatus___")

            @@map("users")
        }

        enum UserStatus {
            CONFIRMED
            CANCELED
            BLOCKED
        }
    "#;

    api.schema_push_w_datasource(schema)
        .send()
        .assert_green()
        .assert_has_executed_steps();
    api.schema_push_w_datasource(schema)
        .send()
        .assert_green()
        .assert_no_steps();
}

#[test_connector(tags(KingbaseMysql))]
fn enum_column_arity_changes_are_migrated(api: TestApi) {
    let initial_schema = r#"
        enum Color {
            RED
            GREEN
            BLUE
        }

        model A {
            id           Int   @id
            primaryColor Color
            secondaryColor Color?
        }
    "#;

    api.schema_push_w_datasource(initial_schema)
        .send()
        .assert_green()
        .assert_has_executed_steps();

    let changed_schema = r#"
        enum Color {
            RED
            GREEN
            BLUE
        }

        model A {
            id           Int   @id
            primaryColor Color?
            secondaryColor Color
        }
    "#;

    api.schema_push_w_datasource(changed_schema)
        .send()
        .assert_green()
        .assert_has_executed_steps();
    api.assert_schema().assert_table("A", |table| {
        table
            .assert_column("primaryColor", |column| column.assert_is_nullable())
            .assert_column("secondaryColor", |column| column.assert_is_required())
    });
}

#[test_connector(tags(KingbaseMysql))]
fn enum_alteration_preserves_column_arity(api: TestApi) {
    let initial_schema = r#"
        enum Color {
            RED
            GREEN
            BLUE
        }

        model A {
            id            Int   @id
            primaryColor  Color
            secondaryColor Color?
        }
    "#;

    api.schema_push_w_datasource(initial_schema)
        .send()
        .assert_green()
        .assert_has_executed_steps();

    let changed_schema = r#"
        enum Color {
            ROT
            GRUEN
            BLAU
        }

        model A {
            id            Int   @id
            primaryColor  Color
            secondaryColor Color?
        }
    "#;

    api.schema_push_w_datasource(changed_schema)
        .force(true)
        .send()
        .assert_executable()
        .assert_has_executed_steps();
    api.assert_schema().assert_table("A", |table| {
        table
            .assert_column("primaryColor", |column| column.assert_is_required())
            .assert_column("secondaryColor", |column| column.assert_is_nullable())
    });
}

#[test_connector(tags(KingbaseMysql))]
fn datetime_defaults_follow_column_precision(api: TestApi) {
    let schema = r#"
        model Event {
            id        Int      @id
            createdAt DateTime @default(now()) @db.DateTime(3)
        }
    "#;

    api.schema_push_w_datasource(schema)
        .send()
        .assert_green()
        .assert_has_executed_steps();
    api.assert_schema().assert_table("Event", |table| {
        table.assert_column("createdAt", |column| column.assert_is_required())
    });
    api.schema_push_w_datasource(schema)
        .send()
        .assert_green()
        .assert_no_steps();
}

#[test_connector(tags(KingbaseMysql))]
fn datetime_defaults_follow_unspecified_column_precision(api: TestApi) {
    let schema = r#"
        model Event {
            id        Int      @id
            createdAt DateTime @default(now()) @db.DateTime()
        }
    "#;

    api.schema_push_w_datasource(schema)
        .send()
        .assert_green()
        .assert_has_executed_steps();
    api.schema_push_w_datasource(schema)
        .send()
        .assert_green()
        .assert_no_steps();
}

#[test_connector(tags(KingbaseMysql))]
fn datetime_dbgenerated_defaults_are_migrated(api: TestApi) {
    let schema = r#"
        model Event {
            id Int @id
            day DateTime @default(dbgenerated("'2020-01-01'")) @db.Date
            at  DateTime @default(dbgenerated("'16:20:00'")) @db.Time(0)
        }
    "#;

    api.schema_push_w_datasource(schema)
        .send()
        .assert_green()
        .assert_has_executed_steps();
    api.schema_push_w_datasource(schema)
        .send()
        .assert_green()
        .assert_no_steps();
}

#[test_connector(tags(KingbaseMysql))]
fn bigint_defaults_are_migrated(api: TestApi) {
    let schema = r#"
        model Value {
            id  String @id
            bar BigInt @default(0)
        }
    "#;

    api.schema_push_w_datasource(schema)
        .send()
        .assert_green()
        .assert_has_executed_steps();
    api.assert_schema().assert_table("Value", |table| {
        table.assert_column("bar", |column| column.assert_type_family(ColumnTypeFamily::BigInt))
    });
    api.schema_push_w_datasource(schema)
        .send()
        .assert_green()
        .assert_no_steps();
}

#[test_connector(tags(KingbaseMysql))]
fn constraint_names_can_be_changed(api: TestApi) {
    let initial_schema = r#"
        model Parent {
            id       Int    @id
            code     String @unique
            children Child[]
        }

        model Child {
            id       Int    @id
            parentId Int
            parent   Parent @relation(fields: [parentId], references: [id])

            @@index([parentId])
        }
    "#;

    api.schema_push_w_datasource(initial_schema)
        .send()
        .assert_green()
        .assert_has_executed_steps();

    let changed_schema = r#"
        model Parent {
            id       Int    @id
            code     String @unique(map: "Parent_code_unique")
            children Child[]
        }

        model Child {
            id       Int    @id
            parentId Int
            parent   Parent @relation(map: "Child_parent_fk", fields: [parentId], references: [id])

            @@index([parentId], map: "Child_parent_idx")
        }
    "#;

    api.schema_push_w_datasource(changed_schema)
        .send()
        .assert_green()
        .assert_has_executed_steps();
    api.schema_push_w_datasource(changed_schema)
        .send()
        .assert_green()
        .assert_no_steps();
}

#[test_connector(tags(KingbaseMysql))]
fn foreign_keys_survive_removal_of_a_covering_unique_constraint(api: TestApi) {
    let initial_schema = r#"
        model User {
            id           Int           @id
            transactions Transaction[]
        }

        model Account {
            userId       Int
            id           Int
            transactions Transaction[]

            @@id([userId, id])
        }

        model Transaction {
            id        Int @id
            userId    Int
            accountId Int

            user    User    @relation(fields: [userId], references: [id])
            account Account @relation(fields: [userId, accountId], references: [userId, id])

            @@unique([userId, accountId])
        }
    "#;

    api.schema_push_w_datasource(initial_schema)
        .send()
        .assert_green()
        .assert_has_executed_steps();

    let changed_schema = r#"
        model User {
            id           Int           @id
            transactions Transaction[]
        }

        model Account {
            userId       Int
            id           Int
            transactions Transaction[]

            @@id([userId, id])
        }

        model Transaction {
            id        Int @id
            userId    Int
            accountId Int

            user    User    @relation(fields: [userId], references: [id])
            account Account @relation(fields: [userId, accountId], references: [userId, id])
        }
    "#;

    api.schema_push_w_datasource(changed_schema)
        .send()
        .assert_green()
        .assert_has_executed_steps();

    api.assert_schema().assert_table("Transaction", |table| {
        table.assert_foreign_keys_count(2).assert_indexes_count(0)
    });
}

#[test_connector(tags(KingbaseMysql))]
fn foreign_keys_and_their_covering_index_can_be_deleted_together(api: TestApi) {
    let initial_schema = r#"
        model User {
            id           Int           @id
            transactions Transaction[]
        }

        model Account {
            userId       Int
            id           Int
            transactions Transaction[]

            @@id([userId, id])
        }

        model Transaction {
            id        Int @id
            userId    Int
            accountId Int

            user    User    @relation(fields: [userId], references: [id])
            account Account @relation(fields: [userId, accountId], references: [userId, id])

            @@unique([userId, accountId])
        }
    "#;

    api.schema_push_w_datasource(initial_schema)
        .send()
        .assert_green()
        .assert_has_executed_steps();

    let changed_schema = r#"
        model User {
            id Int @id
        }

        model Account {
            userId Int
            id     Int

            @@id([userId, id])
        }

        model Transaction {
            id        Int @id
            userId    Int
            accountId Int
        }
    "#;

    api.schema_push_w_datasource(changed_schema)
        .send()
        .assert_green()
        .assert_has_executed_steps();
    api.assert_schema().assert_table("Transaction", |table| {
        table.assert_foreign_keys_count(0).assert_indexes_count(0)
    });
}
