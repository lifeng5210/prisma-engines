use crate::{
    datamodel_connector::{Connector, walker_ext_traits::ScalarFieldWalkerExt},
    diagnostics::{DatamodelWarning, Diagnostics},
    parser_database::{
        ReferentialAction,
        ast::WithSpan,
        walkers::{IndexWalker, PrimaryKeyWalker, RelationFieldWalker},
    },
};
use indoc::formatdoc;

const LENGTH_GUIDE: &str = " Please use the `length` argument to the field in the index definition to allow this.";

const NATIVE_TYPES_THAT_CAN_NOT_BE_USED_IN_KEY_SPECIFICATION: &[&str] = &[
    super::TEXT_TYPE_NAME,
    super::LONG_TEXT_TYPE_NAME,
    super::MEDIUM_TEXT_TYPE_NAME,
    super::TINY_TEXT_TYPE_NAME,
    super::BLOB_TYPE_NAME,
    super::TINY_BLOB_TYPE_NAME,
    super::MEDIUM_BLOB_TYPE_NAME,
    super::LONG_BLOB_TYPE_NAME,
];

pub(crate) fn field_types_can_be_used_in_an_index(
    connector: &dyn Connector,
    index: IndexWalker<'_>,
    errors: &mut Diagnostics,
) {
    for field in index.scalar_field_attributes() {
        let Some(native_type) = field.as_index_field().native_type_instance(connector) else {
            continue;
        };
        let (native_type_name, _) = connector.native_type_to_parts(&native_type);

        if !NATIVE_TYPES_THAT_CAN_NOT_BE_USED_IN_KEY_SPECIFICATION.contains(&native_type_name)
            || field.length().is_some()
            || index.is_fulltext()
        {
            continue;
        }

        let error = if index.is_unique() {
            connector
                .native_instance_error(&native_type)
                .new_incompatible_native_type_with_unique(LENGTH_GUIDE, index.ast_attribute().span)
        } else {
            connector
                .native_instance_error(&native_type)
                .new_incompatible_native_type_with_index(LENGTH_GUIDE, index.ast_attribute().span)
        };

        errors.push_error(error);
        break;
    }
}

pub(crate) fn field_types_can_be_used_in_a_primary_key(
    connector: &dyn Connector,
    primary_key: PrimaryKeyWalker<'_>,
    errors: &mut Diagnostics,
) {
    for field in primary_key.scalar_field_attributes() {
        let Some(native_type) = field.as_index_field().native_type_instance(connector) else {
            continue;
        };
        let (native_type_name, _) = connector.native_type_to_parts(&native_type);

        if !NATIVE_TYPES_THAT_CAN_NOT_BE_USED_IN_KEY_SPECIFICATION.contains(&native_type_name)
            || field.length().is_some()
        {
            continue;
        }

        errors.push_error(
            connector
                .native_instance_error(&native_type)
                .new_incompatible_native_type_with_id(LENGTH_GUIDE, primary_key.ast_attribute().span),
        );
        break;
    }
}

pub(crate) fn uses_native_referential_action_set_default(
    connector: &dyn Connector,
    field: RelationFieldWalker<'_>,
    diagnostics: &mut Diagnostics,
) {
    let span_for = |referential_action_type: &str| {
        field
            .ast_field()
            .span_for_argument("relation", referential_action_type)
            .unwrap_or_else(|| field.ast_field().span())
    };
    let warning = || {
        formatdoc!(
            r#"
            {connector_name} does not actually support the `{set_default}` referential action, so using it may result in unexpected errors.
            Read more at https://pris.ly/d/mysql-set-default
            "#,
            connector_name = connector.name(),
            set_default = ReferentialAction::SetDefault.as_str(),
        )
        .replace('\n', " ")
    };

    if let Some(ReferentialAction::SetDefault) = field.explicit_on_delete() {
        diagnostics.push_warning(DatamodelWarning::new(warning(), span_for("onDelete")));
    }
    if let Some(ReferentialAction::SetDefault) = field.explicit_on_update() {
        diagnostics.push_warning(DatamodelWarning::new(warning(), span_for("onUpdate")));
    }
}
