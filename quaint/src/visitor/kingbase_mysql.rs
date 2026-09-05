use crate::{
    ast::{NativeColumnType, Query, Value, ValueType},
    error::{Error, ErrorKind},
    visitor::{Mysql, Visitor},
};
use query_template::{Fragment, PlaceholderFormat, QueryTemplate};
use std::borrow::Cow;

/// A MySQL-compatible SQL visitor for KingbaseES.
///
/// It preserves the MySQL visitor's SQL semantics, adding only the metadata
/// and casts required by Kingbase's `jsonpath` function arguments.
pub struct KingbaseMysql;

impl KingbaseMysql {
    pub fn build_template<'a, Q>(query: Q) -> crate::Result<QueryTemplate<Value<'a>>>
    where
        Q: Into<Query<'a>>,
    {
        let template = Mysql::build_template(query)?;
        let QueryTemplate {
            fragments,
            mut parameters,
            placeholder_format,
        } = template;
        let (sql, parameter_fragments) = template_sql(fragments);

        let sql = rewrite_full_text_search(sql, &mut parameters)?;
        let sql = cast_sum_parameters(sql, &parameters);
        promote_json_numeric_comparison_params(&sql, &mut parameters);
        let sql = cast_json_comparison_expressions(sql);

        for index in json_extract_path_parameter_indexes(&sql) {
            if let Some(param) = parameters.get_mut(index) {
                param.native_column_type = Some(NativeColumnType {
                    name: Cow::Borrowed("JSONPATH"),
                    length: None,
                });
            }
        }

        let sql = cast_dynamic_jsonpaths(sql);
        let sql = qualify_json_contains(sql);
        let sql = unquote_json_values(sql);
        let sql = native_uuid_as_text(sql);

        rebuild_template(sql, parameter_fragments, parameters, placeholder_format)
    }

    pub fn build<'a, Q>(query: Q) -> crate::Result<(String, Vec<Value<'a>>)>
    where
        Q: Into<Query<'a>>,
    {
        let template = Self::build_template(query)?;
        let sql = template.to_sql().map_err(|_| {
            Error::builder(ErrorKind::conversion(
                "Kingbase MySQL query contains dynamic parameter fragments",
            ))
            .build()
        })?;
        let params = template.parameters;
        Ok((sql, params))
    }
}

/// Rewrites the MySQL visitor's `MATCH (...) AGAINST (... IN BOOLEAN MODE)`
/// output to Kingbase's PostgreSQL-compatible full-text syntax.
///
/// MySQL Boolean Mode is not the same language as PostgreSQL's
/// `websearch_to_tsquery`: bare MySQL terms are optional (OR), while bare
/// PostgreSQL terms are required (AND). Convert the supported Boolean Mode
/// subset to a `tsquery` before binding it instead of silently changing which
/// rows match. We keep the `simple` configuration fixed because it makes the
/// `to_tsvector` expression immutable, allowing the same expression to back a
/// GIN index.
fn rewrite_full_text_search(sql: String, parameters: &mut [Value<'_>]) -> crate::Result<String> {
    const MATCH: &str = "MATCH";
    const AGAINST: &str = "AGAINST";
    const BOOLEAN_MODE: &str = "IN BOOLEAN MODE";

    let mut output = String::with_capacity(sql.len());
    let mut cursor = 0;
    let mut search_start = 0;

    while let Some(offset) = sql[search_start..].find(MATCH) {
        let match_start = search_start + offset;
        let match_open = skip_whitespace(&sql, match_start + MATCH.len());

        if sql.as_bytes().get(match_open) != Some(&b'(') {
            search_start = match_start + MATCH.len();
            continue;
        }

        let Some(columns_end) = matching_parenthesis(&sql, match_open) else {
            break;
        };

        let against_start = skip_whitespace(&sql, columns_end + 1);
        if !sql[against_start..].starts_with(AGAINST) {
            search_start = columns_end + 1;
            continue;
        }

        let against_open = skip_whitespace(&sql, against_start + AGAINST.len());
        if sql.as_bytes().get(against_open) != Some(&b'(') {
            search_start = against_start + AGAINST.len();
            continue;
        }

        let Some(against_end) = matching_parenthesis(&sql, against_open) else {
            break;
        };

        let Some(mode_start) = sql[against_open + 1..against_end]
            .rfind(BOOLEAN_MODE)
            .map(|offset| against_open + 1 + offset)
        else {
            search_start = against_end + 1;
            continue;
        };

        if !sql[mode_start + BOOLEAN_MODE.len()..against_end].trim().is_empty() {
            search_start = against_end + 1;
            continue;
        }

        let document = fulltext_document(&sql[match_open + 1..columns_end]);
        let query = sql[against_open + 1..mode_start].trim();
        let query_parameter_index = fulltext_query_parameter_index(query, &sql)?;
        let tsquery = mysql_boolean_mode_to_tsquery(fulltext_query_parameter(parameters, query_parameter_index)?)?;
        set_fulltext_query_parameter(parameters, query_parameter_index, tsquery)?;

        output.push_str(&sql[cursor..match_start]);

        if is_fulltext_relevance_expression(&sql, match_start, against_end) {
            output.push_str("ts_rank(to_tsvector('simple', ");
            output.push_str(&document);
            output.push_str("), to_tsquery('simple', ");
            output.push_str(query);
            output.push_str("))");
        } else {
            output.push_str("to_tsvector('simple', ");
            output.push_str(&document);
            output.push_str(") @@ to_tsquery('simple', ");
            output.push_str(query);
            output.push(')');
        }

        cursor = against_end + 1;
        search_start = cursor;
    }

    output.push_str(&sql[cursor..]);
    Ok(output)
}

fn fulltext_query_parameter_index(query: &str, sql: &str) -> crate::Result<usize> {
    if query != "?" {
        return Err(fulltext_conversion_error(
            "Kingbase MySQL full-text search requires a parameterized search string",
        ));
    }

    let query_position = query.as_ptr() as usize - sql.as_ptr() as usize;
    parameter_positions(sql)
        .binary_search(&query_position)
        .map_err(|_| fulltext_conversion_error("Kingbase MySQL full-text search parameter could not be located"))
}

fn fulltext_query_parameter<'a>(parameters: &'a [Value<'_>], index: usize) -> crate::Result<&'a str> {
    match parameters.get(index).map(|parameter| &parameter.typed) {
        Some(ValueType::Text(Some(query))) => Ok(query),
        _ => Err(fulltext_conversion_error(
            "Kingbase MySQL full-text search requires a non-null string search value",
        )),
    }
}

fn set_fulltext_query_parameter(
    parameters: &mut [Value<'_>],
    index: usize,
    tsquery: Option<String>,
) -> crate::Result<()> {
    let parameter = parameters
        .get_mut(index)
        .ok_or_else(|| fulltext_conversion_error("Kingbase MySQL full-text search parameter could not be updated"))?;

    let ValueType::Text(Some(query)) = &mut parameter.typed else {
        return Err(fulltext_conversion_error(
            "Kingbase MySQL full-text search requires a non-null string search value",
        ));
    };

    // MySQL returns no rows for a Boolean query containing only prohibited
    // terms. A contradiction is a valid tsquery and preserves that behavior.
    *query = Cow::Owned(tsquery.unwrap_or_else(|| "'__prisma_never_match__' & !'__prisma_never_match__'".to_owned()));
    Ok(())
}

fn fulltext_conversion_error(message: impl Into<String>) -> Error {
    let message = message.into();
    let mut builder = Error::builder(ErrorKind::conversion(message.clone()));
    builder.set_original_message(message);
    builder.build()
}

#[derive(Clone, Copy)]
enum BooleanModeModifier {
    Optional,
    Required,
    Prohibited,
}

struct BooleanModeTerm {
    modifier: BooleanModeModifier,
    expression: String,
}

/// Translate the Boolean Mode subset whose matching semantics can be expressed
/// with a PostgreSQL `tsquery`. Relevance-only modifiers (`>`, `<`, `~`) and
/// proximity searches cannot be represented faithfully, so reject them rather
/// than returning a different result set.
fn mysql_boolean_mode_to_tsquery(query: &str) -> crate::Result<Option<String>> {
    let mut parser = MySqlBooleanModeParser::new(query);
    let terms = parser.parse_terms(false)?;

    if parser.next_char().is_some() {
        return Err(fulltext_conversion_error(
            "Kingbase MySQL full-text search contains an unexpected closing parenthesis",
        ));
    }

    Ok(compose_boolean_mode_terms(terms))
}

struct MySqlBooleanModeParser<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
}

impl<'a> MySqlBooleanModeParser<'a> {
    fn new(query: &'a str) -> Self {
        Self {
            chars: query.chars().peekable(),
        }
    }

    fn next_char(&mut self) -> Option<char> {
        self.chars.next()
    }

    fn peek_char(&mut self) -> Option<char> {
        self.chars.peek().copied()
    }

    fn skip_whitespace(&mut self) {
        while self.peek_char().is_some_and(char::is_whitespace) {
            self.next_char();
        }
    }

    fn parse_terms(&mut self, stop_at_closing_parenthesis: bool) -> crate::Result<Vec<BooleanModeTerm>> {
        let mut terms = Vec::new();

        loop {
            self.skip_whitespace();

            match self.peek_char() {
                None => {
                    if stop_at_closing_parenthesis {
                        return Err(fulltext_conversion_error(
                            "Kingbase MySQL full-text search contains an unclosed parenthesis",
                        ));
                    }
                    return Ok(terms);
                }
                Some(')') if stop_at_closing_parenthesis => {
                    self.next_char();
                    return Ok(terms);
                }
                Some(')') => return Ok(terms),
                _ => terms.push(self.parse_term()?),
            }
        }
    }

    fn parse_term(&mut self) -> crate::Result<BooleanModeTerm> {
        let modifier = match self.peek_char() {
            Some('+') => {
                self.next_char();
                BooleanModeModifier::Required
            }
            Some('-') => {
                self.next_char();
                BooleanModeModifier::Prohibited
            }
            Some('~' | '>' | '<' | '@') => {
                return Err(fulltext_conversion_error(
                    "Kingbase MySQL full-text search does not support MySQL relevance or proximity modifiers",
                ));
            }
            _ => BooleanModeModifier::Optional,
        };

        self.skip_whitespace();
        let expression = match self.next_char() {
            Some('(') => {
                let terms = self.parse_terms(true)?;
                compose_boolean_mode_terms(terms).ok_or_else(|| {
                    fulltext_conversion_error(
                        "Kingbase MySQL full-text search group must contain a positive search term",
                    )
                })?
            }
            Some('"') => self.parse_phrase()?,
            Some(character) => self.parse_word(character)?,
            None => {
                return Err(fulltext_conversion_error(
                    "Kingbase MySQL full-text search modifier is missing a search term",
                ));
            }
        };

        Ok(BooleanModeTerm { modifier, expression })
    }

    fn parse_phrase(&mut self) -> crate::Result<String> {
        let mut words = Vec::new();
        let mut word = String::new();

        loop {
            match self.next_char() {
                Some('"') => {
                    if !word.is_empty() {
                        words.push(tsquery_lexeme(&word, false)?);
                    }
                    break;
                }
                Some(character) if character.is_whitespace() => {
                    if !word.is_empty() {
                        words.push(tsquery_lexeme(&word, false)?);
                        word.clear();
                    }
                }
                Some(character) => word.push(character),
                None => {
                    return Err(fulltext_conversion_error(
                        "Kingbase MySQL full-text search contains an unterminated phrase",
                    ));
                }
            }
        }

        if words.is_empty() {
            return Err(fulltext_conversion_error(
                "Kingbase MySQL full-text search phrase must contain a word",
            ));
        }

        if matches!(self.peek_char(), Some('@')) {
            return Err(fulltext_conversion_error(
                "Kingbase MySQL full-text search does not support MySQL proximity modifiers",
            ));
        }

        Ok(format!("({})", words.join(" <-> ")))
    }

    fn parse_word(&mut self, first: char) -> crate::Result<String> {
        let mut word = String::from(first);
        while self
            .peek_char()
            .is_some_and(|character| character.is_alphanumeric() || character == '_')
        {
            word.push(self.next_char().unwrap());
        }

        let prefix = matches!(self.peek_char(), Some('*'));
        if prefix {
            self.next_char();
        }

        if self
            .peek_char()
            .is_some_and(|character| !character.is_whitespace() && character != '(' && character != ')')
        {
            return Err(fulltext_conversion_error(
                "Kingbase MySQL full-text search contains an unsupported Boolean Mode token",
            ));
        }

        tsquery_lexeme(&word, prefix)
    }
}

fn tsquery_lexeme(word: &str, prefix: bool) -> crate::Result<String> {
    if word.is_empty()
        || !word
            .chars()
            .all(|character| character.is_alphanumeric() || character == '_')
    {
        return Err(fulltext_conversion_error(
            "Kingbase MySQL full-text search supports only word, phrase, prefix, required, prohibited, and grouped terms",
        ));
    }

    let mut lexeme = format!("'{}'", word.replace('\\', "\\\\").replace('\'', "''"));
    if prefix {
        lexeme.push_str(":*");
    }
    Ok(lexeme)
}

fn compose_boolean_mode_terms(terms: Vec<BooleanModeTerm>) -> Option<String> {
    let mut required = Vec::new();
    let mut optional = Vec::new();
    let mut prohibited = Vec::new();

    for term in terms {
        match term.modifier {
            BooleanModeModifier::Required => required.push(term.expression),
            BooleanModeModifier::Optional => optional.push(term.expression),
            BooleanModeModifier::Prohibited => prohibited.push(term.expression),
        }
    }

    let required_expression = join_tsquery_terms(&required, " & ");
    let optional_expression = join_tsquery_terms(&optional, " | ");

    let mut expression = match (required_expression.as_deref(), optional_expression.as_deref()) {
        (Some(required), Some(optional)) => format!("({required}) & (({required}) | ({optional}))"),
        (Some(required), None) => required.to_owned(),
        (None, Some(optional)) => optional.to_owned(),
        (None, None) => return None,
    };

    if let Some(prohibited) = join_tsquery_terms(&prohibited, " | ") {
        expression = format!("({expression}) & !({prohibited})");
    }

    Some(expression)
}

fn join_tsquery_terms(terms: &[String], separator: &str) -> Option<String> {
    (!terms.is_empty()).then(|| {
        terms
            .iter()
            .map(|term| format!("({term})"))
            .collect::<Vec<_>>()
            .join(separator)
    })
}

fn fulltext_document(columns: &str) -> String {
    columns
        .split(',')
        .map(|column| format!("COALESCE({}, '')", column.trim()))
        .reduce(|document, column| format!("textcat({document}, textcat(' ', {column}))"))
        .expect("a MySQL MATCH expression must contain at least one column")
}

fn is_fulltext_relevance_expression(sql: &str, match_start: usize, against_end: usize) -> bool {
    let before_match = &sql[..match_start];
    let last_order_by = before_match.rfind("ORDER BY ");
    let last_where = before_match.rfind(" WHERE ");

    if last_order_by.is_some_and(|order_by| last_where.is_none_or(|where_clause| order_by > where_clause)) {
        return true;
    }

    // `text_search_relevance()` may be selected directly, before the first
    // FROM clause, rather than only used as an ORDER BY expression.
    if before_match.rfind(" FROM ").is_none() {
        return true;
    }

    // A relevance expression can also be compared in a WHERE clause. A plain
    // MATCH expression there is the Boolean filter form instead.
    matches!(
        sql[against_end + 1..].trim_start().as_bytes().first(),
        Some(b'>' | b'<' | b'=')
    )
}

fn template_sql(fragments: Vec<Fragment>) -> (String, Vec<Fragment>) {
    let mut sql = String::new();
    let mut parameter_fragments = Vec::new();

    for fragment in fragments {
        match fragment {
            Fragment::StringChunk { chunk } => sql.push_str(&chunk),
            fragment => {
                sql.push('?');
                parameter_fragments.push(fragment);
            }
        }
    }

    (sql, parameter_fragments)
}

fn rebuild_template<'a>(
    sql: String,
    parameter_fragments: Vec<Fragment>,
    parameters: Vec<Value<'a>>,
    placeholder_format: PlaceholderFormat,
) -> crate::Result<QueryTemplate<Value<'a>>> {
    let positions = parameter_positions(&sql);

    if positions.len() != parameter_fragments.len() {
        let message = format!(
            "Kingbase MySQL SQL rewrite changed the parameter count from {} to {}",
            parameter_fragments.len(),
            positions.len()
        );
        let mut builder = Error::builder(ErrorKind::conversion(message.clone()));
        builder.set_original_message(message);
        return Err(builder.build());
    }

    let mut fragments = Vec::with_capacity(parameter_fragments.len() * 2 + 1);
    let mut cursor = 0;

    for (position, fragment) in positions.into_iter().zip(parameter_fragments) {
        if cursor < position {
            fragments.push(Fragment::StringChunk {
                chunk: sql[cursor..position].to_owned(),
            });
        }
        fragments.push(fragment);
        cursor = position + 1;
    }

    if cursor < sql.len() {
        fragments.push(Fragment::StringChunk {
            chunk: sql[cursor..].to_owned(),
        });
    }

    Ok(QueryTemplate {
        fragments,
        parameters,
        placeholder_format,
    })
}

fn cast_sum_parameters(sql: String, params: &[Value<'_>]) -> String {
    let parameter_positions = parameter_positions(&sql);
    let mut output = String::with_capacity(sql.len());
    let mut cursor = 0;
    let mut search_start = 0;

    while let Some(offset) = sql[search_start..].find("SUM(") {
        let function_start = search_start + offset;
        let open_paren = function_start + "SUM".len();
        let Some(function_end) = matching_parenthesis(&sql, open_paren) else {
            break;
        };
        let argument_start = skip_whitespace(&sql, open_paren + 1);
        let argument_end = previous_non_whitespace(sql.as_bytes(), function_end);

        let Some(argument_end) = argument_end else {
            search_start = function_end + 1;
            continue;
        };

        if argument_start != argument_end || sql.as_bytes().get(argument_start) != Some(&b'?') {
            search_start = function_end + 1;
            continue;
        }

        let Ok(parameter_index) = parameter_positions.binary_search(&argument_start) else {
            search_start = function_end + 1;
            continue;
        };
        let Some(param) = params.get(parameter_index) else {
            search_start = function_end + 1;
            continue;
        };
        let Some(type_name) = sum_parameter_type(param) else {
            search_start = function_end + 1;
            continue;
        };

        output.push_str(&sql[cursor..argument_start]);
        output.push_str("CAST(? AS ");
        output.push_str(type_name);
        output.push(')');
        cursor = argument_end + 1;
        search_start = function_end + 1;
    }

    output.push_str(&sql[cursor..]);
    output
}

fn sum_parameter_type(param: &Value<'_>) -> Option<&'static str> {
    match param.typed {
        // MySQL SUM() promotes exact integer inputs to DECIMAL. Kingbase's
        // SUM(INT4/INT8) instead returns INT8/NUMERIC respectively, so cast
        // integer parameters to NUMERIC before aggregation.
        ValueType::Int32(_) | ValueType::Int64(_) => Some("NUMERIC"),
        ValueType::Float(_) => Some("FLOAT4"),
        ValueType::Double(_) => Some("FLOAT8"),
        ValueType::Numeric(_) => Some("NUMERIC"),
        _ => None,
    }
}

fn promote_json_numeric_comparison_params(sql: &str, params: &mut [Value<'_>]) {
    for index in json_numeric_comparison_parameter_indexes(sql) {
        let Some(param) = params.get_mut(index) else {
            continue;
        };

        let json = match &param.typed {
            ValueType::Int32(Some(value)) => Some(serde_json::Value::from(*value)),
            ValueType::Int64(Some(value)) => Some(serde_json::Value::from(*value)),
            ValueType::Float(Some(value)) => serde_json::Number::from_f64(f64::from(*value)).map(Into::into),
            ValueType::Double(Some(value)) => serde_json::Number::from_f64(*value).map(Into::into),
            ValueType::Text(Some(value)) => Some(serde_json::Value::String(value.to_string())),
            _ => None,
        };

        if let Some(json) = json {
            param.typed = ValueType::Json(Some(json));
        }
    }
}

fn cast_json_comparison_expressions(sql: String) -> String {
    let mut output = String::with_capacity(sql.len());
    let mut cursor = 0;
    let mut search_start = 0;

    while let Some(offset) = sql[search_start..].find("JSON_EXTRACT(") {
        let function_start = search_start + offset;
        let open_paren = function_start + "JSON_EXTRACT".len();
        let Some(function_end) = matching_parenthesis(&sql, open_paren) else {
            break;
        };

        let is_comparison_operand = numeric_comparison_parameter_after(&sql, function_end + 1).is_some()
            || numeric_comparison_parameter_before(&sql, function_start).is_some();

        if is_comparison_operand {
            output.push_str(&sql[cursor..function_start]);
            output.push_str("CAST(");
            output.push_str(&sql[function_start..=function_end]);
            output.push_str(" AS jsonb)");
            cursor = function_end + 1;
        }

        search_start = function_end + 1;
    }

    output.push_str(&sql[cursor..]);
    output
}

fn json_numeric_comparison_parameter_indexes(sql: &str) -> Vec<usize> {
    let parameter_positions = parameter_positions(sql);
    let mut indexes = Vec::new();
    let mut search_start = 0;

    while let Some(offset) = sql[search_start..].find("JSON_EXTRACT(") {
        let function_start = search_start + offset;
        let open_paren = function_start + "JSON_EXTRACT".len();
        let Some(function_end) = matching_parenthesis(sql, open_paren) else {
            break;
        };

        if let Some(parameter_position) = numeric_comparison_parameter_after(sql, function_end + 1) {
            if let Ok(index) = parameter_positions.binary_search(&parameter_position) {
                indexes.push(index);
            }
        }

        if let Some(parameter_position) = numeric_comparison_parameter_before(sql, function_start) {
            if let Ok(index) = parameter_positions.binary_search(&parameter_position) {
                indexes.push(index);
            }
        }

        search_start = function_end + 1;
    }

    indexes.sort_unstable();
    indexes.dedup();
    indexes
}

fn parameter_positions(sql: &str) -> Vec<usize> {
    let bytes = sql.as_bytes();
    let mut positions = Vec::new();
    let mut quoted = None;
    let mut index = 0;

    while index < bytes.len() {
        let byte = bytes[index];

        if let Some(quote) = quoted {
            if byte == b'\\' {
                index += 2;
                continue;
            }

            if byte == quote {
                quoted = None;
            }

            index += 1;
            continue;
        }

        match byte {
            b'\'' | b'`' => quoted = Some(byte),
            b'?' => positions.push(index),
            _ => {}
        }

        index += 1;
    }

    positions
}

fn numeric_comparison_parameter_after(sql: &str, start: usize) -> Option<usize> {
    let bytes = sql.as_bytes();
    let mut index = skip_whitespace(sql, start);

    match bytes.get(index)? {
        b'>' | b'<' => {
            index += 1;
            if bytes.get(index) == Some(&b'=') {
                index += 1;
            }

            let parameter = skip_whitespace(sql, index);
            (bytes.get(parameter) == Some(&b'?')).then_some(parameter)
        }
        _ => None,
    }
}

fn numeric_comparison_parameter_before(sql: &str, start: usize) -> Option<usize> {
    let bytes = sql.as_bytes();
    let operator_end = previous_non_whitespace(bytes, start)?;
    let parameter_boundary = match bytes[operator_end] {
        b'>' | b'<' => operator_end,
        b'=' if previous_non_whitespace(bytes, operator_end)
            .is_some_and(|index| matches!(bytes[index], b'>' | b'<')) =>
        {
            previous_non_whitespace(bytes, operator_end)?
        }
        _ => return None,
    };

    let parameter = previous_non_whitespace(bytes, parameter_boundary)?;
    (bytes[parameter] == b'?').then_some(parameter)
}

fn previous_non_whitespace(bytes: &[u8], start: usize) -> Option<usize> {
    (0..start).rev().find(|index| !bytes[*index].is_ascii_whitespace())
}

fn json_extract_path_parameter_indexes(sql: &str) -> Vec<usize> {
    #[derive(Clone, Copy)]
    struct Context {
        is_json_extract: bool,
        argument: usize,
    }

    let bytes = sql.as_bytes();
    let mut contexts = Vec::new();
    let mut indexes = Vec::new();
    let mut parameter_index = 0;
    let mut pending_json_extract = false;
    let mut quoted = None;
    let mut index = 0;

    while index < bytes.len() {
        let byte = bytes[index];

        if let Some(quote) = quoted {
            if byte == b'\\' {
                index += 2;
                continue;
            }

            if byte == quote {
                quoted = None;
            }
            index += 1;
            continue;
        }

        match byte {
            b'\'' | b'`' => {
                quoted = Some(byte);
                pending_json_extract = false;
            }
            b'(' => {
                contexts.push(Context {
                    is_json_extract: pending_json_extract,
                    argument: 0,
                });
                pending_json_extract = false;
            }
            b')' => {
                contexts.pop();
                pending_json_extract = false;
            }
            b',' => {
                if let Some(context) = contexts.last_mut()
                    && context.is_json_extract
                {
                    context.argument += 1;
                }
                pending_json_extract = false;
            }
            b'?' => {
                if contexts
                    .last()
                    .is_some_and(|context| context.is_json_extract && context.argument == 1)
                {
                    indexes.push(parameter_index);
                }
                parameter_index += 1;
                pending_json_extract = false;
            }
            byte if byte.is_ascii_alphabetic() || byte == b'_' => {
                let identifier_start = index;
                index += 1;

                while index < bytes.len() && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_') {
                    index += 1;
                }

                pending_json_extract = sql[identifier_start..index].eq_ignore_ascii_case("JSON_EXTRACT");
                continue;
            }
            byte if byte.is_ascii_whitespace() => {}
            _ => pending_json_extract = false,
        }

        index += 1;
    }

    indexes
}

fn cast_dynamic_jsonpaths(sql: String) -> String {
    let mut output = String::with_capacity(sql.len());
    let mut cursor = 0;
    let mut search_start = 0;

    while let Some(offset) = sql[search_start..].find("JSON_EXTRACT(") {
        let function_start = search_start + offset;
        let open_paren = function_start + "JSON_EXTRACT".len();
        let Some(function_end) = matching_parenthesis(&sql, open_paren) else {
            break;
        };
        let Some(comma) = first_function_argument_comma(&sql, open_paren, function_end) else {
            search_start = function_end + 1;
            continue;
        };

        let path_start = skip_whitespace(&sql, comma + 1);
        if !sql[path_start..].starts_with("CONCAT(") {
            search_start = function_end + 1;
            continue;
        }

        let path_open_paren = path_start + "CONCAT".len();
        let Some(path_end) = matching_parenthesis(&sql, path_open_paren).map(|end| end + 1) else {
            search_start = function_end + 1;
            continue;
        };

        if path_end > function_end || !sql[path_end..function_end].trim().is_empty() {
            search_start = function_end + 1;
            continue;
        }

        output.push_str(&sql[cursor..path_start]);
        output.push_str("CAST(");
        output.push_str(&sql[path_start..path_end]);
        output.push_str(" AS JSONPATH)");
        cursor = path_end;
        search_start = function_end + 1;
    }

    output.push_str(&sql[cursor..]);
    output
}

fn qualify_json_contains(sql: String) -> String {
    let mut output = String::with_capacity(sql.len());
    let mut cursor = 0;
    let mut search_start = 0;

    while let Some(offset) = sql[search_start..].find("JSON_CONTAINS(") {
        let function_start = search_start + offset;
        let open_paren = function_start + "JSON_CONTAINS".len();
        let Some(function_end) = matching_parenthesis(&sql, open_paren) else {
            break;
        };

        if function_argument_count(&sql, open_paren, function_end) == 2 {
            output.push_str(&sql[cursor..function_start]);
            output.push_str("sys.JSON_CONTAINS");
            output.push_str(&sql[open_paren..function_end]);
            output.push_str(", CAST('$' AS JSONPATH)");
            cursor = function_end;
        }

        search_start = function_end + 1;
    }

    output.push_str(&sql[cursor..]);
    output
}

fn unquote_json_values(sql: String) -> String {
    let mut output = String::with_capacity(sql.len());
    let mut cursor = 0;
    let mut search_start = 0;

    while let Some(offset) = sql[search_start..].find("JSON_UNQUOTE(") {
        let function_start = search_start + offset;
        let open_paren = function_start + "JSON_UNQUOTE".len();
        let Some(function_end) = matching_parenthesis(&sql, open_paren) else {
            break;
        };

        output.push_str(&sql[cursor..function_start]);
        output.push_str("(CAST(");
        output.push_str(&sql[open_paren + 1..function_end]);
        output.push_str(" AS jsonb) ->> CAST('$' AS JSONPATH))");
        cursor = function_end + 1;
        search_start = function_end + 1;
    }

    output.push_str(&sql[cursor..]);
    output
}

fn native_uuid_as_text(sql: String) -> String {
    sql.replace("uuid()", "CAST(sys_guid() AS text)")
}

#[cfg(test)]
mod tests {
    use super::KingbaseMysql;
    use crate::Value;
    use crate::ast::{
        Column, Comparable, Expression, JsonPath, Select, ValueType, json_extract, json_unquote, native_uuid,
        text_search, text_search_relevance,
    };

    #[test]
    fn json_unquote_uses_the_unambiguous_jsonpath_operator() {
        let query = Select::default().value(json_unquote(col!("json")));
        let (sql, params) = KingbaseMysql::build(query).unwrap();

        assert_eq!("SELECT (CAST(`json` AS jsonb) ->> CAST('$' AS JSONPATH))", sql);
        assert!(params.is_empty());
    }

    #[test]
    fn native_uuid_is_a_text_value() {
        let (sql, params) = KingbaseMysql::build(Select::default().value(native_uuid())).unwrap();

        assert_eq!("SELECT CAST(sys_guid() AS text)", sql);
        assert!(params.is_empty());
    }

    #[test]
    fn build_template_preserves_jsonpath_parameter_metadata() {
        let query = Select::default().value(json_extract(Column::from("json"), JsonPath::string("$.a"), false));
        let template = KingbaseMysql::build_template(query).unwrap();

        assert_eq!("SELECT JSON_EXTRACT(`json`, ?)", template.to_sql().unwrap());
        assert_eq!(template.parameters.len(), 1);
        assert!(matches!(
            template.parameters[0]
                .native_column_type
                .as_ref()
                .map(|t| t.name.as_ref()),
            Some("JSONPATH")
        ));
        assert!(matches!(template.parameters[0].typed, ValueType::Text(Some(_))));
    }

    #[test]
    fn full_text_filters_use_kingbase_syntax() {
        let search: Expression = text_search(&[Column::from("name"), Column::from("email")]).into();
        let query = Select::from_table("User").so_that(search.matches("John Smith"));
        let (sql, params) = KingbaseMysql::build(query).unwrap();

        assert_eq!(
            "SELECT `User`.* FROM `User` WHERE to_tsvector('simple', textcat(COALESCE(`name`, ''), textcat(' ', COALESCE(`email`, '')))) @@ to_tsquery('simple', ?)",
            sql
        );
        assert_eq!(params, vec![Value::text("('John') | ('Smith')")]);
    }

    #[test]
    fn full_text_filters_preserve_mysql_boolean_mode_matching() {
        let search: Expression = text_search(&[Column::from("name")]).into();

        for (query, expected_tsquery) in [
            ("John Smith", "('John') | ('Smith')"),
            ("+John Smith", "(('John')) & ((('John')) | (('Smith')))"),
            ("+John -Smith", "(('John')) & !(('Smith'))"),
            ("-Smith", "'__prisma_never_match__' & !'__prisma_never_match__'"),
            ("John*", "('John':*)"),
            ("\"John Smith\"", "(('John' <-> 'Smith'))"),
        ] {
            let (_, params) =
                KingbaseMysql::build(Select::from_table("User").so_that(search.clone().matches(query))).unwrap();

            assert_eq!(params, vec![Value::text(expected_tsquery)]);
        }
    }

    #[test]
    fn full_text_filters_reject_mysql_boolean_mode_operators_without_a_kingbase_equivalent() {
        let search: Expression = text_search(&[Column::from("name")]).into();

        for query in ["~John", ">John", "\"John Smith\"@2"] {
            assert!(KingbaseMysql::build(Select::from_table("User").so_that(search.clone().matches(query))).is_err());
        }
    }

    #[test]
    fn full_text_relevance_uses_kingbase_syntax() {
        let relevance: Expression = text_search_relevance(&[Column::from("name")], "John Smith").into();
        let (sql, params) = KingbaseMysql::build(Select::from_table("User").value(relevance)).unwrap();

        assert_eq!(
            "SELECT ts_rank(to_tsvector('simple', COALESCE(`name`, '')), to_tsquery('simple', ?)) FROM `User`",
            sql
        );
        assert_eq!(params, vec![Value::text("('John') | ('Smith')")]);
    }
}

fn skip_whitespace(sql: &str, mut index: usize) -> usize {
    while sql.as_bytes().get(index).is_some_and(u8::is_ascii_whitespace) {
        index += 1;
    }

    index
}

fn first_function_argument_comma(sql: &str, open_paren: usize, function_end: usize) -> Option<usize> {
    let bytes = sql.as_bytes();
    let mut depth = 1;
    let mut quoted = None;
    let mut index = open_paren + 1;

    while index < function_end {
        let byte = bytes[index];

        if let Some(quote) = quoted {
            if byte == b'\\' {
                index += 2;
                continue;
            }

            if byte == quote {
                quoted = None;
            }

            index += 1;
            continue;
        }

        match byte {
            b'\'' | b'`' => quoted = Some(byte),
            b'(' => depth += 1,
            b')' => depth -= 1,
            b',' if depth == 1 => return Some(index),
            _ => {}
        }

        index += 1;
    }

    None
}

fn function_argument_count(sql: &str, open_paren: usize, function_end: usize) -> usize {
    let bytes = sql.as_bytes();
    let mut count = 1;
    let mut depth = 1;
    let mut quoted = None;
    let mut index = open_paren + 1;

    if sql[index..function_end].trim().is_empty() {
        return 0;
    }

    while index < function_end {
        let byte = bytes[index];

        if let Some(quote) = quoted {
            if byte == b'\\' {
                index += 2;
                continue;
            }

            if byte == quote {
                quoted = None;
            }

            index += 1;
            continue;
        }

        match byte {
            b'\'' | b'`' => quoted = Some(byte),
            b'(' => depth += 1,
            b')' => depth -= 1,
            b',' if depth == 1 => count += 1,
            _ => {}
        }

        index += 1;
    }

    count
}

fn matching_parenthesis(sql: &str, open_paren: usize) -> Option<usize> {
    let bytes = sql.as_bytes();
    let mut depth = 0;
    let mut quoted = None;
    let mut index = open_paren;

    while index < bytes.len() {
        let byte = bytes[index];

        if let Some(quote) = quoted {
            if byte == b'\\' {
                index += 2;
                continue;
            }

            if byte == quote {
                quoted = None;
            }

            index += 1;
            continue;
        }

        match byte {
            b'\'' | b'`' => quoted = Some(byte),
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }

        index += 1;
    }

    None
}
