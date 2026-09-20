use psl::parser_database::walkers::EnumWalker;

use sql_schema_describer as sql;

use crate::sql_schema_calculator::{
    Context, SqlSchemaCalculatorFlavour, sql_schema_calculator_flavour::JoinTableUniquenessConstraint,
};

/// Schema calculation rules for KingbaseES in Oracle compatibility mode.
///
/// Kingbase keeps PostgreSQL catalog storage for enum definitions, while the
/// public column types and DDL are Oracle-compatible. The calculator therefore
/// creates standalone enum objects, but delegates all scalar native-type
/// resolution to the Kingbase Oracle PSL connector.
#[derive(Debug, Default)]
pub(crate) struct KingbaseOracleSchemaCalculatorFlavour;

impl SqlSchemaCalculatorFlavour for KingbaseOracleSchemaCalculatorFlavour {
    fn datamodel_connector(&self) -> &dyn psl::datamodel_connector::Connector {
        psl::builtin_connectors::KINGBASE_ORACLE
    }

    fn calculate_enums(&self, ctx: &mut Context<'_>) {
        for prisma_enum in ctx.datamodel.db.walk_enums() {
            let namespace_id = prisma_enum
                .schema()
                .and_then(|(name, _)| ctx.schemas.get(name).copied())
                .unwrap_or_default();
            let enum_id =
                ctx.schema
                    .describer_schema
                    .push_enum(namespace_id, prisma_enum.database_name().to_owned(), None);

            ctx.enum_ids.insert(prisma_enum.id, enum_id);

            for value in prisma_enum.values() {
                ctx.schema
                    .describer_schema
                    .push_enum_variant(enum_id, value.database_name().to_owned());
            }
        }
    }

    fn column_type_for_enum(&self, enm: EnumWalker<'_>, ctx: &Context<'_>) -> Option<sql::ColumnTypeFamily> {
        ctx.enum_ids.get(&enm.id).copied().map(sql::ColumnTypeFamily::Enum)
    }

    fn column_default_value_for_autoincrement(&self) -> Option<sql::DefaultValue> {
        // The renderer keeps the Oracle NUMBER type and creates an explicit
        // sequence. Kingbase records nextval() defaults in its
        // PostgreSQL-compatible catalog.
        Some(sql::DefaultValue::sequence(""))
    }

    fn m2m_join_table_constraint(&self) -> JoinTableUniquenessConstraint {
        JoinTableUniquenessConstraint::PrimaryKey
    }
}
