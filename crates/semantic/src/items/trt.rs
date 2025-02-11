use db_utils::define_short_id;
use defs::ids::{
    GenericFunctionId, GenericParamId, LanguageElementId, TraitFunctionId, TraitFunctionLongId,
    TraitId,
};
use diagnostics::{Diagnostics, DiagnosticsBuilder};
use diagnostics_proc_macros::DebugWithDb;
use smol_str::SmolStr;
use syntax::node::{ast, TypedSyntaxNode};
use utils::ordered_hash_map::OrderedHashMap;

use super::attribute::{ast_attributes_to_semantic, Attribute};
use super::generics::semantic_generic_params;
use crate::db::SemanticGroup;
use crate::diagnostic::SemanticDiagnostics;
use crate::expr::compute::Environment;
use crate::resolve_path::Resolver;
use crate::{semantic, GenericArgumentId, SemanticDiagnostic};

#[cfg(test)]
#[path = "trt_test.rs"]
mod test;

#[derive(Clone, Debug, Hash, PartialEq, Eq, DebugWithDb)]
#[debug_db(dyn SemanticGroup + 'static)]
pub struct ConcreteTraitLongId {
    pub trait_id: TraitId,
    pub generic_args: Vec<GenericArgumentId>,
}
define_short_id!(ConcreteTraitId, ConcreteTraitLongId, SemanticGroup, lookup_intern_concrete_trait);

#[derive(Clone, Debug, PartialEq, Eq, DebugWithDb)]
#[debug_db(dyn SemanticGroup + 'static)]
pub struct TraitData {
    diagnostics: Diagnostics<SemanticDiagnostic>,
    generic_params: Vec<GenericParamId>,
    attributes: Vec<Attribute>,
    function_asts: OrderedHashMap<TraitFunctionId, ast::TraitItemFunction>,
}

/// Query implementation of [crate::db::SemanticGroup::trait_semantic_diagnostics].
pub fn trait_semantic_diagnostics(
    db: &dyn SemanticGroup,
    trait_id: TraitId,
) -> Diagnostics<SemanticDiagnostic> {
    let mut diagnostics = DiagnosticsBuilder::default();

    let Some(data) = db.priv_trait_semantic_data(trait_id) else {
        return Diagnostics::default();
    };

    diagnostics.extend(data.diagnostics);
    for trait_function_id in data.function_asts.keys() {
        diagnostics.extend(db.trait_function_diagnostics(*trait_function_id));
    }

    diagnostics.build()
}

/// Query implementation of [crate::db::SemanticGroup::trait_generic_params].
pub fn trait_generic_params(
    db: &dyn SemanticGroup,
    trait_id: TraitId,
) -> Option<Vec<GenericParamId>> {
    Some(db.priv_trait_semantic_data(trait_id)?.generic_params)
}

/// Query implementation of [crate::db::SemanticGroup::trait_attributes].
pub fn trait_attributes(db: &dyn SemanticGroup, trait_id: TraitId) -> Option<Vec<Attribute>> {
    Some(db.priv_trait_semantic_data(trait_id)?.attributes)
}

/// Query implementation of [crate::db::SemanticGroup::trait_functions].
pub fn trait_functions(
    db: &dyn SemanticGroup,
    trait_id: TraitId,
) -> Option<OrderedHashMap<SmolStr, TraitFunctionId>> {
    Some(
        db.priv_trait_semantic_data(trait_id)?
            .function_asts
            .keys()
            .map(|function_id| {
                let function_long_id = db.lookup_intern_trait_function(*function_id);
                (function_long_id.name(db.upcast()), *function_id)
            })
            .collect(),
    )
}

/// Query implementation of [crate::db::SemanticGroup::priv_trait_semantic_data].
pub fn priv_trait_semantic_data(db: &dyn SemanticGroup, trait_id: TraitId) -> Option<TraitData> {
    // TODO(spapini): When asts are rooted on items, don't query module_data directly. Use a
    // selector.

    let syntax_db = db.upcast();
    let module_id = trait_id.module(db.upcast());
    let mut diagnostics = SemanticDiagnostics::new(module_id);
    let module_data = db.module_data(module_id)?;
    let trait_ast = module_data.traits.get(&trait_id)?;

    // Generic params.
    let generic_params = semantic_generic_params(
        db,
        &mut diagnostics,
        module_id,
        &trait_ast.generic_params(syntax_db),
    );

    let attributes = ast_attributes_to_semantic(syntax_db, trait_ast.attributes(syntax_db));
    let mut function_asts = OrderedHashMap::default();
    if let ast::MaybeTraitBody::Some(body) = trait_ast.body(syntax_db) {
        for item in body.items(syntax_db).elements(syntax_db) {
            match item {
                ast::TraitItem::Function(func) => {
                    function_asts.insert(
                        db.intern_trait_function(TraitFunctionLongId(module_id, func.stable_ptr())),
                        func,
                    );
                }
            }
        }
    }

    Some(TraitData { diagnostics: diagnostics.build(), generic_params, attributes, function_asts })
}

// Trait function.
#[derive(Clone, Debug, PartialEq, Eq, DebugWithDb)]
#[debug_db(dyn SemanticGroup + 'static)]
pub struct TraitFunctionData {
    diagnostics: Diagnostics<SemanticDiagnostic>,
    signature: semantic::Signature,
    generic_params: Vec<GenericParamId>,
    attributes: Vec<Attribute>,
}

// Selectors.
/// Query implementation of [crate::db::SemanticGroup::trait_function_diagnostics].
pub fn trait_function_diagnostics(
    db: &dyn SemanticGroup,
    trait_function_id: TraitFunctionId,
) -> Diagnostics<SemanticDiagnostic> {
    db.priv_trait_function_data(trait_function_id).map(|data| data.diagnostics).unwrap_or_default()
}
/// Query implementation of [crate::db::SemanticGroup::trait_function_signature].
pub fn trait_function_signature(
    db: &dyn SemanticGroup,
    trait_function_id: TraitFunctionId,
) -> Option<semantic::Signature> {
    Some(db.priv_trait_function_data(trait_function_id)?.signature)
}
/// Query implementation of [crate::db::SemanticGroup::trait_function_generic_params].
pub fn trait_function_generic_params(
    db: &dyn SemanticGroup,
    trait_function_id: TraitFunctionId,
) -> Option<Vec<GenericParamId>> {
    Some(db.priv_trait_function_data(trait_function_id)?.generic_params)
}

/// Query implementation of [crate::db::SemanticGroup::priv_trait_function_data].
pub fn priv_trait_function_data(
    db: &dyn SemanticGroup,
    trait_function_id: TraitFunctionId,
) -> Option<TraitFunctionData> {
    let module_id = trait_function_id.module(db.upcast());
    let mut diagnostics = SemanticDiagnostics::new(module_id);
    let trait_id = trait_function_id.trait_id(db.upcast());
    let data = db.priv_trait_semantic_data(trait_id)?;
    let function_syntax = &data.function_asts[trait_function_id];
    let generic_params = semantic_generic_params(
        db,
        &mut diagnostics,
        module_id,
        &function_syntax.generic_params(db.upcast()),
    );
    let mut resolver = Resolver::new(db, module_id, &generic_params);
    let syntax_db = db.upcast();
    let signature_syntax = function_syntax.signature(syntax_db);
    let mut environment = Environment::default();
    let signature = semantic::Signature::from_ast(
        &mut diagnostics,
        db,
        &mut resolver,
        &signature_syntax,
        GenericFunctionId::TraitFunction(trait_function_id),
        &mut environment,
    );

    let attributes = ast_attributes_to_semantic(syntax_db, function_syntax.attributes(syntax_db));

    Some(TraitFunctionData {
        diagnostics: diagnostics.build(),
        signature,
        generic_params,
        attributes,
    })
}
