use sql_migration_tests::test_api::*;
use sql_schema_describer::{ColumnTypeFamily, ForeignKeyAction};

#[test_connector(tags(KingbaseMysql))]
fn kb_remaining_quoted_identifier_lifecycle(mut api: TestApi) {
    let source = r#"
        model Parent {
            id Int @id @map("id`key")
            children Child[]
            @@map("parent`table")
        }
        model Child {
            id Int @id @map("id`key")
            value String @map("v`name") @db.VarChar(32)
            parentId Int? @map("parent`id")
            parent Parent? @relation(fields: [parentId], references: [id], map: "fk`parent")
            @@index([value], map: "old`index")
            @@unique([id, value], map: "unique`index")
            @@map("child`table")
        }
    "#;
    api.raw_cmd("SET sql_mode = ''");
    api.schema_push_w_datasource(source).send().assert_green();
    api.raw_cmd("INSERT INTO `parent``table` VALUES (1)");
    api.raw_cmd("INSERT INTO `child``table` VALUES (1, 'kept', 1)");
    let query = quaint::ast::Select::from_table("child`table").column("v`name");
    let (sql, parameters) = quaint::visitor::KingbaseMysql::build(query).unwrap();
    assert!(sql.contains("`child``table`"), "{sql}");
    api.query_raw(&sql, &parameters)
        .assert_single_row(|row| row.assert_text_value("v`name", "kept"));
    let renamed = source.replace("old`index", "new`index");
    api.schema_push_w_datasource(&renamed)
        .send()
        .assert_green()
        .assert_has_executed_steps();
    api.schema_push_w_datasource(&renamed)
        .send()
        .assert_green()
        .assert_no_steps();
    let removed = renamed
        .replace("@@unique([id, value], map: \"unique`index\")", "")
        .replace("@@index([value], map: \"new`index\")", "");
    api.schema_push_w_datasource(&removed)
        .send()
        .assert_green()
        .assert_has_executed_steps();
    api.schema_push_w_datasource(&removed)
        .send()
        .assert_green()
        .assert_no_steps();
    api.query_raw("SELECT `v``name` FROM `child``table`", &[])
        .assert_single_row(|row| row.assert_text_value("v`name", "kept"));
}

#[test_connector(tags(KingbaseMysql))]
fn kb_remaining_shared_index_names_are_table_scoped(mut api: TestApi) {
    for attribute in ["index", "unique", "fulltext"] {
        let schema = |left: bool, right: bool, name: &str| {
            [("Left", left), ("Right", right)]
                .into_iter()
                .map(|(table, enabled)| {
                    let index = if enabled {
                        format!("@@{attribute}([value], map: \"{name}\")")
                    } else {
                        String::new()
                    };
                    format!("model {table} {{\n id Int @id\n value String @db.VarChar(32)\n {index}\n }}")
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        let both = schema(true, true, "shared");
        api.schema_push_w_datasource(&both).send().assert_green();
        api.schema_push_w_datasource(&both)
            .send()
            .assert_green()
            .assert_no_steps();
        for table in ["Left", "Right"] {
            api.raw_cmd(&format!("INSERT INTO `{table}` VALUES (1, 'kept')"));
            api.assert_schema().assert_table(table, |table| {
                table.assert_index_on_columns(&["value"], |index| index.assert_name("shared"))
            });
        }
        let renamed = schema(true, true, "renamed");
        api.schema_push_w_datasource(&renamed)
            .send()
            .assert_green()
            .assert_has_executed_steps();
        api.schema_push_w_datasource(&renamed)
            .send()
            .assert_green()
            .assert_no_steps();
        let right_only = schema(false, true, "renamed");
        api.schema_push_w_datasource(&right_only)
            .send()
            .assert_green()
            .assert_has_executed_steps();
        api.schema_push_w_datasource(&right_only)
            .send()
            .assert_green()
            .assert_no_steps();
        let none = schema(false, false, "renamed");
        api.schema_push_w_datasource(&none)
            .send()
            .assert_green()
            .assert_has_executed_steps();
        api.schema_push_w_datasource(&none)
            .send()
            .assert_green()
            .assert_no_steps();
        for table in ["Left", "Right"] {
            api.query_raw(&format!("SELECT value FROM `{table}` WHERE id=1"), &[])
                .assert_single_row(|row| row.assert_text_value("value", "kept"));
        }
        api.raw_cmd("DROP TABLE `Left`; DROP TABLE `Right`");
    }

    // Different table/column partitions can produce the same default name.
    let schema =
        "model A {\n id Int @id\n b_c Int\n @@index([b_c])\n }\n model A_b {\n id Int @id\n c Int\n @@index([c])\n }";
    api.schema_push_w_datasource(schema).send().assert_green();
    api.schema_push_w_datasource(schema)
        .send()
        .assert_green()
        .assert_no_steps();
    for (table, column) in [("A", "b_c"), ("A_b", "c")] {
        api.assert_schema().assert_table(table, |table| {
            table.assert_index_on_columns(&[column], |index| index.assert_name("A_b_c_idx"))
        });
    }
    api.raw_cmd("ALTER TABLE `A` RENAME TO `Moved`");
    let renamed = schema
        .replace("model A {", "model A {\n @@map(\"Moved\")")
        .replace("@@index([b_c])", "@@index([b_c], map: \"A_b_c_idx\")");
    api.schema_push_w_datasource(&renamed)
        .send()
        .assert_green()
        .assert_no_steps();
}

#[test_connector(tags(KingbaseMysql))]
fn kb_remaining_small_unsigned_ranges_round_trip(mut api: TestApi) {
    use quaint::prelude::Queryable;
    let source = "model SmallUnsigned {\n id Int @id\n tiny Int? @db.UnsignedTinyInt\n small Int? @db.UnsignedSmallInt\n medium Int? @db.UnsignedMediumInt\n }";
    api.schema_push_w_datasource(source).send().assert_green();
    api.schema_push_w_datasource(source)
        .send()
        .assert_green()
        .assert_no_steps();
    api.raw_cmd("INSERT INTO `SmallUnsigned` VALUES (1,255,65535,16777215),(2,NULL,NULL,NULL)");
    let conn = tok(quaint::single::Quaint::new(api.connection_string())).unwrap();
    for (column, maximum) in [("tiny", 255), ("small", 65535), ("medium", 16777215)] {
        for value in [-1, maximum + 1] {
            let error =
                tok(conn.raw_cmd(&format!("UPDATE `SmallUnsigned` SET `{column}`={value} WHERE id=1"))).unwrap_err();
            assert!(
                matches!(error.original_code(), Some("23514" | "22003" | "22P02")),
                "{error:?}"
            );
        }
        api.query_raw(
            &format!("SELECT `{column}`::bigint AS n FROM `SmallUnsigned` WHERE id=1"),
            &[],
        )
        .assert_single_row(|row| row.assert_int_value("n", maximum));
    }
    api.schema_push_w_datasource(source)
        .send()
        .assert_green()
        .assert_no_steps();
    let widened = source.replace("UnsignedTinyInt", "UnsignedSmallInt");
    api.schema_push_w_datasource(&widened)
        .force(true)
        .send()
        .assert_executable()
        .assert_has_executed_steps();
    api.raw_cmd("UPDATE `SmallUnsigned` SET tiny=256 WHERE id=1");
    api.schema_push_w_datasource(&widened)
        .send()
        .assert_green()
        .assert_no_steps();
    let error = api.schema_push_w_datasource(source).force(true).send_unwrap_err();
    assert!(error.to_string().contains("__prisma_uint8_range"), "{error}");
    api.query_raw("SELECT tiny::bigint AS n FROM `SmallUnsigned` WHERE id=1", &[])
        .assert_single_row(|row| row.assert_int_value("n", 256));
    api.schema_push_w_datasource(&widened)
        .send()
        .assert_green()
        .assert_no_steps();
}

#[test_connector(tags(KingbaseMysql))]
fn kb_remaining_small_unsigned_autoincrement_and_relations(mut api: TestApi) {
    use quaint::prelude::Queryable;
    for (native, maximum) in [
        ("UnsignedTinyInt", 255),
        ("UnsignedSmallInt", 65535),
        ("UnsignedMediumInt", 16777215),
    ] {
        let schema = format!(
            "model AutoParent {{\n id Int @id @default(autoincrement()) @db.{native}\n children AutoChild[]\n }}\n\
            model AutoChild {{\n id Int @id\n parentId Int @db.{native}\n parent AutoParent @relation(fields:[parentId],references:[id])\n }}"
        );
        api.schema_push_w_datasource(&schema).send().assert_green();
        api.raw_cmd("INSERT INTO `AutoParent` DEFAULT VALUES");
        api.raw_cmd("INSERT INTO `AutoChild` VALUES (1,1)");
        let conn = tok(quaint::single::Quaint::new(api.connection_string())).unwrap();
        tok(conn.execute_raw("UPDATE `AutoChild` SET parentId=? WHERE id=?", &[1.into(), 1.into()])).unwrap();
        let error = tok(conn.raw_cmd(&format!("INSERT INTO `AutoParent` (id) VALUES ({})", maximum + 1))).unwrap_err();
        assert_eq!(error.original_code(), Some("23514"), "{error:?}");
        api.query_raw("SELECT id::bigint AS n FROM `AutoParent`", &[])
            .assert_single_row(|row| row.assert_int_value("n", 1));
        api.schema_push_w_datasource(&schema)
            .send()
            .assert_green()
            .assert_no_steps();
        if native == "UnsignedTinyInt" {
            let widened = schema.replace("UnsignedTinyInt", "UnsignedSmallInt");
            api.schema_push_w_datasource(&widened)
                .force(true)
                .send()
                .assert_executable()
                .assert_warnings(&["You are about to alter the column `id` on the `AutoParent` table, which contains 1 non-null values. The data in that column will be cast from `UnsignedTinyInt` to `UnsignedSmallInt`.".into(), "You are about to alter the column `parentId` on the `AutoChild` table, which contains 1 non-null values. The data in that column will be cast from `UnsignedTinyInt` to `UnsignedSmallInt`.".into()]);
            api.raw_cmd("INSERT INTO `AutoParent` (id) VALUES (256)");
            let narrowed = widened.replace(
                "@default(autoincrement()) @db.UnsignedSmallInt",
                "@default(autoincrement()) @db.UnsignedTinyInt",
            );
            let error = api.schema_push_w_datasource(&narrowed).force(true).send_unwrap_err();
            assert!(error.to_string().contains("Value out of range"), "{error}");
            api.schema_push_w_datasource(&widened)
                .send()
                .assert_green()
                .assert_no_steps();
            let error = tok(conn.raw_cmd("INSERT INTO `AutoParent` (id) VALUES (65536)")).unwrap_err();
            assert_eq!(
                error.original_code(),
                Some("23514"),
                "guard must survive rollback: {error:?}"
            );
            api.query_raw("SELECT count(*)::int AS n FROM `AutoParent`", &[])
                .assert_single_row(|row| row.assert_int_value("n", 2));
        }
        api.raw_cmd("DROP TABLE `AutoChild`; DROP TABLE `AutoParent`");
    }
}

#[test_connector(tags(KingbaseMysql))]
fn kb_remaining_unsigned_domain_collision_is_rejected(mut api: TestApi) {
    api.raw_cmd("CREATE DOMAIN public.__prisma_uint8 AS sys.uint4 CHECK(VALUE <= 999)");
    let schema = "model Collision {\n id Int @id\n value Int @db.UnsignedTinyInt\n }";
    let error = api.schema_push_w_datasource(schema).send_unwrap_err();
    assert!(error.to_string().contains("incompatible definition"), "{error}");
}

#[test_connector(tags(KingbaseMysql))]
fn kb_remaining_native_metadata_migrations_replay(mut api: TestApi) {
    let migrations = api.create_migrations_directory();
    let initial = api.datamodel_with_provider(
        "model Left {\n id Int @id @default(autoincrement()) @db.UnsignedTinyInt\n value Int @db.UnsignedTinyInt\n @@unique([value], map: \"shared\")\n }\n model Right {\n id Int @id\n value Int @db.UnsignedTinyInt\n @@index([value], map: \"shared\")\n }",
    );
    let widened = initial.replace("UnsignedTinyInt", "UnsignedSmallInt");
    let renamed = widened.replace("\"shared\"", "\"renamed\"");
    for (name, schema) in [("01_init", &initial), ("02_widen", &widened), ("03_rename", &renamed)] {
        api.create_migration(name, schema, &migrations).send_sync();
        api.apply_migrations(&migrations)
            .send_sync()
            .assert_applied_migrations(&[name]);
        if name == "01_init" {
            api.raw_cmd("INSERT INTO `Left` (value) VALUES (255); INSERT INTO `Right` VALUES (1,255)");
        }
    }
    assert!(
        api.diagnose_migration_history(&migrations)
            .send_sync()
            .into_output()
            .is_empty()
    );
    api.schema_push(renamed).send().assert_green().assert_no_steps();
    api.query_raw("SELECT value::bigint AS n FROM `Left` WHERE id=1", &[])
        .assert_single_row(|row| row.assert_int_value("n", 255));
}

fn set_default_foreign_key_schema(nullable: bool, default: &str) -> String {
    let arity = if nullable { "?" } else { "" };
    format!(
        "model DefaultParent {{\n id Int @id\n children DefaultChild[]\n }}\n\
         model DefaultChild {{\n id Int @id\n parentId Int{arity} {default}\n \
         parent DefaultParent{arity} @relation(fields: [parentId], references: [id], onDelete: SetDefault, onUpdate: SetDefault)\n }}"
    )
}

#[test_connector(tags(KingbaseMysql))]
fn set_default_foreign_keys_apply_on_delete_and_update(mut api: TestApi) {
    use psl::parser_database::NoExtensionTypes;
    use schema_core::schema_connector::DiffTarget;

    let schema = api.datamodel_with_provider(&set_default_foreign_key_schema(false, "@default(1)"));
    let script = api.connector_diff(
        DiffTarget::Database,
        DiffTarget::Datamodel(vec![("schema.prisma".into(), schema.clone().into())], &NoExtensionTypes),
        None,
    );
    assert!(
        script.contains("ON DELETE SET DEFAULT ON UPDATE SET DEFAULT"),
        "{script}"
    );
    api.schema_push(schema.clone()).send().assert_green();
    api.assert_schema().assert_table("DefaultChild", |table| {
        table
            .assert_foreign_keys_count(1)
            .assert_fk_on_columns(&["parentId"], |fk| {
                fk.assert_references("DefaultParent", &["id"])
                    .assert_referential_action_on_delete(ForeignKeyAction::SetDefault)
                    .assert_referential_action_on_update(ForeignKeyAction::SetDefault)
            })
    });
    api.raw_cmd("INSERT INTO `DefaultParent` VALUES (1), (2), (3)");
    api.raw_cmd("INSERT INTO `DefaultChild` VALUES (10, 2), (11, 3)");
    api.raw_cmd("DELETE FROM `DefaultParent` WHERE id = 2");
    api.raw_cmd("UPDATE `DefaultParent` SET id = 4 WHERE id = 3");
    api.query_raw(
        "SELECT COUNT(*)::int AS n FROM `DefaultChild` WHERE id IN (10,11) AND parentId = 1",
        &[],
    )
    .assert_single_row(|row| row.assert_int_value("n", 2));
    api.query_raw("SELECT COUNT(*)::int AS n FROM `DefaultParent` WHERE id IN (1,4)", &[])
        .assert_single_row(|row| row.assert_int_value("n", 2));
    api.schema_push(schema).send().assert_green().assert_no_steps();
}

#[test_connector(tags(KingbaseMysql))]
fn set_default_without_explicit_default_sets_nullable_foreign_keys_to_null(mut api: TestApi) {
    let schema = set_default_foreign_key_schema(true, "");
    api.schema_push_w_datasource(&schema).send().assert_green();
    api.raw_cmd("INSERT INTO `DefaultParent` VALUES (2), (3)");
    api.raw_cmd("INSERT INTO `DefaultChild` VALUES (10, 2), (11, 3)");
    api.raw_cmd("DELETE FROM `DefaultParent` WHERE id = 2");
    api.raw_cmd("UPDATE `DefaultParent` SET id = 4 WHERE id = 3");
    api.query_raw(
        "SELECT COUNT(*)::int AS n FROM `DefaultChild` WHERE id IN (10,11) AND parentId IS NULL",
        &[],
    )
    .assert_single_row(|row| row.assert_int_value("n", 2));
    api.schema_push_w_datasource(&schema)
        .send()
        .assert_green()
        .assert_no_steps();
}

#[test_connector(tags(KingbaseMysql))]
fn set_default_still_enforces_foreign_key_constraints(mut api: TestApi) {
    use quaint::prelude::Queryable;

    let schema = set_default_foreign_key_schema(false, "@default(1)");
    api.schema_push_w_datasource(&schema).send().assert_green();
    api.raw_cmd("INSERT INTO `DefaultParent` VALUES (2), (3)");
    api.raw_cmd("INSERT INTO `DefaultChild` VALUES (10, 2), (11, 3)");
    let conn = tok(quaint::single::Quaint::new(api.connection_string())).unwrap();
    // No parent with the default id 1 exists. Both actions must fail instead
    // of creating dangling child references, preserving the original rows.
    for sql in [
        "DELETE FROM `DefaultParent` WHERE id = 2",
        "UPDATE `DefaultParent` SET id = 4 WHERE id = 3",
    ] {
        // The Schema Engine raw_cmd wrapper does not expose a Client P2003
        // here. Check the native constraint error and SQLSTATE directly.
        let error = tok(conn.raw_cmd(sql)).unwrap_err();
        assert_eq!(error.original_code(), Some("23503"), "{error}");
        assert!(
            matches!(
                error.kind(),
                quaint::error::ErrorKind::ForeignKeyConstraintViolation { .. }
            ),
            "{error:?}"
        );
        api.query_raw("SELECT COUNT(*)::int AS n FROM `DefaultChild` WHERE (id = 10 AND parentId = 2) OR (id = 11 AND parentId = 3)", &[])
            .assert_single_row(|row| row.assert_int_value("n", 2));
        api.query_raw("SELECT COUNT(*)::int AS n FROM `DefaultParent` WHERE id IN (2,3)", &[])
            .assert_single_row(|row| row.assert_int_value("n", 2));
    }
    api.raw_cmd("INSERT INTO `DefaultParent` VALUES (1)");
    api.raw_cmd("DELETE FROM `DefaultParent` WHERE id = 2");
    api.raw_cmd("UPDATE `DefaultParent` SET id = 4 WHERE id = 3");
    api.query_raw(
        "SELECT COUNT(*)::int AS n FROM `DefaultChild` WHERE id IN (10,11) AND parentId = 1",
        &[],
    )
    .assert_single_row(|row| row.assert_int_value("n", 2));
    api.schema_push_w_datasource(&schema)
        .send()
        .assert_green()
        .assert_no_steps();
}

#[test_connector(tags(KingbaseMysql))]
fn full_column_text_and_blob_keys_are_enforced_and_idempotent(mut api: TestApi) {
    let mut schema = String::new();
    let mut cases = Vec::new();
    for (index, (native, scalar)) in [
        ("Text", "String"),
        ("TinyText", "String"),
        ("MediumText", "String"),
        ("LongText", "String"),
        ("Blob", "Bytes"),
        ("TinyBlob", "Bytes"),
        ("MediumBlob", "Bytes"),
        ("LongBlob", "Bytes"),
    ]
    .into_iter()
    .enumerate()
    {
        for (shape, columns) in [("single", "value"), ("compound", "value, other")] {
            for (kind, attribute) in [("normal", "index"), ("unique", "unique"), ("primary", "id")] {
                let table = format!("key_{index}_{shape}_{kind}");
                let id_key = if kind == "primary" { "@unique" } else { "@id" };
                schema.push_str(&format!(
                    "model {table} {{\n id Int {id_key}\n value {scalar} @db.{native}\n other {scalar} @db.{native}\n @@{attribute}([{columns}])\n }}\n"
                ));
                cases.push((table, kind));
            }
        }
    }
    api.schema_push_w_datasource(&schema)
        .send()
        .assert_green()
        .assert_has_executed_steps();
    for (table, kind) in &cases {
        api.raw_cmd(&format!("INSERT INTO `{table}` VALUES (1, 'same', 'same')"));
        let duplicate = format!("INSERT INTO `{table}` VALUES (2, 'same', 'same')");
        if *kind == "normal" {
            api.raw_cmd(&duplicate);
        } else {
            let error = tok(api.connector.raw_cmd(&duplicate)).unwrap_err();
            assert_eq!(error.error_code(), Some("P2002"), "{table}: {error}");
        }
        api.query_raw(&format!("SELECT COUNT(*)::INT AS count FROM `{table}`"), &[])
            .assert_single_row(|row| row.assert_int_value("count", if *kind == "normal" { 2 } else { 1 }));
    }
    api.schema_push_w_datasource(&schema)
        .send()
        .assert_green()
        .assert_no_steps();
    api.schema_push_w_datasource(&schema.replace("Bytes @db.LongBlob", "Bytes"))
        .send()
        .assert_green()
        .assert_no_steps();
}

#[test_connector(tags(KingbaseMysql))]
fn extended_native_parameters_preserve_values_and_are_idempotent(mut api: TestApi) {
    let schema = r#"
        model NativeLimits {
            id Int @id
            c String @db.Char(256)
            v String @db.VarChar(655350)
            p Decimal @db.Decimal(66,20)
            s Decimal @db.Decimal(44,33)
            n Decimal @db.Decimal(2,4)
            c0 String? @db.Char(0)
            v0 String? @db.VarChar(0)
            max_char String? @db.Char(10485760)
            max_varchar String? @db.VarChar(10485760)
            max_decimal Decimal? @db.Decimal(1000,1000)
        }
    "#;
    api.schema_push_w_datasource(schema)
        .send()
        .assert_green()
        .assert_has_executed_steps();
    api.raw_cmd(
        "INSERT INTO `NativeLimits`(id,c,v,p,s,n) VALUES (1,'kept','kept', \
         1234567890123456789012345678901234567890.12345678901234567890, \
         123.123456789012345678901234567890123, 0.1234)",
    );
    api.schema_push_w_datasource(schema)
        .send()
        .assert_green()
        .assert_no_steps();
    api.schema_push_w_datasource(&schema.replace("Decimal(2,4)", "Decimal(4,4)"))
        .send()
        .assert_green()
        .assert_no_steps();
    api.query_raw(
        "SELECT RTRIM(c) AS c, v, p::TEXT AS p, s::TEXT AS s, n::TEXT AS n FROM `NativeLimits` WHERE id=1",
        &[],
    )
    .assert_single_row(|row| {
        row.assert_text_value("c", "kept")
            .assert_text_value("v", "kept")
            .assert_text_value("p", "1234567890123456789012345678901234567890.12345678901234567890")
            .assert_text_value("s", "123.123456789012345678901234567890123")
            .assert_text_value("n", "0.1234")
    });
}

#[test_connector(tags(KingbaseMysql))]
fn extended_native_parameter_migrations_can_be_replayed(api: TestApi) {
    let migrations = api.create_migrations_directory();
    let schema = api.datamodel_with_provider(
        "model NativeLimits {\n id Int @id\n c String @db.Char(256)\n v String @db.VarChar(655350)\n \
         p Decimal @db.Decimal(66,20)\n s Decimal @db.Decimal(44,33)\n n Decimal @db.Decimal(2,4)\n }",
    );
    api.create_migration("01_init", &schema, &migrations).send_sync();
    api.apply_migrations(&migrations)
        .send_sync()
        .assert_applied_migrations(&["01_init"]);
    assert!(
        api.diagnose_migration_history(&migrations)
            .send_sync()
            .into_output()
            .is_empty()
    );
}

#[test_connector(tags(KingbaseMysql))]
fn function_shaped_identifiers_can_be_queried(mut api: TestApi) {
    use quaint::ast::{Column, Comparable, Select};

    api.raw_cmd(
        "CREATE TABLE `uuid()` (`uuid()` INT, `JSON_UNQUOTE(v)` INT, `JSON_CONTAINS(a,b)` INT, \
         `JSON_EXTRACT(v,CONCAT(a,b))` INT, `SUM(?)` INT); \
         INSERT INTO `uuid()` VALUES (1, 2, 3, 4, 5);",
    );
    let query = Select::from_table("uuid()")
        .column("uuid()")
        .column("JSON_UNQUOTE(v)")
        .column("JSON_CONTAINS(a,b)")
        .column("JSON_EXTRACT(v,CONCAT(a,b))")
        .column("SUM(?)")
        .so_that(Column::from("uuid()").equals(1));
    api.query(query.into()).assert_single_row(|row| {
        row.assert_int_value("uuid()", 1)
            .assert_int_value("JSON_UNQUOTE(v)", 2)
            .assert_int_value("JSON_CONTAINS(a,b)", 3)
            .assert_int_value("JSON_EXTRACT(v,CONCAT(a,b))", 4)
            .assert_int_value("SUM(?)", 5)
    });
}

#[test_connector(tags(KingbaseMysql))]
fn descending_indexes_are_preserved_and_changes_are_applied(mut api: TestApi) {
    let initial = r#"
        model Sorted {
            id Int @id
            a Int
            b Int
            @@index([a, b(sort: Desc)], map: "mixed_order")
            @@unique([b(sort: Desc), a], map: "unique_order")
        }
    "#;
    api.schema_push_w_datasource(initial).send().assert_green();
    api.raw_cmd("INSERT INTO `Sorted` VALUES (1, 2, 3)");
    api.schema_push_w_datasource(initial)
        .send()
        .assert_green()
        .assert_no_steps();

    let changed = initial.replace("@@index([a, b(sort: Desc)]", "@@index([a(sort: Desc), b]");
    api.schema_push_w_datasource(&changed)
        .send()
        .assert_green()
        .assert_has_executed_steps();
    api.schema_push_w_datasource(&changed)
        .send()
        .assert_green()
        .assert_no_steps();
    api.query_raw("SELECT a FROM `Sorted` WHERE id = 1 AND b = 3", &[])
        .assert_single_row(|row| row.assert_int_value("a", 2));
}

#[test_connector(tags(KingbaseMysql))]
fn calculated_descending_unique_index_can_be_removed(mut api: TestApi) {
    use psl::parser_database::NoExtensionTypes;
    use schema_core::schema_connector::DiffTarget;

    let initial = api.datamodel_with_provider("model SortedUnique {\n id Int @id\n value Int @unique(sort: Desc)\n }");
    let target = initial.replace("@unique(sort: Desc)", "");
    api.schema_push(initial.clone()).send().assert_green();
    api.raw_cmd("INSERT INTO `SortedUnique` VALUES (1, 2)");
    let script = api.connector_diff(
        DiffTarget::Datamodel(vec![("schema.prisma".into(), initial.into())], &NoExtensionTypes),
        DiffTarget::Datamodel(vec![("schema.prisma".into(), target.clone().into())], &NoExtensionTypes),
        None,
    );
    assert!(script.contains("DROP INDEX `_prisma_idx_"), "{script}");
    assert!(!script.contains("DROP CONSTRAINT"), "{script}");
    api.raw_cmd(&script);
    api.query_raw("SELECT value FROM `SortedUnique` WHERE id = 1", &[])
        .assert_single_row(|row| row.assert_int_value("value", 2));
    api.schema_push(target).send().assert_green().assert_no_steps();
}

#[test_connector(tags(KingbaseMysql))]
fn descending_unique_index_migrations_can_be_replayed(api: TestApi) {
    let migrations = api.create_migrations_directory();
    let initial = api.datamodel_with_provider("model SortedUnique {\n id Int @id\n value Int @unique(sort: Desc)\n }");
    let target = initial.replace("@unique(sort: Desc)", "");
    for (name, schema) in [("01_init", &initial), ("02_remove_unique", &target)] {
        api.create_migration(name, schema, &migrations).send_sync();
        api.apply_migrations(&migrations)
            .send_sync()
            .assert_applied_migrations(&[name]);
        if name == "01_init" {
            api.raw_cmd("INSERT INTO `SortedUnique` VALUES (1, 2)");
        }
    }
    api.query_raw("SELECT value FROM `SortedUnique` WHERE id = 1", &[])
        .assert_single_row(|row| row.assert_int_value("value", 2));
    assert!(
        api.diagnose_migration_history(&migrations)
            .send_sync()
            .into_output()
            .is_empty()
    );
}

#[test_connector(tags(KingbaseMysql))]
fn tinyblob_values_can_be_migrated_to_bits(api: TestApi) {
    assert_blob_values_can_be_migrated_to_bits(api, "TinyBlob");
}

#[test_connector(tags(KingbaseMysql))]
fn temporal_values_fit_decimal_integer_capacity(mut api: TestApi) {
    use psl::parser_database::NoExtensionTypes;
    use schema_core::schema_connector::DiffTarget;

    for (native, precision, input, expected) in [
        ("Date", 10, "2026-10-24", "20261024.00"),
        ("Time(3)", 8, "12:34:56.789", "123456.00"),
        ("DateTime(3)", 16, "2026-10-24 12:34:56.789", "20261024123456.00"),
        ("Timestamp(3)", 16, "2026-10-24 12:34:56.789", "20261024123456.00"),
    ] {
        let source = format!("model TemporalDecimal {{\n id Int @id\n value DateTime? @db.{native}\n }}");
        let target = api.datamodel_with_provider(&format!(
            "model TemporalDecimal {{\n id Int @id\n value Decimal? @db.Decimal({precision},2)\n }}"
        ));
        api.schema_push_w_datasource(&source).send().assert_green();
        api.raw_cmd(&format!(
            "INSERT INTO `TemporalDecimal` VALUES (1, '{input}'), (2, NULL)"
        ));

        let warnings = [format!(
            "You are about to alter the column `value` on the `TemporalDecimal` table, which contains 1 non-null values. \
             The data in that column will be cast from `{native}` to `Decimal({precision},2)`."
        ).into()];
        api.schema_push(target.clone())
            .send()
            .assert_executable()
            .assert_warnings(&warnings)
            .assert_no_steps();
        api.schema_push_w_datasource(&source)
            .send()
            .assert_green()
            .assert_no_steps();
        let script = api.connector_diff(
            DiffTarget::Database,
            DiffTarget::Datamodel(vec![("schema.prisma".into(), target.clone().into())], &NoExtensionTypes),
            None,
        );
        assert!(script.contains("USING"), "{script}");
        assert!(!script.contains("DROP COLUMN"), "{script}");
        api.schema_push(target.clone())
            .force(true)
            .send()
            .assert_executable()
            .assert_warnings(&warnings)
            .assert_has_executed_steps();
        api.query_raw(
            &format!("SELECT COUNT(*)::int AS n FROM `TemporalDecimal` WHERE (id = 1 AND value = {expected}) OR (id = 2 AND value IS NULL)"),
            &[],
        ).assert_single_row(|row| row.assert_int_value("n", 2));
        api.schema_push(target).send().assert_green().assert_no_steps();
        api.raw_cmd("DROP TABLE `TemporalDecimal`");
    }
}

#[test_connector(tags(KingbaseMysql))]
fn undersized_temporal_decimal_rebuild_checks_required_rows(mut api: TestApi) {
    let source = "model TemporalDecimal {\n id Int @id\n value DateTime @db.Date\n }";
    let target = "model TemporalDecimal {\n id Int @id\n value Decimal @db.Decimal(8,2)\n }";
    api.schema_push_w_datasource(source).send().assert_green();
    // No data to lose: an empty required column can be recreated.
    api.schema_push_w_datasource(target)
        .send()
        .assert_green()
        .assert_has_executed_steps();
    api.schema_push_w_datasource(target)
        .send()
        .assert_green()
        .assert_no_steps();
    api.raw_cmd("DROP TABLE `TemporalDecimal`");

    api.schema_push_w_datasource(source).send().assert_green();
    api.raw_cmd("INSERT INTO `TemporalDecimal` VALUES (1, '2026-10-24')");
    let errors = ["Changed the type of `value` on the `TemporalDecimal` table. No cast exists, the column would be dropped and recreated, which cannot be done since the column is required and there is data in the table.".to_owned()];
    for force in [false, true] {
        api.schema_push_w_datasource(target)
            .force(force)
            .send()
            .assert_unexecutable(&errors)
            .assert_no_steps();
        api.query_raw(
            "SELECT to_char(value, 'YYYY-MM-DD') AS original FROM `TemporalDecimal` WHERE id = 1",
            &[],
        )
        .assert_single_row(|row| row.assert_text_value("original", "2026-10-24"));
        api.schema_push_w_datasource(source)
            .send()
            .assert_green()
            .assert_no_steps();
    }
}

#[test_connector(tags(KingbaseMysql))]
fn undersized_temporal_decimal_rebuild_warns_before_discarding_values(mut api: TestApi) {
    let source = "model TemporalDecimal {\n id Int @id\n value DateTime? @db.Date\n }";
    let target = "model TemporalDecimal {\n id Int @id\n value Decimal? @db.Decimal(8,2)\n }";
    api.schema_push_w_datasource(source).send().assert_green();
    api.raw_cmd("INSERT INTO `TemporalDecimal` VALUES (1, '2026-10-24')");
    let warnings = ["The `value` column on the `TemporalDecimal` table would be dropped and recreated. This will lead to data loss.".into()];
    api.schema_push_w_datasource(target)
        .send()
        .assert_executable()
        .assert_warnings(&warnings)
        .assert_no_steps();
    api.query_raw(
        "SELECT to_char(value, 'YYYY-MM-DD') AS original FROM `TemporalDecimal` WHERE id = 1",
        &[],
    )
    .assert_single_row(|row| row.assert_text_value("original", "2026-10-24"));
    // Explicitly forced destructive migration: the row/id survives, the old
    // value does not. This must never be asserted as value-preserving casting.
    api.schema_push_w_datasource(target)
        .force(true)
        .send()
        .assert_executable()
        .assert_warnings(&warnings)
        .assert_has_executed_steps();
    api.query_raw(
        "SELECT COUNT(*)::int AS n FROM `TemporalDecimal` WHERE id = 1 AND value IS NULL",
        &[],
    )
    .assert_single_row(|row| row.assert_int_value("n", 1));
    api.schema_push_w_datasource(target)
        .send()
        .assert_green()
        .assert_no_steps();
}

#[test_connector(tags(KingbaseMysql))]
fn blob_values_can_be_migrated_to_bits(api: TestApi) {
    assert_blob_values_can_be_migrated_to_bits(api, "Blob");
}

#[test_connector(tags(KingbaseMysql))]
fn mediumblob_values_can_be_migrated_to_bits(api: TestApi) {
    assert_blob_values_can_be_migrated_to_bits(api, "MediumBlob");
}

#[test_connector(tags(KingbaseMysql))]
fn longblob_values_can_be_migrated_to_bits(api: TestApi) {
    assert_blob_values_can_be_migrated_to_bits(api, "LongBlob");
}

fn assert_blob_values_can_be_migrated_to_bits(mut api: TestApi, native_type: &str) {
    use psl::parser_database::NoExtensionTypes;
    use schema_core::schema_connector::DiffTarget;

    api.raw_cmd("SET sql_mode = 'STRICT_ALL_TABLES'");
    let source = format!(
        "model BlobCast {{\n id Int @id\n b1 Bytes? @db.{native_type}\n b8 Bytes? @db.{native_type}\n \
         b16 Bytes? @db.{native_type}\n b64 Bytes? @db.{native_type}\n }}"
    );
    let target = api.datamodel_with_provider(
        "model BlobCast {\n id Int @id\n b1 Boolean? @db.Bit(1)\n b8 Bytes? @db.Bit(8)\n \
         b16 Bytes? @db.Bit(16)\n b64 Bytes? @db.Bit(64)\n }",
    );
    api.schema_push_w_datasource(&source).send().assert_green();
    api.raw_cmd(&format!(
        r#"INSERT INTO `BlobCast` VALUES
        (1, NULL, NULL, NULL, NULL),
        (2, '', '', '', ''),
        (3, '\xff'::bytea, '\xff'::bytea, '\xff'::bytea, '\xff'::bytea),
        (4, '1', '255', '256', '18446744073709551615'),
        (5, '1e2', '12.7', '12abc', '-1'),
        (6, 'abc', '  +42  ', '\x31320033'::bytea, '9223372036854775808'),
        (7, '0', '0', '0', '-9223372036854775809'),
        (8, concat(repeat('0', 100), '1')::{native_type}, concat(repeat('0', 100), '255')::{native_type},
         concat(repeat('0', 100), '65535')::{native_type}, repeat('9', 100)::{native_type})"#,
    ));

    let warnings: Vec<_> = [1, 8, 16, 64]
        .map(|width| {
            format!(
                "You are about to alter the column `b{width}` on the `BlobCast` table, which contains 7 non-null values. \
                 The data in that column will be cast from `{native_type}` to `Bit({width})`."
            )
            .into()
        })
        .into();
    api.schema_push(target.clone())
        .send()
        .assert_executable()
        .assert_warnings(&warnings)
        .assert_no_steps();
    api.query_raw("SELECT b8::text AS original FROM `BlobCast` WHERE id = 4", &[])
        .assert_single_row(|row| row.assert_text_value("original", "255"));

    let script = api.connector_diff(
        DiffTarget::Database,
        DiffTarget::Datamodel(vec![("schema.prisma".into(), target.clone().into())], &NoExtensionTypes),
        None,
    );
    assert!(script.contains("USING"));
    assert!(!script.contains("DROP COLUMN"));
    api.schema_push(target.clone())
        .force(true)
        .send()
        .assert_executable()
        .assert_warnings(&warnings)
        .assert_has_executed_steps();

    // Expected decimal-prefix conversion results were independently checked on
    // MySQL 8.0.46, including signed saturation and non-UTF-8 / NUL bytes.
    let expected = [
        [0, 0, 0, 0],
        [0, 0, 0, 0],
        [1, 255, 256, i64::MAX],
        [1, 12, 12, -1],
        [0, 42, 12, i64::MAX],
        [0, 0, 0, i64::MIN],
        [1, 255, 65535, i64::MAX],
    ];
    for (offset, values) in expected.into_iter().enumerate() {
        for (width, value) in [1, 8, 16, 64].into_iter().zip(values) {
            api.query_raw(
                &format!(
                    "SELECT CASE WHEN b{width} IS NOT DISTINCT FROM ({value})::bigint::BIT({width}) \
                     THEN 1 ELSE 0 END AS matches FROM `BlobCast` WHERE id = {}",
                    offset + 2,
                ),
                &[],
            )
            .assert_single_row(|row| row.assert_int_value("matches", 1));
        }
    }
    api.query_raw(
        "SELECT CASE WHEN b1 IS NULL AND b8 IS NULL AND b16 IS NULL AND b64 IS NULL \
         THEN 1 ELSE 0 END AS matches FROM `BlobCast` WHERE id = 1",
        &[],
    )
    .assert_single_row(|row| row.assert_int_value("matches", 1));
    api.query_raw("SELECT COUNT(*)::int AS n FROM `BlobCast`", &[])
        .assert_single_row(|row| row.assert_int_value("n", 8));
    api.schema_push(target).send().assert_green().assert_no_steps();
}

#[test_connector(tags(KingbaseMysql))]
fn empty_blob_columns_can_be_migrated_to_bits(mut api: TestApi) {
    let source = "model EmptyBlob {\n id Int @id\n a Bytes @db.TinyBlob\n b Bytes @db.Blob\n \
                  c Bytes @db.MediumBlob\n d Bytes @db.LongBlob\n }";
    let target = "model EmptyBlob {\n id Int @id\n a Boolean @db.Bit(1)\n b Bytes @db.Bit(8)\n \
                  c Bytes @db.Bit(16)\n d Bytes @db.Bit(64)\n }";
    api.schema_push_w_datasource(source).send().assert_green();
    api.schema_push_w_datasource(target)
        .send()
        .assert_green()
        .assert_has_executed_steps();
    api.schema_push_w_datasource(target)
        .send()
        .assert_green()
        .assert_no_steps();
}

#[test_connector(tags(KingbaseMysql))]
fn blob_to_bit_overflow_in_strict_mode_preserves_source(mut api: TestApi) {
    // C010 accepts STRICT_ALL_TABLES, but rejects STRICT_TRANS_TABLES.
    api.raw_cmd("SET sql_mode = 'STRICT_ALL_TABLES'");
    for value in ["256", "-1"] {
        for native_type in ["TinyBlob", "Blob", "MediumBlob", "LongBlob"] {
            let source = format!("model BlobRange {{\n id Int @id\n v Bytes @db.{native_type}\n }}");
            let target = "model BlobRange {\n id Int @id\n v Bytes @db.Bit(8)\n }";
            api.schema_push_w_datasource(&source).send().assert_green();
            api.raw_cmd(&format!("INSERT INTO `BlobRange` VALUES (1, '{value}')"));
            let error = api.schema_push_w_datasource(target).force(true).send_unwrap_err();
            assert!(error.to_string().contains("out of range for BIT(8)"), "{error}");
            api.query_raw("SELECT v::text AS original FROM `BlobRange` WHERE id = 1", &[])
                .assert_single_row(|row| row.assert_text_value("original", value));
            api.schema_push_w_datasource(&source)
                .send()
                .assert_green()
                .assert_no_steps();
            api.raw_cmd("DROP TABLE `BlobRange`");
        }
    }
}

#[test_connector(tags(KingbaseMysql))]
fn blob_to_bit_overflow_without_strict_mode_saturates(mut api: TestApi) {
    api.raw_cmd("SET sql_mode = ''");
    for native_type in ["TinyBlob", "Blob", "MediumBlob", "LongBlob"] {
        let source = format!("model BlobRange {{\n id Int @id\n v Bytes @db.{native_type}\n }}");
        let target = "model BlobRange {\n id Int @id\n v Bytes @db.Bit(8)\n }";
        api.schema_push_w_datasource(&source).send().assert_green();
        api.raw_cmd("INSERT INTO `BlobRange` VALUES (1, '256'), (2, '-1')");
        api.schema_push_w_datasource(target)
            .force(true)
            .send()
            .assert_executable()
            .assert_has_executed_steps();
        api.query_raw(
            "SELECT COUNT(*)::int AS n FROM `BlobRange` WHERE v = 255::bigint::BIT(8)",
            &[],
        )
        .assert_single_row(|row| row.assert_int_value("n", 2));
        api.schema_push_w_datasource(target)
            .send()
            .assert_green()
            .assert_no_steps();
        api.raw_cmd("DROP TABLE `BlobRange`");
    }
}

#[test_connector(tags(KingbaseMysql))]
fn custom_primary_key_name_is_used_when_changing_id(api: TestApi) {
    api.raw_cmd(
        r#"
        CREATE TABLE `PkTarget` (
            id INT NOT NULL,
            nextId INT NOT NULL,
            name VARCHAR(64) NOT NULL,
            CONSTRAINT custom_primary_key PRIMARY KEY(id)
        );
        "#,
    );
    assert_primary_key_change_uses_constraint_name(api, "custom_primary_key");
}

#[test_connector(tags(KingbaseMysql))]
fn renamed_table_primary_key_name_is_used_when_changing_id(api: TestApi) {
    api.raw_cmd(
        r#"
        CREATE TABLE `PkOriginal` (
            id INT NOT NULL PRIMARY KEY,
            nextId INT NOT NULL,
            name VARCHAR(64) NOT NULL
        );
        ALTER TABLE `PkOriginal` RENAME TO `PkTarget`;
        "#,
    );
    assert_primary_key_change_uses_constraint_name(api, "PkOriginal_pkey");
}

fn assert_primary_key_change_uses_constraint_name(mut api: TestApi, constraint_name: &str) {
    use psl::parser_database::NoExtensionTypes;
    use schema_core::schema_connector::DiffTarget;

    api.raw_cmd("INSERT INTO `PkTarget` (id, nextId, name) VALUES (1, 2, 'kept')");
    let schema = api.datamodel_with_provider(
        r#"
        model PkTarget {
            id     Int
            nextId Int    @id
            name   String @db.VarChar(64)
        }
        "#,
    );
    let script = api.connector_diff(
        DiffTarget::Database,
        DiffTarget::Datamodel(vec![("schema.prisma".into(), schema.clone().into())], &NoExtensionTypes),
        None,
    );
    api.raw_cmd(&script);
    assert!(script.contains(&format!("DROP CONSTRAINT `{constraint_name}`")));
    api.assert_schema()
        .assert_table("PkTarget", |table| table.assert_pk(|pk| pk.assert_columns(&["nextId"])));
    api.query_raw("SELECT name FROM `PkTarget` WHERE id = 1 AND nextId = 2", &[])
        .assert_single_row(|row| row.assert_text_value("name", "kept"));
    api.schema_push(schema).send().assert_green().assert_no_steps();
}

#[test_connector(tags(KingbaseMysql))]
fn calculated_primary_key_name_handles_long_table_names(api: TestApi) {
    use psl::parser_database::NoExtensionTypes;
    use schema_core::schema_connector::DiffTarget;

    // Exactly 63 bytes: the generated constraint name must preserve its _pkey
    // suffix by shortening the table portion, rather than truncate the suffix.
    let table_name = "abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijk";
    let initial = api.datamodel_with_provider(&format!(
        r#"
        model LongPkTable {{
            id     Int @id
            nextId Int

            @@map("{table_name}")
        }}
        "#,
    ));
    let target = api.datamodel_with_provider(&format!(
        r#"
        model LongPkTable {{
            id     Int
            nextId Int @id

            @@map("{table_name}")
        }}
        "#,
    ));
    api.schema_push(initial.clone()).send().assert_green();
    api.raw_cmd(&format!("INSERT INTO `{table_name}` (id, nextId) VALUES (1, 2)"));

    let script = api.connector_diff(
        DiffTarget::Datamodel(vec![("schema.prisma".into(), initial.into())], &NoExtensionTypes),
        DiffTarget::Datamodel(vec![("schema.prisma".into(), target.clone().into())], &NoExtensionTypes),
        None,
    );
    api.raw_cmd(&script);
    api.assert_schema()
        .assert_table(table_name, |table| table.assert_pk(|pk| pk.assert_columns(&["nextId"])));
    api.query_raw(&format!("SELECT id FROM `{table_name}` WHERE nextId = 2"), &[])
        .assert_single_row(|row| row.assert_int_value("id", 1));
    api.schema_push(target).send().assert_green().assert_no_steps();
}

#[test_connector(tags(KingbaseMysql))]
fn index_and_primary_key_changes_work_without_ansi_quotes(api: TestApi) {
    assert_ddl_identifier_quoting(api, "");
}

#[test_connector(tags(KingbaseMysql))]
fn index_and_primary_key_changes_work_with_ansi_quotes(api: TestApi) {
    assert_ddl_identifier_quoting(api, "ANSI_QUOTES");
}

fn assert_ddl_identifier_quoting(mut api: TestApi, sql_mode: &str) {
    use psl::parser_database::NoExtensionTypes;
    use schema_core::schema_connector::DiffTarget;

    api.raw_cmd(&format!("SET sql_mode = '{sql_mode}'"));
    api.query_raw(
        "SELECT CASE WHEN current_setting('sql_mode') LIKE '%ANSI_QUOTES%' THEN 1 ELSE 0 END AS ansi_quotes",
        &[],
    )
    .assert_single_row(|row| row.assert_int_value("ansi_quotes", if sql_mode == "ANSI_QUOTES" { 1 } else { 0 }));

    let initial_schema = r#"
        model QuoteTarget {
            id     Int    @id
            nextId Int
            name   String @db.VarChar(64)

            @@index([name], map: "old_index")
        }
    "#;
    let target_schema = api.datamodel_with_provider(
        r#"
        model QuoteTarget {
            id     Int
            nextId Int    @id
            name   String @db.VarChar(64)

            @@index([name], map: "new_index")
        }
        "#,
    );
    api.schema_push_w_datasource(initial_schema).send().assert_green();
    api.raw_cmd("INSERT INTO `QuoteTarget` (id, `nextId`, name) VALUES (1, 2, 'kept')");

    let script = api.connector_diff(
        DiffTarget::Database,
        DiffTarget::Datamodel(
            vec![("schema.prisma".into(), target_schema.clone().into())],
            &NoExtensionTypes,
        ),
        None,
    );
    api.raw_cmd(&script);
    // Custom logical names use schema-wide physical identifiers. Keep checking
    // backtick syntax, then execute and verify the logical name below.
    assert!(script.contains("ALTER INDEX `_prisma_idx_"), "{script}");
    assert!(script.contains(" RENAME TO `_prisma_idx_"), "{script}");
    assert!(script.contains("prisma:logical-index:new_index"), "{script}");
    assert!(script.contains("DROP CONSTRAINT `QuoteTarget_pkey`"));
    api.assert_schema().assert_table("QuoteTarget", |table| {
        table
            .assert_pk(|pk| pk.assert_columns(&["nextId"]))
            .assert_index_on_columns(&["name"], |index| index.assert_name("new_index"))
    });
    api.query_raw("SELECT name FROM `QuoteTarget` WHERE id = 1 AND `nextId` = 2", &[])
        .assert_single_row(|row| row.assert_text_value("name", "kept"));
    api.schema_push(target_schema).send().assert_green().assert_no_steps();
}

#[test_connector(tags(KingbaseMysql))]
fn added_unique_index_can_be_removed(api: TestApi) {
    let without_unique = r#"
        model Account {
            id    Int    @id
            email String @db.VarChar(64)
        }
    "#;
    let with_unique = r#"
        model Account {
            id    Int    @id
            email String @unique @db.VarChar(64)
        }
    "#;

    api.schema_push_w_datasource(without_unique).send().assert_green();
    api.schema_push_w_datasource(with_unique)
        .force(true)
        .send()
        .assert_executable()
        .assert_warnings(&[
            "A unique constraint covering the columns `[email]` on the table `Account` will be added. If there are existing duplicate values, this will fail."
                .into(),
        ])
        .assert_has_executed_steps();
    api.assert_schema().assert_table("Account", |table| {
        table.assert_index_on_columns(&["email"], |index| index.assert_is_unique())
    });
    api.raw_cmd("INSERT INTO `Account` (`id`, `email`) VALUES (1, 'same@example.com')");

    api.schema_push_w_datasource(without_unique)
        .send()
        .assert_green()
        .assert_has_executed_steps();
    api.assert_schema()
        .assert_table("Account", |table| table.assert_indexes_count(0));
    api.raw_cmd("INSERT INTO `Account` (`id`, `email`) VALUES (2, 'same@example.com')");
    api.schema_push_w_datasource(without_unique)
        .send()
        .assert_green()
        .assert_no_steps();
}

#[test_connector(tags(KingbaseMysql))]
fn calculated_unique_constraint_can_be_removed(api: TestApi) {
    use psl::parser_database::NoExtensionTypes;
    use schema_core::schema_connector::DiffTarget;

    let with_unique = api.datamodel_with_provider(
        r#"
        model Account {
            id    Int    @id
            email String @unique @db.VarChar(64)
        }
        "#,
    );
    let without_unique = api.datamodel_with_provider(
        r#"
        model Account {
            id    Int    @id
            email String @db.VarChar(64)
        }
        "#,
    );

    let script = api.connector_diff(
        DiffTarget::Datamodel(
            vec![("schema.prisma".into(), with_unique.clone().into())],
            &NoExtensionTypes,
        ),
        DiffTarget::Datamodel(
            vec![("schema.prisma".into(), without_unique.clone().into())],
            &NoExtensionTypes,
        ),
        None,
    );
    expect![[r#"
        -- DropIndex
        ALTER TABLE `Account` DROP CONSTRAINT `_prisma_idx_b8a6725f7952edc8`;
    "#]]
    .assert_eq(&script);

    api.schema_push(with_unique).send().assert_green();
    api.raw_cmd(&script);
    api.schema_push(without_unique).send().assert_green().assert_no_steps();
}

#[test_connector(tags(KingbaseMysql))]
fn added_unique_index_migrations_can_be_replayed(api: TestApi) {
    let migrations = api.create_migrations_directory();
    let without_unique = api.datamodel_with_provider(
        r#"
        model Account {
            id    Int    @id
            email String @db.VarChar(64)
        }
        "#,
    );
    let with_unique = api.datamodel_with_provider(
        r#"
        model Account {
            id    Int    @id
            email String @unique @db.VarChar(64)
        }
        "#,
    );

    for (name, schema) in [
        ("01_init", &without_unique),
        ("02_add_unique", &with_unique),
        ("03_remove_unique", &without_unique),
    ] {
        api.create_migration(name, schema, &migrations).send_sync();
        api.apply_migrations(&migrations)
            .send_sync()
            .assert_applied_migrations(&[name]);
    }

    api.assert_schema()
        .assert_table("Account", |table| table.assert_indexes_count(0));
    api.raw_cmd("INSERT INTO `Account` (id, email) VALUES (1, 'same@example.com'), (2, 'same@example.com')");
    api.apply_migrations(&migrations)
        .send_sync()
        .assert_applied_migrations(&[]);
    assert!(
        api.diagnose_migration_history(&migrations)
            .send_sync()
            .into_output()
            .is_empty()
    );
}

#[test_connector(tags(KingbaseMysql))]
fn inline_unique_constraint_can_be_removed(api: TestApi) {
    let with_unique = r#"
        model Account {
            id    Int    @id
            email String @unique @db.VarChar(64)
        }
    "#;
    let without_unique = r#"
        model Account {
            id    Int    @id
            email String @db.VarChar(64)
        }
    "#;

    api.schema_push_w_datasource(with_unique).send().assert_green();
    api.raw_cmd("INSERT INTO `Account` (`id`, `email`) VALUES (1, 'same@example.com')");
    api.schema_push_w_datasource(without_unique)
        .send()
        .assert_green()
        .assert_has_executed_steps();
    api.assert_schema()
        .assert_table("Account", |table| table.assert_indexes_count(0));
    api.raw_cmd("INSERT INTO `Account` (`id`, `email`) VALUES (2, 'same@example.com')");
    api.schema_push_w_datasource(without_unique)
        .send()
        .assert_green()
        .assert_no_steps();
}

#[test_connector(tags(KingbaseMysql))]
fn mixed_unique_objects_can_be_removed(api: TestApi) {
    api.raw_cmd(
        r#"
        CREATE TABLE `Account` (
            id INT NOT NULL PRIMARY KEY,
            email VARCHAR(64) NOT NULL,
            tenant INT NOT NULL,
            UNIQUE INDEX account_email_unique(email)
        );
        CREATE UNIQUE INDEX account_email_tenant_unique ON `Account`(email, tenant);
        CREATE INDEX account_tenant_index ON `Account`(tenant);
        INSERT INTO `Account`(id, email, tenant) VALUES (1, 'same@example.com', 1);
        "#,
    );

    let schema = r#"
        model Account {
            id     Int    @id
            email  String @db.VarChar(64)
            tenant Int
        }
    "#;
    api.schema_push_w_datasource(schema)
        .send()
        .assert_green()
        .assert_has_executed_steps();
    api.assert_schema()
        .assert_table("Account", |table| table.assert_indexes_count(0));
    api.raw_cmd("INSERT INTO `Account`(id, email, tenant) VALUES (2, 'same@example.com', 1)");
    api.schema_push_w_datasource(schema)
        .send()
        .assert_green()
        .assert_no_steps();
}

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
