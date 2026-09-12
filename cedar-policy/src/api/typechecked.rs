/*
 * Copyright Cedar Contributors
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *      https://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! A policy set typechecked once, in every request environment of a schema:
//! [`TypecheckedPolicySet`] over [`cedar_policy_core::typechecked`].
//!
//! The typed conditions it hands out are core types
//! (`Expr<Option<Type>>`, [`cedar_policy_core::validator::types::Type`]),
//! as the consumers of typed expressions (the symbolic evaluator) work on
//! those directly.

use std::sync::Arc;

use cedar_policy_core::ast;
use cedar_policy_core::typechecked as core;
pub use cedar_policy_core::typechecked::{
    all_outcomes, EvaluationOutcome, StaticError, TypecheckError, Typecheckable,
};
use cedar_policy_core::validator::types::{RequestEnv as ValidatorRequestEnv, Type};
use cedar_policy_core::validator::ValidatorSchema;
use nonempty_collections::NESet;
use ref_cast::RefCast;

use crate::{Policy, PolicyId, PolicySet, RequestEnv, Schema};

#[cfg(feature = "tpe")]
use cedar_policy_core::tpe::residual::Residual;

/// A policy set typechecked in every request environment up front.
///
/// Every executable policy (static or linked) is validated against the
/// schema and its typed condition kept per environment, so that consumers of
/// typed conditions never re-run the typechecker. The set owns its policies
/// and schema; there is no way to change them, so what was computed never
/// goes stale.
#[derive(Debug, Clone)]
pub struct TypecheckedPolicySet {
    inner: core::TypecheckedPolicySet,
    policies: PolicySet,
    schema: Schema,
}

impl PolicySet {
    /// Validates the policy set against `schema` (strict mode) and typechecks
    /// every executable policy in every request environment.
    pub fn typecheck(&self, schema: &Schema) -> Result<TypecheckedPolicySet, TypecheckError> {
        let inner = core::TypecheckedPolicySet::new(self.ast.clone(), Arc::new(schema.0.clone()))?;
        Ok(TypecheckedPolicySet {
            inner,
            policies: self.clone(),
            schema: schema.clone(),
        })
    }
}

impl TypecheckedPolicySet {
    /// The policy set.
    pub fn policy_set(&self) -> &PolicySet {
        &self.policies
    }

    /// The policy set, giving up what was computed for it.
    pub fn into_policy_set(self) -> PolicySet {
        self.policies
    }

    /// The schema.
    pub fn schema(&self) -> &Schema {
        &self.schema
    }

    /// The typechecked policies, in the policy set's order.
    pub fn policies(&self) -> impl Iterator<Item = &TypecheckedPolicy> {
        self.inner.policies().map(TypecheckedPolicy::ref_cast)
    }

    /// The typechecked policies, mutably (for a fold).
    pub fn policies_mut(&mut self) -> impl Iterator<Item = &mut TypecheckedPolicy> {
        self.inner
            .policies_mut()
            .map(TypecheckedPolicy::ref_cast_mut)
    }

    /// The typechecked policy with `id`.
    pub fn get(&self, id: &PolicyId) -> Option<&TypecheckedPolicy> {
        self.inner.get(id.as_ref()).map(TypecheckedPolicy::ref_cast)
    }

    /// The typechecked policy with `id`, mutably (for a fold).
    pub fn get_mut(&mut self, id: &PolicyId) -> Option<&mut TypecheckedPolicy> {
        self.inner
            .get_mut(id.as_ref())
            .map(TypecheckedPolicy::ref_cast_mut)
    }

    /// The policy with `id`.
    pub fn policy(&self, id: &PolicyId) -> Option<&Policy> {
        self.policies.policy(id)
    }

    /// Constant-folds every policy's condition in every environment with
    /// type-aware partial evaluation against the maximally unknown request of
    /// that environment (known principal, action and resource types; unknown
    /// entities and context): scope constraints become literals, literal
    /// subexpressions are evaluated, and the possible outcomes narrow. A
    /// condition that always errors becomes a [`StaticError`].
    #[cfg(feature = "tpe")]
    #[must_use]
    pub fn constant_fold(self) -> Self {
        Self {
            inner: self.inner.constant_fold(),
            policies: self.policies,
            schema: self.schema,
        }
    }
}

#[doc(hidden)]
impl AsRef<core::TypecheckedPolicySet> for TypecheckedPolicySet {
    fn as_ref(&self) -> &core::TypecheckedPolicySet {
        &self.inner
    }
}

/// One policy of a [`TypecheckedPolicySet`].
///
/// Holds its typed condition, possible outcomes and (with `tpe`) residual in
/// every request environment.
#[repr(transparent)]
#[derive(Debug, Clone, RefCast)]
pub struct TypecheckedPolicy(core::TypecheckedPolicy);

impl TypecheckedPolicy {
    /// The policy's id.
    pub fn id(&self) -> PolicyId {
        PolicyId::new(self.0.id())
    }

    /// The request environments the policy is typechecked in (every
    /// environment of the schema).
    pub fn request_envs(&self) -> impl Iterator<Item = RequestEnv> + '_ {
        self.0.request_types().map(request_env)
    }

    /// The typed condition in `env` (slots substituted, every node typed),
    /// or the static error a fold reduced it to; `None` for an environment
    /// not of the schema.
    pub fn typed_condition(
        &self,
        env: &RequestEnv,
    ) -> Option<&Result<Arc<ast::Expr<Option<Type>>>, StaticError>> {
        self.0.typed_condition(&request_type(env))
    }

    /// The possible outcomes of the condition in `env` (always sound, an
    /// over-approximation); `None` for an environment not of the schema.
    pub fn outcomes(&self, env: &RequestEnv) -> Option<&NESet<EvaluationOutcome>> {
        self.0.outcomes(&request_type(env))
    }

    /// The condition as a residual in `env`; `None` for an environment not
    /// of the schema.
    #[cfg(feature = "tpe")]
    pub fn residual(&self, env: &RequestEnv) -> Option<&Arc<Residual>> {
        self.0.residual(&request_type(env))
    }

    /// Whether `schema` is the schema the conditions were typechecked
    /// against — when the cached typed condition answers
    /// [`Typecheckable::typechecked`].
    pub fn same_schema(&self, schema: &Schema) -> bool {
        self.0.same_schema(&schema.0)
    }

    /// Replaces what is known in `env` by the result of an analysis that
    /// produced `typed` (semantically equivalent to the condition) and
    /// `outcomes` (sound for it) — the symbolic evaluator's fold.
    pub fn fold_env(
        &mut self,
        env: &RequestEnv,
        typed: Arc<ast::Expr<Option<Type>>>,
        outcomes: NESet<EvaluationOutcome>,
    ) -> Result<(), TypecheckError> {
        self.0.fold_env(&request_type(env), typed, outcomes)
    }
}

#[doc(hidden)]
impl AsRef<core::TypecheckedPolicy> for TypecheckedPolicy {
    fn as_ref(&self) -> &core::TypecheckedPolicy {
        &self.0
    }
}

impl Typecheckable for TypecheckedPolicy {
    fn untyped(&self) -> &ast::Expr {
        self.0.untyped()
    }

    fn typechecked(
        &self,
        env: &ValidatorRequestEnv<'_>,
        schema: &ValidatorSchema,
    ) -> Result<Arc<ast::Expr<Option<Type>>>, TypecheckError> {
        self.0.typechecked(env, schema)
    }
}

fn request_type(env: &RequestEnv) -> ast::RequestType {
    ast::RequestType {
        principal: env.principal().as_ref().clone(),
        action: env.action().as_ref().clone(),
        resource: env.resource().as_ref().clone(),
    }
}

fn request_env(rt: &ast::RequestType) -> RequestEnv {
    RequestEnv::new(
        rt.principal.clone().into(),
        rt.action.clone().into(),
        rt.resource.clone().into(),
    )
}

#[cfg(test)]
mod test {
    use super::*;
    use std::str::FromStr;

    const SCHEMA: &str = r"
        entity User { age: Long };
        entity Doc { public: Bool };
        action view appliesTo { principal: User, resource: Doc };
        action edit appliesTo { principal: User, resource: Doc };
    ";

    fn schema() -> Schema {
        Schema::from_cedarschema_str(SCHEMA).expect("schema").0
    }

    fn policies() -> PolicySet {
        PolicySet::from_str(
            r#"permit(principal, action == Action::"view", resource) when { resource.public };"#,
        )
        .expect("policies")
    }

    fn env(action: &str) -> RequestEnv {
        RequestEnv::new(
            "User".parse().expect("type"),
            action.parse().expect("uid"),
            "Doc".parse().expect("type"),
        )
    }

    #[test]
    fn typecheck_and_query() {
        let schema = schema();
        let tps = policies().typecheck(&schema).expect("typechecks");
        assert_eq!(tps.policies().count(), 1);
        let policy = tps.get(&PolicyId::new("policy0")).expect("by id");
        assert_eq!(policy.id(), PolicyId::new("policy0"));
        assert!(tps.policy(&PolicyId::new("policy0")).is_some());
        let mut envs: Vec<_> = policy.request_envs().collect();
        envs.sort();
        assert_eq!(
            envs,
            vec![env(r#"Action::"edit""#), env(r#"Action::"view""#)]
        );
        for e in &envs {
            let typed = policy.typed_condition(e).expect("env");
            assert!(typed.is_ok());
            assert!(policy
                .outcomes(e)
                .expect("env")
                .contains(&EvaluationOutcome::False));
        }
        assert!(policy.same_schema(&schema));
        assert!(policy
            .typed_condition(&RequestEnv::new(
                "Doc".parse().expect("type"),
                r#"Action::"view""#.parse().expect("uid"),
                "Doc".parse().expect("type"),
            ))
            .is_none());
    }

    #[test]
    fn invalid_policies_are_rejected() {
        let ps = PolicySet::from_str("permit(principal, action, resource) when { resource.nope };")
            .expect("policies");
        assert!(matches!(
            ps.typecheck(&schema()),
            Err(TypecheckError::NotWellTyped { .. })
        ));
    }

    #[cfg(feature = "tpe")]
    #[test]
    fn constant_fold_narrows() {
        let tps = policies()
            .typecheck(&schema())
            .expect("typechecks")
            .constant_fold();
        let policy = tps.get(&PolicyId::new("policy0")).expect("by id");
        let edit = env(r#"Action::"edit""#);
        assert!(policy.residual(&edit).expect("env").is_false());
        assert_eq!(
            policy.outcomes(&edit).expect("env"),
            &nonempty_collections::nes![EvaluationOutcome::False]
        );
    }
}
