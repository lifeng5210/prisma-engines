//! SQL visitor for KingbaseES running in Oracle-compatible mode.
//!
//! Kingbase Oracle mode uses the PostgreSQL wire protocol, but its SQL dialect
//! and default scalar types are Oracle-oriented. Keep the renderer independent
//! from [`super::Postgres`]: a wire-protocol match must not silently select
//! PostgreSQL syntax for Oracle-mode schemas.

use crate::visitor::query_writer::QueryWriter;
use crate::{
    ast::*,
    error::{Error, ErrorKind},
    visitor::{self, Visitor},
};
use query_template::{PlaceholderFormat, QueryTemplate};
use std::{borrow::Cow, fmt};

/// A visitor to generate SQL accepted by KingbaseES Oracle-compatible mode.
pub struct KingbaseOracle<'a> {
    query_template: QueryTemplate<Value<'a>>,
}

impl<'a> KingbaseOracle<'a> {
    fn unsupported(feature: &'static str) -> Error {
        Error::builder(ErrorKind::QueryInvalidInput(format!(
            "Kingbase Oracle does not support {feature} queries."
        )))
        .build()
    }

    fn write_quoted_string(&mut self, value: &str) -> visitor::Result {
        self.write("'")?;
        self.write(value.replace('\'', "''"))?;
        self.write("'")
    }

    fn visit_returning(&mut self, returning: Option<Vec<Column<'a>>>) -> visitor::Result {
        if let Some(returning) = returning
            && !returning.is_empty()
        {
            self.write(" RETURNING ")?;
            self.visit_columns(returning.into_iter().map(Into::into).collect())?;
        }

        Ok(())
    }

    fn visit_insert_values(&mut self, columns: Vec<Column<'a>>, values: Expression<'a>) -> visitor::Result {
        match values {
            Expression {
                kind: ExpressionKind::Parameterized(row),
                ..
            } => {
                self.visit_insert_columns(columns)?;
                self.write(" VALUES ")?;
                self.query_template.write_parameter_tuple_list("(", ",", ")", ",");
                self.query_template.parameters.push(row);
            }
            Expression {
                kind: ExpressionKind::Row(row),
                ..
            } => {
                if row.values.is_empty() {
                    self.write(" DEFAULT VALUES")?;
                } else {
                    self.visit_insert_columns(columns)?;
                    self.write(" VALUES ")?;
                    self.visit_row(row)?;
                }
            }
            Expression {
                kind: ExpressionKind::Values(values),
                ..
            } => {
                self.visit_insert_columns(columns)?;
                self.write(" VALUES ")?;

                let values_len = values.len();
                for (index, row) in values.into_iter().enumerate() {
                    self.visit_row(row)?;

                    if index < values_len - 1 {
                        self.write(", ")?;
                    }
                }
            }
            expression => self.surround_with("(", ")", |visitor| visitor.visit_expression(expression))?,
        }

        Ok(())
    }

    fn visit_insert_columns(&mut self, columns: Vec<Column<'a>>) -> visitor::Result {
        self.write(" (")?;
        let len = columns.len();

        for (index, column) in columns.into_iter().enumerate() {
            self.visit_column(column.into_bare())?;

            if index < len - 1 {
                self.write(",")?;
            }
        }

        self.write(")")
    }

    /// Kingbase Oracle SQL/JSON functions currently operate on JSONB values.
    /// Casting here keeps the public native `JSON` type usable in expressions
    /// while retaining Oracle-mode SQL function syntax.
    fn visit_json_as_jsonb(&mut self, expression: Expression<'a>) -> visitor::Result {
        self.write("CAST(")?;
        self.visit_expression(expression)?;
        self.write(" AS JSONB)")
    }

    fn visit_json_path(&mut self, path: JsonPath<'a>) -> visitor::Result {
        let path = match path {
            JsonPath::String(path) => path.into_owned(),
            JsonPath::Array(path) => {
                let mut oracle_path = String::from("$");

                for segment in path {
                    if !segment.is_empty() && segment.bytes().all(|byte| byte.is_ascii_digit()) {
                        oracle_path.push('[');
                        oracle_path.push_str(&segment);
                        oracle_path.push(']');
                    } else {
                        oracle_path.push_str(".\"");

                        for character in segment.chars() {
                            if matches!(character, '\\' | '\"') {
                                oracle_path.push('\\');
                            }
                            oracle_path.push(character);
                        }

                        oracle_path.push('\"');
                    }
                }

                oracle_path
            }
        };

        self.visit_parameterized(Value::text(path))
    }

    fn visit_json_type(&mut self, expression: Expression<'a>) -> visitor::Result {
        self.write("JSONB_TYPEOF(")?;
        self.visit_json_as_jsonb(expression)?;
        self.write(")")
    }

    fn visit_json_aware_comparison(
        &mut self,
        left: Expression<'a>,
        operator: &str,
        right: Expression<'a>,
    ) -> visitor::Result {
        if left.is_json_value() || right.is_json_value() {
            self.visit_json_as_jsonb(left)?;
            self.write(operator)?;
            self.visit_json_as_jsonb(right)
        } else {
            self.visit_expression(left)?;
            self.write(operator)?;
            self.visit_expression(right)
        }
    }

    fn upsert_source_query(columns: &[Column<'a>], values: Expression<'a>) -> crate::Result<Query<'a>> {
        let select_from_row = |row: Row<'a>| {
            columns
                .iter()
                .zip(row.values)
                .fold(Select::default(), |query, (column, value)| {
                    query.value(value.alias(column.name.clone()))
                })
        };

        match values.kind {
            ExpressionKind::Row(row) => Ok(select_from_row(row).into()),
            ExpressionKind::Values(values) => {
                let mut rows = values.rows.into_iter();
                let Some(first_row) = rows.next() else {
                    let kind = ErrorKind::conversion("An upsert needs at least one insert row.");
                    return Err(Error::builder(kind).build());
                };

                let union = rows.fold(Union::new(select_from_row(first_row)), |union, row| {
                    union.all(select_from_row(row))
                });
                Ok(union.into())
            }
            ExpressionKind::Selection(selection) => Ok(selection.into()),
            ExpressionKind::Parameterized(value) => {
                Ok(Select::default().value(ExpressionKind::ParameterizedRow(value)).into())
            }
            _ => {
                let kind = ErrorKind::conversion("Unsupported insert value expression for an Oracle upsert.");
                Err(Error::builder(kind).build())
            }
        }
    }

    fn visit_upsert_update(
        &mut self,
        table: Table<'a>,
        columns: Vec<Column<'a>>,
        values: Expression<'a>,
        update: Update<'a>,
        constraints: Vec<Column<'a>>,
        returning: Option<Vec<Column<'a>>>,
    ) -> visitor::Result {
        if returning.is_some() || update.returning.is_some() {
            return Err(Self::unsupported("MERGE upsert with RETURNING"));
        }

        if constraints.is_empty() {
            let kind = ErrorKind::conversion("An Oracle upsert needs at least one conflict constraint.");
            return Err(Error::builder(kind).build());
        }

        if update
            .columns
            .iter()
            .any(|column| constraints.iter().any(|constraint| constraint.name == column.name))
        {
            return Err(Self::unsupported("MERGE upsert updating a conflict constraint"));
        }

        let source_query = Self::upsert_source_query(&columns, values)?;
        let source_columns: Vec<_> = columns.into_iter().map(Column::into_bare).collect();
        let mut on_conditions = ConditionTree::NoCondition;

        for constraint in constraints {
            let target = constraint.into_bare().table(table.clone());
            let source = target.clone().table("source");
            let condition: ConditionTree<'a> = source.equals(target).into();
            on_conditions = match on_conditions {
                ConditionTree::NoCondition => condition,
                conditions => conditions.and(condition),
            };
        }

        self.write("MERGE INTO ")?;
        self.visit_table(table, true)?;
        self.write(" USING ")?;
        self.surround_with("(", ")", |visitor| visitor.visit_query(source_query))?;
        self.write(" source ")?;
        self.visit_row(Row::from(source_columns.clone()))?;
        self.write(" ON (")?;
        self.visit_conditions(on_conditions)?;
        self.write(") WHEN MATCHED THEN UPDATE SET ")?;

        let pairs = update.columns.into_iter().zip(update.values);
        let pair_count = pairs.len();
        for (index, (column, value)) in pairs.enumerate() {
            self.visit_column(column.into_bare())?;
            self.write(" = ")?;
            self.visit_expression(value)?;

            if index < pair_count - 1 {
                self.write(", ")?;
            }
        }

        if let Some(conditions) = update.conditions {
            self.write(" WHERE ")?;
            self.visit_conditions(conditions)?;
        }

        self.write(" WHEN NOT MATCHED THEN INSERT")?;
        self.visit_insert_columns(source_columns.clone())?;
        self.write(" VALUES ")?;
        self.visit_row(Row::from(
            source_columns
                .into_iter()
                .map(|column| column.table("source"))
                .collect::<Vec<_>>(),
        ))?;

        if let Some(comment) = update.comment {
            self.write(" ")?;
            self.visit_comment(comment)?;
        }

        Ok(())
    }
}

impl<'a> Visitor<'a> for KingbaseOracle<'a> {
    const C_BACKTICK_OPEN: &'static str = "\"";
    const C_BACKTICK_CLOSE: &'static str = "\"";
    const C_WILDCARD: &'static str = "%";

    fn build_template<Q>(query: Q) -> crate::Result<QueryTemplate<Value<'a>>>
    where
        Q: Into<Query<'a>>,
    {
        let mut this = Self {
            query_template: QueryTemplate::new(PlaceholderFormat {
                prefix: "$",
                has_numbering: true,
            }),
        };

        Self::visit_query(&mut this, query.into())?;

        Ok(this.query_template)
    }

    fn write(&mut self, value: impl fmt::Display) -> visitor::Result {
        self.query_template.write_string_chunk(value.to_string());
        Ok(())
    }

    fn add_parameter(&mut self, value: Value<'a>) {
        self.query_template.parameters.push(value);
    }

    fn parameter_substitution(&mut self) -> visitor::Result {
        self.query_template.write_parameter();
        Ok(())
    }

    fn visit_parameterized_row(
        &mut self,
        value: Value<'a>,
        item_prefix: impl Into<Cow<'static, str>>,
        separator: impl Into<Cow<'static, str>>,
        item_suffix: impl Into<Cow<'static, str>>,
    ) -> visitor::Result {
        self.query_template
            .write_parameter_tuple(item_prefix, separator, item_suffix);
        self.query_template.parameters.push(value);
        Ok(())
    }

    fn visit_parameterized_enum_array(
        &mut self,
        _variants: Vec<EnumVariant<'a>>,
        _name: Option<EnumName<'a>>,
    ) -> visitor::Result {
        Err(Self::unsupported("array bind parameter"))
    }

    fn visit_limit_and_offset(&mut self, limit: Option<Value<'a>>, offset: Option<Value<'a>>) -> visitor::Result {
        match (limit, offset) {
            (Some(limit), Some(offset)) => {
                self.write(" OFFSET ")?;
                self.visit_parameterized(offset)?;
                self.write(" ROWS FETCH NEXT ")?;
                self.visit_parameterized(limit)?;
                self.write(" ROWS ONLY")
            }
            (None, Some(offset)) => {
                self.write(" OFFSET ")?;
                self.visit_parameterized(offset)?;
                self.write(" ROWS")
            }
            (Some(limit), None) => {
                self.write(" FETCH FIRST ")?;
                self.visit_parameterized(limit)?;
                self.write(" ROWS ONLY")
            }
            (None, None) => Ok(()),
        }
    }

    fn visit_ordering(&mut self, ordering: Ordering<'a>) -> visitor::Result {
        let len = ordering.0.len();

        for (index, (value, ordering)) in ordering.0.into_iter().enumerate() {
            let direction = ordering.map(|direction| match direction {
                Order::Asc => " ASC",
                Order::Desc => " DESC",
                Order::AscNullsFirst => " ASC NULLS FIRST",
                Order::AscNullsLast => " ASC NULLS LAST",
                Order::DescNullsFirst => " DESC NULLS FIRST",
                Order::DescNullsLast => " DESC NULLS LAST",
            });

            self.visit_expression(value)?;
            self.write(direction.unwrap_or(""))?;

            if index < len - 1 {
                self.write(", ")?;
            }
        }

        Ok(())
    }

    fn visit_concat(&mut self, concat: Concat<'a>) -> visitor::Result {
        let len = concat.exprs.len();

        self.surround_with("(", ")", |visitor| {
            for (index, expression) in concat.exprs.into_iter().enumerate() {
                visitor.visit_expression(expression)?;

                if index < len - 1 {
                    visitor.write(" || ")?;
                }
            }

            Ok(())
        })
    }

    fn visit_equals(&mut self, left: Expression<'a>, right: Expression<'a>) -> visitor::Result {
        self.visit_json_aware_comparison(left, " = ", right)
    }

    fn visit_not_equals(&mut self, left: Expression<'a>, right: Expression<'a>) -> visitor::Result {
        self.visit_json_aware_comparison(left, " != ", right)
    }

    fn visit_insert(&mut self, mut insert: Insert<'a>) -> visitor::Result {
        match insert.on_conflict.take() {
            Some(OnConflict::DoNothing) => return self.visit_merge(Merge::try_from(insert)?),
            Some(OnConflict::Update(update, constraints)) => {
                let table = insert.table.ok_or_else(|| {
                    Error::builder(ErrorKind::conversion("An upsert needs an insert target table.")).build()
                })?;

                return self.visit_upsert_update(
                    table,
                    insert.columns,
                    insert.values,
                    update,
                    constraints,
                    insert.returning,
                );
            }
            None => (),
        }

        self.write("INSERT ")?;

        if let Some(table) = insert.table {
            self.write("INTO ")?;
            self.visit_table(table, true)?;
        }

        self.visit_insert_values(insert.columns, insert.values)?;
        self.visit_returning(insert.returning)?;

        if let Some(comment) = insert.comment {
            self.write(" ")?;
            self.visit_comment(comment)?;
        }

        Ok(())
    }

    fn visit_merge(&mut self, merge: Merge<'a>) -> visitor::Result {
        if merge.returning.is_some() {
            return Err(Self::unsupported("MERGE upsert with RETURNING"));
        }

        self.write("MERGE INTO ")?;
        self.visit_table(merge.table, true)?;
        self.write(" USING ")?;
        self.surround_with("(", ")", |visitor| visitor.visit_query(merge.using.base_query))?;
        self.write(" ")?;
        self.visit_table(merge.using.as_table, false)?;
        self.write(" ")?;
        self.visit_row(Row::from(merge.using.columns))?;
        self.write(" ON (")?;
        self.visit_conditions(merge.using.on_conditions)?;
        self.write(")")?;

        if let Some(query) = merge.when_not_matched {
            self.write(" WHEN NOT MATCHED THEN ")?;
            self.visit_query(query)?;
        }

        Ok(())
    }

    fn visit_delete(&mut self, delete: Delete<'a>) -> visitor::Result {
        self.write("DELETE FROM ")?;
        self.visit_table(delete.table, true)?;

        if let Some(conditions) = delete.conditions {
            self.write(" WHERE ")?;
            self.visit_conditions(conditions)?;
        }

        self.visit_returning(delete.returning)?;

        if let Some(comment) = delete.comment {
            self.write(" ")?;
            self.visit_comment(comment)?;
        }

        Ok(())
    }

    fn visit_aggregate_to_string(&mut self, value: Expression<'a>) -> visitor::Result {
        // Kingbase Oracle mode provides STRING_AGG. LISTAGG would require an
        // ordering expression which Quaint's aggregate AST does not carry.
        self.write("STRING_AGG")?;
        self.surround_with("(", ", ',' )", |visitor| visitor.visit_expression(value))
    }

    fn visit_raw_value(&mut self, value: Value<'a>) -> visitor::Result {
        let result = match &value.typed {
            ValueType::Int32(value) => value.map(|value| self.write(value)),
            ValueType::Int64(value) => value.map(|value| self.write(value)),
            ValueType::Numeric(value) => value.as_ref().map(|value| self.write(value)),
            ValueType::Float(value) => value.map(|value| match value {
                value if value.is_nan() => self.write("'NaN'"),
                value if value == f32::INFINITY => self.write("'Infinity'"),
                value if value == f32::NEG_INFINITY => self.write("'-Infinity'"),
                value => self.write(format!("{value:?}")),
            }),
            ValueType::Double(value) => value.map(|value| match value {
                value if value.is_nan() => self.write("'NaN'"),
                value if value == f64::INFINITY => self.write("'Infinity'"),
                value if value == f64::NEG_INFINITY => self.write("'-Infinity'"),
                value => self.write(format!("{value:?}")),
            }),
            ValueType::Boolean(value) => value.map(|value| self.write(if value { "1" } else { "0" })),
            ValueType::Text(value) => value.as_deref().map(|value| self.write_quoted_string(value)),
            ValueType::Enum(value, _) => value.as_deref().map(|value| self.write_quoted_string(value)),
            ValueType::Char(value) => value.map(|value| self.write_quoted_string(&value.to_string())),
            ValueType::Bytes(value) => value
                .as_deref()
                .map(|value| self.write(format!("decode('{}', 'hex')", hex::encode(value)))),
            ValueType::Json(value) => value.as_ref().map(|value| {
                let json = serde_json::to_string(value)?;
                self.write_quoted_string(&json)
            }),
            ValueType::Xml(value) => value.as_deref().map(|value| self.write_quoted_string(value)),
            ValueType::Uuid(value) => value.map(|value| self.write_quoted_string(&value.hyphenated().to_string())),
            ValueType::DateTime(value) => value.map(|value| {
                self.write(format!(
                    "TIMESTAMP '{}'",
                    value.naive_utc().format("%Y-%m-%d %H:%M:%S%.f")
                ))
            }),
            ValueType::Date(value) => value.map(|value| self.write(format!("DATE '{value}'"))),
            ValueType::Time(value) => value.map(|value| self.write_quoted_string(&value.to_string())),
            ValueType::Array(_) | ValueType::EnumArray(_, _) => return Err(Self::unsupported("array literal")),
            ValueType::Opaque(value) => {
                return Err(Error::builder(ErrorKind::OpaqueAsRawValue(value.to_string())).build());
            }
        };

        result.unwrap_or_else(|| self.write("NULL"))
    }

    fn visit_json_extract(&mut self, json_extract: JsonExtract<'a>) -> visitor::Result {
        if json_extract.extract_as_string {
            self.write("JSON_VALUE(")?;
            self.visit_json_as_jsonb(*json_extract.column)?;
            self.write(", ")?;
            self.visit_json_path(json_extract.path)?;
            self.write(")")
        } else {
            self.write("CAST(JSON_QUERY(")?;
            self.visit_json_as_jsonb(*json_extract.column)?;
            self.write(", ")?;
            self.visit_json_path(json_extract.path)?;
            self.write(") AS JSONB)")
        }
    }

    fn visit_json_extract_last_array_item(&mut self, extract: JsonExtractLastArrayElem<'a>) -> visitor::Result {
        self.write("CAST(JSON_QUERY(")?;
        self.visit_json_as_jsonb(*extract.expr)?;
        self.write(", '$[last]') AS JSONB)")
    }

    fn visit_json_extract_first_array_item(&mut self, extract: JsonExtractFirstArrayElem<'a>) -> visitor::Result {
        self.write("CAST(JSON_QUERY(")?;
        self.visit_json_as_jsonb(*extract.expr)?;
        self.write(", '$[0]') AS JSONB)")
    }

    fn visit_json_array_contains(&mut self, left: Expression<'a>, right: Expression<'a>, not: bool) -> visitor::Result {
        if not {
            self.write("(NOT ")?;
        }

        self.visit_json_as_jsonb(left)?;
        self.write(" @> ")?;
        self.visit_json_as_jsonb(right)?;

        if not {
            self.write(")")?;
        }

        Ok(())
    }

    fn visit_json_type_equals(&mut self, left: Expression<'a>, right: JsonType<'a>, not: bool) -> visitor::Result {
        self.visit_json_type(left)?;
        self.write(if not { " != " } else { " = " })?;

        match right {
            JsonType::Array => self.write("'array'"),
            JsonType::Boolean => self.write("'boolean'"),
            JsonType::Number => self.write("'number'"),
            JsonType::Object => self.write("'object'"),
            JsonType::String => self.write("'string'"),
            JsonType::Null => self.write("'null'"),
            JsonType::ColumnRef(column) => self.visit_json_type((*column).into()),
        }
    }

    fn visit_json_unquote(&mut self, json_unquote: JsonUnquote<'a>) -> visitor::Result {
        // `JSON_VALUE` only returns JSON scalars. `#>> ARRAY[]::text[]` returns the
        // textual representation for objects and arrays as well, while unquoting a
        // JSON string, which is the semantics expected by `JsonUnquote`.
        self.write("(")?;
        self.visit_json_as_jsonb(*json_unquote.expr)?;
        self.write("#>>ARRAY[]::text[])")
    }

    fn visit_json_array_agg(&mut self, array_agg: JsonArrayAgg<'a>) -> visitor::Result {
        self.write("JSON_ARRAYAGG(")?;
        self.visit_expression(*array_agg.expr)?;
        self.write(" RETURNING JSONB)")
    }

    fn visit_json_build_object(&mut self, build_obj: JsonBuildObject<'a>) -> visitor::Result {
        self.write("JSON_OBJECT")?;
        self.surround_with("(", " RETURNING JSONB)", |visitor| {
            let len = build_obj.exprs.len();

            for (index, (name, expression)) in build_obj.exprs.into_iter().enumerate() {
                visitor.visit_raw_value(Value::text(name))?;
                visitor.write(" VALUE ")?;
                visitor.visit_expression(expression)?;

                if index < len - 1 {
                    visitor.write(", ")?;
                }
            }

            Ok(())
        })
    }

    fn visit_stringify(&mut self, stringify: Stringify<'a>) -> visitor::Result {
        self.write("CAST(")?;
        self.visit_expression(*stringify.expression)?;
        self.write(" AS VARCHAR2(4000))")
    }

    fn visit_text_search(&mut self, text_search: TextSearch<'a>) -> visitor::Result {
        let len = text_search.exprs.len();
        self.surround_with("to_tsvector(concat_ws(' ', ", "))", |visitor| {
            for (index, expression) in text_search.exprs.into_iter().enumerate() {
                visitor.visit_expression(expression)?;

                if index < len - 1 {
                    visitor.write(",")?;
                }
            }

            Ok(())
        })
    }

    fn visit_matches(&mut self, left: Expression<'a>, right: Expression<'a>, not: bool) -> visitor::Result {
        if not {
            self.write("(NOT ")?;
        }

        self.visit_expression(left)?;
        self.write(" @@ ")?;
        self.surround_with("to_tsquery(", ")", |visitor| visitor.visit_expression(right))?;

        if not {
            self.write(")")?;
        }

        Ok(())
    }

    fn visit_text_search_relevance(&mut self, relevance: TextSearchRelevance<'a>) -> visitor::Result {
        let len = relevance.exprs.len();
        let exprs = relevance.exprs;
        let query = relevance.query;

        self.write("ts_rank(")?;
        self.surround_with("to_tsvector(concat_ws(' ', ", "))", |visitor| {
            for (index, expression) in exprs.into_iter().enumerate() {
                visitor.visit_expression(expression)?;

                if index < len - 1 {
                    visitor.write(",")?;
                }
            }

            Ok(())
        })?;
        self.write(", ")?;
        self.surround_with("to_tsquery(", ")", |visitor| visitor.visit_expression(query))?;
        self.write(")")
    }
}

#[cfg(test)]
mod tests {
    use super::KingbaseOracle;
    use crate::{ast::*, visitor::Visitor};

    #[test]
    fn renders_oracle_insert_returning_and_default_values() {
        let insert = Insert::single_into("users").value("name", "金仓");
        let (sql, params) = KingbaseOracle::build(Insert::from(insert).returning(vec!["id"])).unwrap();

        assert_eq!("INSERT INTO \"users\" (\"name\") VALUES ($1) RETURNING \"id\"", sql);
        assert_eq!(vec![Value::text("金仓")], params);

        let (sql, params) = KingbaseOracle::build(Insert::single_into("users")).unwrap();
        assert_eq!("INSERT INTO \"users\" DEFAULT VALUES", sql);
        assert!(params.is_empty());
    }

    #[test]
    fn renders_oracle_offset_fetch_pagination() {
        let query = Select::from_table("users")
            .column("id")
            .order_by("id")
            .limit(10)
            .offset(2);
        let (sql, params) = KingbaseOracle::build(query).unwrap();

        assert_eq!(
            "SELECT \"id\" FROM \"users\" ORDER BY \"id\" OFFSET $1 ROWS FETCH NEXT $2 ROWS ONLY",
            sql
        );
        assert_eq!(vec![Value::int64(2), Value::int64(10)], params);
    }

    #[test]
    fn renders_multi_expression_concat_with_oracle_operator() {
        let query = Select::default().value(
            concat::<'_, Expression<'_>>(vec![
                Column::from("first_name").into(),
                " ".into(),
                Column::from("last_name").into(),
            ])
            .alias("full_name"),
        );
        let (sql, params) = KingbaseOracle::build(query).unwrap();

        assert_eq!("SELECT (\"first_name\" || $1 || \"last_name\") AS \"full_name\"", sql);
        assert_eq!(vec![Value::text(" ")], params);
    }

    #[test]
    fn renders_oracle_merge_upserts() {
        let id = Column::from("id").table("users");
        let table = Table::from("users").add_unique_index(id.clone());
        let insert: Insert<'_> = Insert::single_into(table).value(id, 1).value("name", "created").into();
        let (sql, params) = KingbaseOracle::build(insert.on_conflict(OnConflict::DoNothing)).unwrap();

        assert_eq!(
            "MERGE INTO \"users\" USING (SELECT $1 AS \"id\", $2 AS \"name\") \"dual\" (\"id\",\"name\") ON (\"dual\".\"id\" = \"users\".\"id\") WHEN NOT MATCHED THEN INSERT  (\"id\",\"name\") VALUES (\"dual\".\"id\",\"dual\".\"name\")",
            sql
        );
        assert_eq!(vec![Value::int32(1), Value::text("created")], params);

        let update = Update::table("users")
            .set("name", "updated")
            .so_that(("users", "id").equals(1));
        let insert: Insert<'_> = Insert::single_into("users")
            .value("id", 1)
            .value("name", "created")
            .into();
        let (sql, params) =
            KingbaseOracle::build(insert.on_conflict(OnConflict::Update(update, vec!["id".into()]))).unwrap();

        assert_eq!(
            "MERGE INTO \"users\" USING (SELECT $1 AS \"id\", $2 AS \"name\") source (\"id\",\"name\") ON (\"source\".\"id\" = \"users\".\"id\") WHEN MATCHED THEN UPDATE SET \"name\" = $3 WHERE \"users\".\"id\" = $4 WHEN NOT MATCHED THEN INSERT (\"id\",\"name\") VALUES (\"source\".\"id\",\"source\".\"name\")",
            sql
        );
        assert_eq!(
            vec![
                Value::int32(1),
                Value::text("created"),
                Value::text("updated"),
                Value::int32(1)
            ],
            params
        );
    }

    #[test]
    fn rejects_oracle_merge_upserts_that_change_conflict_constraints() {
        let update = Update::table("users").set("id", 2);
        let insert: Insert<'_> = Insert::single_into("users").value("id", 1).into();
        let error =
            KingbaseOracle::build(insert.on_conflict(OnConflict::Update(update, vec!["id".into()]))).unwrap_err();

        assert!(error.to_string().contains("updating a conflict constraint"));
    }

    #[test]
    fn refuses_sql_array_literals() {
        let query = Select::default().value(Value::array(vec![Value::int32(1)]).raw());
        let error = KingbaseOracle::build(query).unwrap_err();

        assert!(error.to_string().contains("array literal"));
    }

    #[test]
    fn refuses_sql_array_bind_parameters() {
        let values = Value::enum_array_with_name(
            vec![EnumVariant::new("A"), EnumVariant::new("B")],
            EnumName::new("Alphabet", Option::<String>::None),
        );
        let error = KingbaseOracle::build(Select::default().value(values)).unwrap_err();

        assert!(error.to_string().contains("array bind parameter"));
    }

    #[test]
    fn renders_delete_returning() {
        let query = Delete::from_table("users")
            .so_that("id".equals(1))
            .returning(vec!["id"]);
        let (sql, params) = KingbaseOracle::build(query).unwrap();

        assert_eq!("DELETE FROM \"users\" WHERE \"id\" = $1 RETURNING \"id\"", sql);
        assert_eq!(vec![Value::int32(1)], params);
    }

    #[test]
    fn renders_special_float_literals_as_kingbase_constants() {
        let query = Select::default()
            .value(Value::float(f32::NAN).raw())
            .value(Value::double(f64::INFINITY).raw())
            .value(Value::double(f64::NEG_INFINITY).raw());
        let (sql, params) = KingbaseOracle::build(query).unwrap();

        assert_eq!("SELECT 'NaN', 'Infinity', '-Infinity'", sql);
        assert!(params.is_empty());
    }

    #[test]
    fn renders_raw_bytes_with_the_kingbase_hex_decoder() {
        let (sql, params) =
            KingbaseOracle::build(Select::default().value(Value::bytes(vec![0xab, 0xcd]).raw())).unwrap();

        assert_eq!("SELECT decode('abcd', 'hex')", sql);
        assert!(params.is_empty());
    }

    #[test]
    fn renders_oracle_json_functions_with_jsonb_inputs() {
        let extract: Expression<'_> =
            json_extract(Column::from("payload"), JsonPath::array(["items", "0"]), false).into();
        let query = Select::from_table("documents")
            .value(extract)
            .so_that("payload".json_array_contains(serde_json::json!([1, 2])));
        let (sql, params) = KingbaseOracle::build(query).unwrap();

        assert_eq!(
            "SELECT CAST(JSON_QUERY(CAST(\"payload\" AS JSONB), $1) AS JSONB) FROM \"documents\" WHERE CAST(\"payload\" AS JSONB) @> CAST($2 AS JSONB)",
            sql
        );
        assert_eq!(
            vec![Value::text("$.\"items\"[0]"), Value::json(serde_json::json!([1, 2]))],
            params
        );

        let query =
            Select::from_table("documents").so_that(Column::from("payload").equals(serde_json::json!({ "id": 1 })));
        let (sql, params) = KingbaseOracle::build(query).unwrap();

        assert_eq!(
            "SELECT \"documents\".* FROM \"documents\" WHERE CAST(\"payload\" AS JSONB) = CAST($1 AS JSONB)",
            sql
        );
        assert_eq!(vec![Value::json(serde_json::json!({ "id": 1 }))], params);

        let query = Select::from_table("documents").value(json_unquote(Column::from("payload")));
        let (sql, params) = KingbaseOracle::build(query).unwrap();

        assert_eq!(
            "SELECT (CAST(\"payload\" AS JSONB)#>>ARRAY[]::text[]) FROM \"documents\"",
            sql
        );
        assert!(params.is_empty());

        let query = Select::from_table("documents")
            .value(json_array_agg(Column::from("payload")))
            .value(json_build_object(vec![(
                "payload".into(),
                Column::from("payload").into(),
            )]))
            .so_that("payload".json_type_equals(JsonType::Array));
        let (sql, params) = KingbaseOracle::build(query).unwrap();

        assert_eq!(
            "SELECT JSON_ARRAYAGG(\"payload\" RETURNING JSONB), JSON_OBJECT('payload' VALUE \"payload\" RETURNING JSONB) FROM \"documents\" WHERE JSONB_TYPEOF(CAST(\"payload\" AS JSONB)) = 'array'",
            sql
        );
        assert!(params.is_empty());
    }

    #[test]
    fn renders_oracle_full_text_search() {
        let search: Expression = text_search(&[Column::from("title"), Column::from("body")]).into();
        let query = Select::from_table("documents").so_that(search.matches("prisma & compiler"));
        let (sql, params) = KingbaseOracle::build(query).unwrap();

        assert_eq!(
            "SELECT \"documents\".* FROM \"documents\" WHERE to_tsvector(concat_ws(' ', \"title\",\"body\")) @@ to_tsquery($1)",
            sql
        );
        assert_eq!(vec![Value::text("prisma & compiler")], params);

        let relevance: Expression = text_search_relevance(&[Column::from("title")], "prisma").into();
        let (sql, params) = KingbaseOracle::build(Select::from_table("documents").value(relevance)).unwrap();

        assert_eq!(
            "SELECT ts_rank(to_tsvector(concat_ws(' ', \"title\")), to_tsquery($1)) FROM \"documents\"",
            sql
        );
        assert_eq!(vec![Value::text("prisma")], params);
    }
}
