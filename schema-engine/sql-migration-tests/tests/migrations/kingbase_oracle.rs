use schema_core::schema_connector::Namespaces;
use sql_migration_tests::test_api::*;
use sql_schema_describer::{ColumnTypeFamily, DefaultValue, ForeignKeyAction};

#[test_connector(tags(KingbaseOracle), namespaces("one", "two"))]
fn schema_push_tracks_all_configured_namespaces(mut api: TestApi) {
    let datasource = api.datasource_block_with(&[("schemas", r#"["one", "two"]"#)]);
    let generator = api.generator_block();
    let schema = format!(
        r#"
            {datasource}

            {generator}

            model First {{
                id Int @id

                @@schema("one")
            }}

            model Second {{
                id Int @id

                @@schema("two")
            }}
        "#
    );

    api.schema_push(schema.clone())
        .send()
        .assert_green()
        .assert_has_executed_steps();

    let namespaces = Namespaces::from_vec(&mut vec!["one".to_owned(), "two".to_owned()]);
    api.assert_schema_with_namespaces(namespaces)
        .assert_has_table_with_ns("one", "First")
        .assert_has_table_with_ns("two", "Second");

    api.schema_push(schema).send().assert_green().assert_no_steps();
}

#[test_connector(tags(KingbaseOracle))]
fn changing_an_enum_preserves_replaced_column_defaults(api: TestApi) {
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

    api.schema_push_w_datasource(initial_schema).send().assert_green();

    let changed_schema = r#"
        model Post {
            id     Int    @id
            status Status @default(ARCHIVED)
        }

        enum Status {
            ARCHIVED
            PUBLISHED
        }
    "#;

    api.schema_push_w_datasource(changed_schema)
        .force(true)
        .send()
        .assert_executable()
        .assert_warnings(&[
            "The values [DRAFT] on the enum `Status` will be removed. If these variants are still used in the database, this will fail."
                .into(),
        ])
        .assert_has_executed_steps();

    api.assert_schema().assert_table("Post", |table| {
        table.assert_column("status", |column| column.assert_enum_default("ARCHIVED"))
    });

    api.schema_push_w_datasource(changed_schema)
        .send()
        .assert_green()
        .assert_no_steps();
}

#[test_connector(tags(KingbaseOracle))]
fn schema_push_creates_oracle_native_types_and_is_idempotent(api: TestApi) {
    let schema = r#"
        model Account {
            id        Int      @id
            code      String   @unique @db.VarChar2(64)
            amount    Decimal  @db.Number(12, 4)
            createdAt DateTime @db.Timestamp(3) @default(now())
            payload   Json
            data      Bytes    @db.Blob
        }
    "#;

    api.schema_push_w_datasource(schema)
        .send()
        .assert_green()
        .assert_has_executed_steps();

    api.assert_schema().assert_table("Account", |table| {
        table
            .assert_pk(|pk| pk.assert_columns(&["id"]))
            .assert_column("code", |column| column.assert_type_family(ColumnTypeFamily::String))
            .assert_column("amount", |column| column.assert_type_family(ColumnTypeFamily::Decimal))
            .assert_column("payload", |column| column.assert_type_family(ColumnTypeFamily::Json))
            .assert_column("data", |column| column.assert_type_family(ColumnTypeFamily::Binary))
            .assert_index_on_columns(&["code"], |index| index.assert_is_unique())
    });

    api.schema_push_w_datasource(schema)
        .send()
        .assert_green()
        .assert_no_steps();
}

#[test_connector(tags(KingbaseOracle))]
fn oracle_migrations_can_be_created_applied_and_reapplied(api: TestApi) {
    let migrations_directory = api.create_migrations_directory();
    let schema = api.datamodel_with_provider(
        r#"
            model Account {
                id      Int    @id
                code    String @unique @db.VarChar2(64)
                balance Decimal @db.Number(12, 4)
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
            .assert_index_on_columns(&["code"], |index| index.assert_is_unique())
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

#[test_connector(tags(KingbaseOracle))]
fn oracle_foreign_keys_and_indexes_can_be_added_and_removed(api: TestApi) {
    let initial_schema = r#"
        model Parent {
            id Int @id
        }

        model Child {
            id       Int @id
            parentId Int
        }
    "#;

    api.schema_push_w_datasource(initial_schema).send().assert_green();

    let relational_schema = r#"
        model Parent {
            id      Int     @id
            children Child[]
        }

        model Child {
            id       Int    @id
            parentId Int
            parent   Parent @relation(fields: [parentId], references: [id], onDelete: Cascade, onUpdate: Cascade)

            @@index([parentId], map: "Child_parentId_idx")
        }
    "#;

    api.schema_push_w_datasource(relational_schema)
        .send()
        .assert_green()
        .assert_has_executed_steps();

    api.assert_schema().assert_table("Child", |table| {
        table
            .assert_foreign_keys_count(1)
            .assert_fk_on_columns(&["parentId"], |fk| {
                fk.assert_references("Parent", &["id"])
                    .assert_referential_action_on_delete(ForeignKeyAction::Cascade)
                    .assert_referential_action_on_update(ForeignKeyAction::Cascade)
            })
            .assert_index_on_columns(&["parentId"], |index| {
                index.assert_is_not_unique().assert_name("Child_parentId_idx")
            })
    });

    api.schema_push_w_datasource(initial_schema)
        .send()
        .assert_green()
        .assert_has_executed_steps();

    api.assert_schema().assert_table("Child", |table| {
        table.assert_foreign_keys_count(0).assert_indexes_count(0)
    });
}

#[test_connector(tags(KingbaseOracle))]
fn oracle_autoincrement_columns_round_trip_through_schema_push(api: TestApi) {
    let schema = r#"
        model Event {
            id   Int    @id @default(autoincrement())
            name String
        }
    "#;

    api.schema_push_w_datasource(schema)
        .send()
        .assert_green()
        .assert_has_executed_steps();

    api.assert_schema().assert_table("Event", |table| {
        table.assert_pk(|pk| pk.assert_columns(&["id"]).assert_has_autoincrement())
    });

    api.insert("Event").value("name", "created").result_raw();
    api.query_raw(r#"SELECT CAST("id" AS VARCHAR2(32)) AS "id" FROM "Event""#, &[])
        .assert_single_row(|row| row.assert_text_value("id", "1"));

    api.schema_push_w_datasource(schema)
        .send()
        .assert_green()
        .assert_no_steps();
}

#[test_connector(tags(KingbaseOracle))]
fn oracle_autoincrement_primary_keys_can_be_referenced_by_foreign_keys(api: TestApi) {
    let migrations_directory = api.create_migrations_directory();
    let schema = api.datamodel_with_provider(
        r#"
            model IntParent {
                id       Int        @id @default(autoincrement())
                children IntChild[]
            }

            model IntChild {
                id       Int       @id @default(autoincrement())
                parentId Int
                parent   IntParent @relation(fields: [parentId], references: [id])
            }

            model BigIntParent {
                id       BigInt        @id @default(autoincrement())
                children BigIntChild[]
            }

            model BigIntChild {
                id       BigInt       @id @default(autoincrement())
                parentId BigInt
                parent   BigIntParent @relation(fields: [parentId], references: [id])
            }
        "#,
    );

    api.create_migration("init", &schema, &migrations_directory).send_sync();
    api.apply_migrations(&migrations_directory)
        .send_sync()
        .assert_applied_migrations(&["init"]);

    api.assert_schema()
        .assert_table("IntParent", |table| {
            table
                .assert_column("id", |column| column.assert_type_family(ColumnTypeFamily::Int))
                .assert_pk(|pk| pk.assert_columns(&["id"]).assert_has_autoincrement())
        })
        .assert_table("IntChild", |table| table.assert_foreign_keys_count(1))
        .assert_table("BigIntParent", |table| {
            table
                .assert_column("id", |column| column.assert_type_family(ColumnTypeFamily::BigInt))
                .assert_pk(|pk| pk.assert_columns(&["id"]).assert_has_autoincrement())
        })
        .assert_table("BigIntChild", |table| table.assert_foreign_keys_count(1));

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

#[test_connector(tags(KingbaseOracle))]
fn oracle_column_type_default_and_arity_changes_are_migrated(api: TestApi) {
    let initial_schema = r#"
        model Task {
            id     Int     @id
            status String? @db.VarChar2(32)
        }
    "#;

    api.schema_push_w_datasource(initial_schema).send().assert_green();

    let changed_schema = r#"
        model Task {
            id     Int    @id
            status String @default("queued") @db.VarChar2(64)
        }
    "#;

    api.schema_push_w_datasource(changed_schema)
        .send()
        .assert_green()
        .assert_has_executed_steps();

    api.insert("Task").value("id", 1).result_raw();
    api.dump_table("Task")
        .assert_single_row(|row| row.assert_text_value("status", "queued"));

    api.assert_schema().assert_table("Task", |table| {
        table.assert_column("status", |column| {
            column
                .assert_is_required()
                .assert_native_type("VarChar2(64)", psl::builtin_connectors::KINGBASE_ORACLE)
        })
    });

    let without_default_schema = r#"
        model Task {
            id     Int    @id
            status String @db.VarChar2(64)
        }
    "#;

    api.schema_push_w_datasource(without_default_schema)
        .send()
        .assert_green()
        .assert_has_executed_steps();

    api.assert_schema().assert_table("Task", |table| {
        table.assert_column("status", |column| column.assert_has_no_default())
    });

    api.schema_push_w_datasource(without_default_schema)
        .send()
        .assert_green()
        .assert_no_steps();
}

#[test_connector(tags(KingbaseOracle))]
fn oracle_named_constraints_and_indexes_can_be_renamed(api: TestApi) {
    let initial_schema = r#"
        model Parent {
            id       Int    @id(map: "Parent_pkey_old")
            code     String @unique(map: "Parent_code_key_old")
            children Child[]
        }

        model Child {
            id       Int    @id(map: "Child_pkey_old")
            parentId Int
            parent   Parent @relation(map: "Child_parent_fkey_old", fields: [parentId], references: [id])

            @@index([parentId], map: "Child_parent_idx_old")
        }
    "#;

    api.schema_push_w_datasource(initial_schema).send().assert_green();

    let changed_schema = r#"
        model Parent {
            id       Int    @id(map: "Parent_pkey_new")
            code     String @unique(map: "Parent_code_key_new")
            children Child[]
        }

        model Child {
            id       Int    @id(map: "Child_pkey_new")
            parentId Int
            parent   Parent @relation(map: "Child_parent_fkey_new", fields: [parentId], references: [id])

            @@index([parentId], map: "Child_parent_idx_new")
        }
    "#;

    api.schema_push_w_datasource(changed_schema)
        .send()
        .assert_green()
        .assert_has_executed_steps();

    api.assert_schema()
        .assert_table("Parent", |table| {
            table
                .assert_pk(|pk| pk.assert_constraint_name("Parent_pkey_new"))
                .assert_index_on_columns(&["code"], |index| index.assert_name("Parent_code_key_new"))
        })
        .assert_table("Child", |table| {
            table
                .assert_pk(|pk| pk.assert_constraint_name("Child_pkey_new"))
                .assert_fk_with_name("Child_parent_fkey_new")
                .assert_index_on_columns(&["parentId"], |index| index.assert_name("Child_parent_idx_new"))
        });

    api.schema_push_w_datasource(changed_schema)
        .send()
        .assert_green()
        .assert_no_steps();
}

#[test_connector(tags(KingbaseOracle))]
fn oracle_enum_columns_can_change_arity(api: TestApi) {
    let initial_schema = r#"
        model Post {
            id     Int     @id
            status Status?
        }

        enum Status {
            DRAFT
            PUBLISHED
        }
    "#;

    api.schema_push_w_datasource(initial_schema).send().assert_green();

    let changed_schema = r#"
        model Post {
            id     Int    @id
            status Status @default(DRAFT)
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
        table.assert_column("status", |column| {
            column.assert_is_required().assert_enum_default("DRAFT")
        })
    });

    api.schema_push_w_datasource(changed_schema)
        .send()
        .assert_green()
        .assert_no_steps();
}

#[test_connector(tags(KingbaseOracle))]
fn oracle_migration_history_can_be_reset_and_reapplied(api: TestApi) {
    let initial_schema = api.datamodel_with_provider(
        r#"
            model Account {
                id   Int    @id @default(autoincrement())
                name String @db.VarChar2(64)
            }
        "#,
    );
    let changed_schema = api.datamodel_with_provider(
        r#"
            model Account {
                id   Int    @id @default(autoincrement())
                name String @db.VarChar2(64)
            }

            model AuditLog {
                id        Int      @id
                createdAt DateTime @default(now()) @db.Timestamp(3)
            }
        "#,
    );
    let migrations_directory = api.create_migrations_directory();

    api.create_migration("01_initial", &initial_schema, &migrations_directory)
        .send_sync();
    api.apply_migrations(&migrations_directory)
        .send_sync()
        .assert_applied_migrations(&["01_initial"]);
    api.create_migration("02_add_audit_log", &changed_schema, &migrations_directory)
        .send_sync();
    api.apply_migrations(&migrations_directory)
        .send_sync()
        .assert_applied_migrations(&["02_add_audit_log"]);

    api.reset().send_sync(None);
    api.assert_schema().assert_tables_count(0);

    api.apply_migrations(&migrations_directory)
        .send_sync()
        .assert_applied_migrations(&["01_initial", "02_add_audit_log"]);
    api.assert_schema()
        .assert_has_table("Account")
        .assert_has_table("AuditLog")
        .assert_has_table("_prisma_migrations");
}

#[test_connector(tags(KingbaseOracle))]
fn oracle_dropping_a_column_removes_its_dependent_index(api: TestApi) {
    let initial_schema = r#"
        model Item {
            id   Int    @id
            code String @db.VarChar2(32)
            name String @db.VarChar2(64)

            @@index([code, name], map: "Item_code_name_idx")
        }
    "#;

    api.schema_push_w_datasource(initial_schema).send().assert_green();

    let changed_schema = r#"
        model Item {
            id   Int    @id
            name String @db.VarChar2(64)
        }
    "#;

    api.schema_push_w_datasource(changed_schema)
        .send()
        .assert_green()
        .assert_has_executed_steps();

    api.assert_schema().assert_table("Item", |table| {
        table.assert_does_not_have_column("code").assert_indexes_count(0)
    });

    api.schema_push_w_datasource(changed_schema)
        .send()
        .assert_green()
        .assert_no_steps();
}

#[test_connector(tags(KingbaseOracle))]
fn oracle_autoincrement_can_be_added_and_removed(api: TestApi) {
    let initial_schema = r#"
        model Event {
            id   Int    @id
            name String
        }
    "#;

    api.schema_push_w_datasource(initial_schema).send().assert_green();

    let autoincrement_schema = r#"
        model Event {
            id   Int    @id @default(autoincrement())
            name String
        }
    "#;

    api.schema_push_w_datasource(autoincrement_schema)
        .send()
        .assert_green()
        .assert_has_executed_steps();
    api.assert_schema().assert_table("Event", |table| {
        table
            .assert_column("id", |column| {
                column.assert_default(Some(DefaultValue::sequence("event_id_seq")))
            })
            .assert_pk(|pk| pk.assert_columns(&["id"]).assert_has_autoincrement())
    });
    api.insert("Event").value("name", "generated").result_raw();
    api.dump_table("Event")
        .assert_single_row(|row| row.assert_text_value("name", "generated"));

    api.schema_push_w_datasource(initial_schema)
        .send()
        .assert_green()
        .assert_has_executed_steps();
    api.assert_schema().assert_table("Event", |table| {
        table.assert_pk(|pk| pk.assert_columns(&["id"]).assert_has_no_autoincrement())
    });

    api.schema_push_w_datasource(initial_schema)
        .send()
        .assert_green()
        .assert_no_steps();
}

#[test_connector(tags(KingbaseOracle))]
fn oracle_table_remapping_with_existing_data_is_reported_as_data_loss(api: TestApi) {
    let initial_schema = r#"
        model Task {
            id   Int    @id
            name String @db.VarChar2(64)

            @@map("Task_old")
        }
    "#;

    api.schema_push_w_datasource(initial_schema).send().assert_green();
    api.insert("Task_old")
        .value("id", 1)
        .value("name", "rename me")
        .result_raw();

    let renamed_schema = r#"
        model Task {
            id   Int    @id
            name String @db.VarChar2(64)

            @@map("Task_new")
        }
    "#;

    let warning = format!(
        "You are about to drop the `{}` table, which is not empty (1 rows).",
        api.normalize_identifier("Task_old")
    );

    api.schema_push_w_datasource(renamed_schema)
        .send()
        .assert_warnings(&[warning.into()])
        .assert_no_steps();
    api.assert_schema()
        .assert_has_table("Task_old")
        .assert_has_no_table("Task_new");
    api.dump_table("Task_old")
        .assert_single_row(|row| row.assert_text_value("name", "rename me"));
}

#[test_connector(tags(KingbaseOracle), namespaces("one", "two"))]
fn oracle_multi_schema_migrations_can_be_created_and_applied(mut api: TestApi) {
    let schema = api.datamodel_with_provider_and_features(
        r#"
            model First {
                id Int @id

                @@schema("one")
            }

            model Second {
                id Int @id

                @@schema("two")
            }
        "#,
        &[("schemas", r#"["one", "two"]"#)],
        &[],
    );
    let migrations_directory = api.create_migrations_directory();

    api.create_migration("initial", &schema, &migrations_directory)
        .send_sync();
    api.apply_migrations(&migrations_directory)
        .send_sync()
        .assert_applied_migrations(&["initial"]);

    let namespaces = Namespaces::from_vec(&mut vec!["one".to_owned(), "two".to_owned()]);
    api.assert_schema_with_namespaces(namespaces)
        .assert_has_table_with_ns("one", "First")
        .assert_has_table_with_ns("two", "Second");
    api.apply_migrations(&migrations_directory)
        .send_sync()
        .assert_applied_migrations(&[]);
}
