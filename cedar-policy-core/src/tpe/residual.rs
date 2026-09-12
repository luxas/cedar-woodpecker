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

//! This module contains the residual.

use std::collections::HashSet;
use std::{collections::BTreeMap, sync::Arc, sync::LazyLock};

use crate::ast::{
    Annotations, Effect, EntityUID, Literal, Policy, PolicyID, SlotEnv, UnwrapInfallible, ValueKind,
};
use crate::evaluator::evaluation_errors;
#[cfg(feature = "tolerant-ast")]
use crate::tpe::err::ErrorNotSupportedError;
use crate::tpe::err::{ExprToResidualError, MissingTypeAnnotationError, UnknownNotSupportedError};
use crate::validator::types::{BoolType, Type};
use crate::{
    ast::{self, BinaryOp, EntityType, Expr, Name, Pattern, UnaryOp, Value, Var},
    expr_builder::ExprBuilder,
};
use nonempty_collections::{nes, NESet, NonEmptyIterator};
use smol_str::SmolStr;

/// The [`Name`] used to represent error nodes in the AST when converting from
/// [`Residual::Error`]. This is a synthetic extension function call that does
/// not correspond to any real Cedar extension.
#[expect(clippy::unwrap_used, reason = "error is a valid name")]
pub(crate) static ERROR_NAME: LazyLock<Name> = LazyLock::new(|| "error".parse().unwrap());

/// The residual produced by TPE
#[derive(Debug, Clone)]
pub enum Residual {
    /// TPE produces a partial expression
    Partial {
        /// The kind of partial expression
        kind: ResidualKind,
        /// Type of the partial expression
        ty: Type,
    },
    /// TPE produces a concrete value
    Concrete {
        /// The concrete value
        value: Value,
        /// Type of the value
        ty: Type,
    },
    /// TPE produces a (typed) error
    /// Evaluating the residual of this variant always produces an evaluation
    /// error. The error kind does not matter for the sake of re-authorization.
    Error(Type),
}

impl Residual {
    /// Construct a residual policy
    pub fn to_policy(self, id: PolicyID, effect: Effect, annotations: Annotations) -> Policy {
        Policy::from_when_clause_annos(
            effect,
            Arc::new(self.into()),
            id,
            None,
            Arc::new(annotations),
        )
    }

    /// All literal uids referenced by this residual
    pub fn all_literal_uids(&self) -> HashSet<EntityUID> {
        match self {
            Residual::Partial { kind, .. } => kind.all_literal_uids(),
            Residual::Concrete { value, .. } => value.all_literal_uids(),
            Residual::Error(_) => HashSet::new(),
        }
    }

    /// Get the type of this residual
    pub fn ty(&self) -> &Type {
        match self {
            Residual::Partial { ty, .. } => ty,
            Residual::Concrete { ty, .. } => ty,
            Residual::Error(ty) => ty,
        }
    }

    /// Whether this residual can result in a runtime error, assuming that self is well-formed, that is, has been validated against a schema.
    pub fn can_error_assuming_well_formed(&self) -> bool {
        match self {
            Residual::Concrete { .. } => false,
            Residual::Error(_) => true,
            Residual::Partial { kind, .. } => match kind {
                // Keep the same order of cases here as in tpe::Evaluator::interpret
                ResidualKind::Var(_) => false,
                // The general rule here is that an expression can only error if any child expression can error.
                ResidualKind::And { left, right } => {
                    left.can_error_assuming_well_formed() || right.can_error_assuming_well_formed()
                }
                ResidualKind::Or { left, right } => {
                    left.can_error_assuming_well_formed() || right.can_error_assuming_well_formed()
                }
                ResidualKind::If {
                    test_expr,
                    then_expr,
                    else_expr,
                } => {
                    test_expr.can_error_assuming_well_formed()
                        || then_expr.can_error_assuming_well_formed()
                        || else_expr.can_error_assuming_well_formed()
                }
                ResidualKind::Is { expr, .. } => expr.can_error_assuming_well_formed(),
                ResidualKind::Like { expr, .. } => expr.can_error_assuming_well_formed(),

                ResidualKind::BinaryApp { op, arg1, arg2 } => match op {
                    // Arithmetic operations could error due to integer overflow
                    ast::BinaryOp::Add => true,
                    ast::BinaryOp::Mul => true,
                    ast::BinaryOp::Sub => true,

                    // <entityUID>.getTag possibly errors during reauthorization if <entityUID> does not exist in the entity store
                    ast::BinaryOp::GetTag => true,

                    // Other binary operations follow the general rule. They are all enumerated here for clarity, although
                    // a _ case could be used.
                    ast::BinaryOp::Contains
                    | ast::BinaryOp::ContainsAll
                    | ast::BinaryOp::ContainsAny
                    | ast::BinaryOp::Eq
                    | ast::BinaryOp::HasTag
                    | ast::BinaryOp::In
                    | ast::BinaryOp::Less
                    | ast::BinaryOp::LessEq => {
                        arg1.can_error_assuming_well_formed()
                            || arg2.can_error_assuming_well_formed()
                    }
                },

                // Extension function invocations can error at runtime.
                ResidualKind::ExtensionFunctionApp { .. } => true,
                // <entityUID>.<attr> possibly errors during reauthorization if <entityUID> does not exist in the entity store
                ResidualKind::GetAttr { .. } => true,

                ResidualKind::HasAttr { expr, .. } => expr.can_error_assuming_well_formed(),

                ResidualKind::UnaryApp { op, arg } => match op {
                    // Integer negation can error due to integer overflow
                    ast::UnaryOp::Neg => true,

                    // General rule for the rest of the unary operations.
                    ast::UnaryOp::IsEmpty | ast::UnaryOp::Not => {
                        arg.can_error_assuming_well_formed()
                    }
                },
                ResidualKind::Set(items) => items.iter().any(Self::can_error_assuming_well_formed),
                ResidualKind::Record(attrs) => attrs
                    .iter()
                    .any(|(_, e)| e.can_error_assuming_well_formed()),
            },
        }
    }
}

impl TryFrom<Residual> for Value {
    type Error = ();
    /// INVARIANT: TPE evaluator assumes this function cannot error if `value.is_concrete()`
    fn try_from(value: Residual) -> std::result::Result<Self, Self::Error> {
        match value {
            Residual::Concrete { value, .. } => Ok(value),
            _ => Err(()),
        }
    }
}

impl Residual {
    /// Convert a typed expression to a residual.
    ///
    /// Takes a `SlotEnv` used to resolve template slots to their values. Each
    /// template slot expression becomes an entityUID literal for the binding of
    /// that slot in the environment.
    pub fn try_from_typed_expr(
        expr: &Expr<Option<Type>>,
        env: &SlotEnv,
    ) -> std::result::Result<Self, ExprToResidualError> {
        let ty = expr.data().clone().ok_or(MissingTypeAnnotationError)?;

        // Otherwise, convert to a partial residual
        let kind = match expr.expr_kind() {
            ast::ExprKind::Var(var) => ResidualKind::Var(*var),
            ast::ExprKind::If {
                test_expr,
                then_expr,
                else_expr,
            } => ResidualKind::If {
                test_expr: Arc::new(Self::try_from_typed_expr(test_expr.as_ref(), env)?),
                then_expr: Arc::new(Self::try_from_typed_expr(then_expr.as_ref(), env)?),
                else_expr: Arc::new(Self::try_from_typed_expr(else_expr.as_ref(), env)?),
            },
            ast::ExprKind::And { left, right } => ResidualKind::And {
                left: Arc::new(Self::try_from_typed_expr(left.as_ref(), env)?),
                right: Arc::new(Self::try_from_typed_expr(right.as_ref(), env)?),
            },
            ast::ExprKind::Or { left, right } => ResidualKind::Or {
                left: Arc::new(Self::try_from_typed_expr(left.as_ref(), env)?),
                right: Arc::new(Self::try_from_typed_expr(right.as_ref(), env)?),
            },
            ast::ExprKind::UnaryApp { op, arg } => ResidualKind::UnaryApp {
                op: *op,
                arg: Arc::new(Self::try_from_typed_expr(arg.as_ref(), env)?),
            },
            ast::ExprKind::BinaryApp { op, arg1, arg2 } => ResidualKind::BinaryApp {
                op: *op,
                arg1: Arc::new(Self::try_from_typed_expr(arg1.as_ref(), env)?),
                arg2: Arc::new(Self::try_from_typed_expr(arg2.as_ref(), env)?),
            },
            ast::ExprKind::ExtensionFunctionApp { fn_name, args } => {
                let residual_args: Result<Vec<_>, _> = args
                    .iter()
                    .map(|e| Self::try_from_typed_expr(e, env))
                    .collect();
                ResidualKind::ExtensionFunctionApp {
                    fn_name: fn_name.clone(),
                    args: Arc::new(residual_args?),
                }
            }
            ast::ExprKind::GetAttr { expr, attr } => ResidualKind::GetAttr {
                expr: Arc::new(Self::try_from_typed_expr(expr.as_ref(), env)?),
                attr: attr.clone(),
            },
            ast::ExprKind::HasAttr { expr, attr } => ResidualKind::HasAttr {
                expr: Arc::new(Self::try_from_typed_expr(expr.as_ref(), env)?),
                attr: attr.clone(),
            },
            ast::ExprKind::Like { expr, pattern } => ResidualKind::Like {
                expr: Arc::new(Self::try_from_typed_expr(expr.as_ref(), env)?),
                pattern: pattern.clone(),
            },
            ast::ExprKind::Is { expr, entity_type } => ResidualKind::Is {
                expr: Arc::new(Self::try_from_typed_expr(expr.as_ref(), env)?),
                entity_type: entity_type.clone(),
            },
            ast::ExprKind::Set(elements) => {
                let residual_elements: Result<Vec<_>, _> = elements
                    .iter()
                    .map(|e| Self::try_from_typed_expr(e, env))
                    .collect();
                ResidualKind::Set(Arc::new(residual_elements?))
            }
            ast::ExprKind::Record(map) => {
                let residual_map: Result<BTreeMap<_, _>, ExprToResidualError> = map
                    .iter()
                    .map(|(k, v)| Ok((k.clone(), Self::try_from_typed_expr(v, env)?)))
                    .collect();
                ResidualKind::Record(Arc::new(residual_map?))
            }
            // Literals should be converted to concrete values
            ast::ExprKind::Lit(lit) => {
                let value = Value::new(lit.clone(), None);
                return Ok(Residual::Concrete { value, ty });
            }
            // Slots are resolved to their linked values in translation to residual
            ast::ExprKind::Slot(slot) => match env.get(slot) {
                Some(euid) => {
                    let value = Value::from(euid.clone());
                    return Ok(Residual::Concrete { value, ty });
                }
                None => {
                    // Any slot not bound in the env is an error now rather than waiting for evaluation
                    return Err(evaluation_errors::UnlinkedSlotError {
                        slot: *slot,
                        source_loc: expr.source_loc().cloned(),
                    }
                    .into());
                }
            },
            ast::ExprKind::Unknown(_) => return Err(UnknownNotSupportedError.into()),
            #[cfg(feature = "tolerant-ast")]
            ast::ExprKind::Error { .. } => {
                return Err(ErrorNotSupportedError.into());
            }
        };

        Ok(Residual::Partial { kind, ty })
    }
}

/// The kind of partial expression
#[derive(Debug, Clone)]
pub enum ResidualKind {
    /// Variable
    Var(Var),
    /// If-then-else expression
    If {
        /// Condition for the ternary expression. Must evaluate to Bool type
        test_expr: Arc<Residual>,
        /// Value if true
        then_expr: Arc<Residual>,
        /// Value if false
        else_expr: Arc<Residual>,
    },
    /// Boolean AND
    And {
        /// Left operand, which will be eagerly evaluated
        left: Arc<Residual>,
        /// Right operand, which may not be evaluated due to short-circuiting
        right: Arc<Residual>,
    },
    /// Boolean OR
    Or {
        /// Left operand, which will be eagerly evaluated
        left: Arc<Residual>,
        /// Right operand, which may not be evaluated due to short-circuiting
        right: Arc<Residual>,
    },
    /// Application of a built-in unary operator (single parameter)
    UnaryApp {
        /// Unary operator to apply
        op: UnaryOp,
        /// Argument to apply operator to
        arg: Arc<Residual>,
    },
    /// Application of a built-in binary operator (two parameters)
    BinaryApp {
        /// Binary operator to apply
        op: BinaryOp,
        /// First arg
        arg1: Arc<Residual>,
        /// Second arg
        arg2: Arc<Residual>,
    },
    /// Application of an extension function to n arguments
    /// INVARIANT (MethodStyleArgs):
    ///   if op.style is MethodStyle then args _cannot_ be empty.
    ///     The first element of args refers to the subject of the method call
    /// Ideally, we find some way to make this non-representable.
    ExtensionFunctionApp {
        /// Extension function to apply
        fn_name: Name,
        /// Args to apply the function to
        args: Arc<Vec<Residual>>,
    },
    /// Get an attribute of an entity, or a field of a record
    GetAttr {
        /// Expression to get an attribute/field of. Must evaluate to either
        /// Entity or Record type
        expr: Arc<Residual>,
        /// Attribute or field to get
        attr: SmolStr,
    },
    /// Does the given `expr` have the given `attr`?
    HasAttr {
        /// Expression to test. Must evaluate to either Entity or Record type
        expr: Arc<Residual>,
        /// Attribute or field to check for
        attr: SmolStr,
    },
    /// Regex-like string matching similar to IAM's `StringLike` operator.
    Like {
        /// Expression to test. Must evaluate to String type
        expr: Arc<Residual>,
        /// Pattern to match on; can include the wildcard *, which matches any string.
        /// To match a literal `*` in the test expression, users can use `\*`.
        /// Be careful the backslash in `\*` must not be another escape sequence. For instance, `\\*` matches a backslash plus an arbitrary string.
        pattern: Pattern,
    },
    /// Entity type test. Does the first argument have the entity type
    /// specified by the second argument.
    Is {
        /// Expression to test. Must evaluate to an Entity.
        expr: Arc<Residual>,
        /// The [`EntityType`] used for the type membership test.
        entity_type: EntityType,
    },
    /// Set (whose elements may be arbitrary expressions)
    //
    // This is backed by `Vec` (and not e.g. `HashSet`), because two `Expr`s
    // that are syntactically unequal, may actually be semantically equal --
    // i.e., we can't do the dedup of duplicates until all of the `Expr`s are
    // evaluated into `Value`s
    Set(Arc<Vec<Residual>>),
    /// Anonymous record (whose elements may be arbitrary expressions)
    Record(Arc<BTreeMap<SmolStr, Residual>>),
}

impl ResidualKind {
    /// All literal uids referenced by this residual kind
    pub fn all_literal_uids(&self) -> HashSet<EntityUID> {
        match self {
            ResidualKind::Var(_) => HashSet::new(),
            ResidualKind::If {
                test_expr,
                then_expr,
                else_expr,
            } => {
                let mut uids = test_expr.all_literal_uids();
                uids.extend(then_expr.all_literal_uids());
                uids.extend(else_expr.all_literal_uids());
                uids
            }
            ResidualKind::And { left, right } | ResidualKind::Or { left, right } => {
                let mut uids = left.all_literal_uids();
                uids.extend(right.all_literal_uids());
                uids
            }
            ResidualKind::UnaryApp { arg, .. } => arg.all_literal_uids(),
            ResidualKind::BinaryApp { arg1, arg2, .. } => {
                let mut uids = arg1.all_literal_uids();
                uids.extend(arg2.all_literal_uids());
                uids
            }
            ResidualKind::ExtensionFunctionApp { args, .. } => {
                let mut uids = HashSet::new();
                for arg in args.as_ref() {
                    uids.extend(arg.all_literal_uids());
                }
                uids
            }
            ResidualKind::GetAttr { expr, .. }
            | ResidualKind::HasAttr { expr, .. }
            | ResidualKind::Like { expr, .. }
            | ResidualKind::Is { expr, .. } => expr.all_literal_uids(),
            ResidualKind::Set(elements) => {
                let mut uids = HashSet::new();
                for element in elements.as_ref() {
                    uids.extend(element.all_literal_uids());
                }
                uids
            }
            ResidualKind::Record(map) => {
                let mut uids = HashSet::new();
                for value in map.values() {
                    uids.extend(value.all_literal_uids());
                }
                uids
            }
        }
    }
}

impl Residual {
    /// If a residual is trivially true
    pub fn is_true(&self) -> bool {
        matches!(
            self,
            Residual::Concrete {
                value: Value {
                    value: ValueKind::Lit(Literal::Bool(true)),
                    ..
                },
                ..
            }
        )
    }

    /// If a residual is trivially false
    pub fn is_false(&self) -> bool {
        matches!(
            self,
            Residual::Concrete {
                value: Value {
                    value: ValueKind::Lit(Literal::Bool(false)),
                    ..
                },
                ..
            }
        )
    }

    /// If a residual is an error
    pub fn is_error(&self) -> bool {
        matches!(self, Residual::Error { .. })
    }

    /// If a residual is a concrete value
    pub fn is_concrete(&self) -> bool {
        matches!(self, Residual::Concrete { .. })
    }

    /// If a residual is still partially unknown
    pub fn is_partial(&self) -> bool {
        matches!(self, Residual::Partial { .. })
    }

    /// Describes what outcomes are possible after concrete evaluation an expression into a Value,
    /// then running value.get_as_bool(). Thus, if the expression evaluates to something else than
    /// a boolean, EvaluationOutcome::Error is returned.
    ///
    /// The result is an over-approximation: every outcome that concrete evaluation can actually
    /// produce is in the returned set, but the set may contain outcomes that cannot occur. It is
    /// exact on the short-circuiting structure (`&&`, `||`, `if`, `!`) and approximate at the
    /// leaves, where it knows only the leaf's type and [`Residual::can_error_assuming_well_formed`].
    /// It also treats each leaf occurrence as independent, so it does not see that
    /// `principal.foo && !principal.foo` is never `True`.
    ///
    /// Callers may therefore rely on an outcome's *absence* (it definitely cannot happen), but not
    /// on its presence.
    ///
    /// As with [`Residual::can_error_assuming_well_formed`], this assumes the residual is
    /// well-formed, that is, has been validated against a schema.
    pub fn possible_bool_outcomes(&self) -> NESet<EvaluationOutcome> {
        let bool_with_possible_error = |ty: &Type| {
            match ty {
                // The validator refines the type of an expression it can prove always evaluates to
                // `true` (resp. `false`) to the singleton type `Bool(True)` (resp. `Bool(False)`).
                // Such an expression may still error at runtime, but whenever it does produce a
                // value, that value is the one named by the singleton type.
                Type::Bool(bool_ty) => {
                    let mut outcomes = match bool_ty {
                        // TODO(luxas): We could rely on the validator, but we in fact don't/won't right now,
                        // but instead will resolve some simple cases in the TPE evaluator "by hand".
                        // Thus, remove these in favor for that other PR doing that, and see what is the diff between what the evaluator refines and we don't.
                        BoolType::True => nes![EvaluationOutcome::True],
                        BoolType::False => nes![EvaluationOutcome::False],
                        BoolType::AnyBool => {
                            nes![EvaluationOutcome::True, EvaluationOutcome::False]
                        }
                    };
                    // `can_error_assuming_well_formed` is itself an over-approximation, so adding
                    // `Error` on its say-so keeps this an over-approximation too.
                    if self.can_error_assuming_well_formed() {
                        outcomes.insert(EvaluationOutcome::Error);
                    }
                    outcomes
                }
                _ => nes![EvaluationOutcome::Error], // Running Value.get_as_bool() on something that is not a bool errors.
            }
        };

        match self {
            Residual::Concrete { value, .. } => match value.get_as_bool() {
                Ok(true) => nes![EvaluationOutcome::True],
                Ok(false) => nes![EvaluationOutcome::False],
                Err(_) => nes![EvaluationOutcome::Error], // Running Value.get_as_bool() on something that is not a bool errors.
            },
            Residual::Error(_) => nes![EvaluationOutcome::Error],
            Residual::Partial { kind, ty } => match kind {
                // The general rule here is that an expression can only error if any child expression can error.
                ResidualKind::And { left, right } => {
                    left.possible_bool_outcomes()
                        .nonempty_iter()
                        .flat_map(|o| match o {
                            EvaluationOutcome::True => right.possible_bool_outcomes(),
                            EvaluationOutcome::False => nes![EvaluationOutcome::False], // false && <RHS> short circuits to false
                            EvaluationOutcome::Error => nes![EvaluationOutcome::Error],
                        })
                        .collect()
                }
                ResidualKind::Or { left, right } => {
                    left.possible_bool_outcomes()
                        .nonempty_iter()
                        .flat_map(|o| match o {
                            EvaluationOutcome::False => right.possible_bool_outcomes(),
                            EvaluationOutcome::True => nes![EvaluationOutcome::True], // true || <RHS> short circuits to true
                            EvaluationOutcome::Error => nes![EvaluationOutcome::Error],
                        })
                        .collect()
                }
                ResidualKind::If {
                    test_expr,
                    then_expr,
                    else_expr,
                } => test_expr
                    .possible_bool_outcomes()
                    .nonempty_iter()
                    .flat_map(|o| match o {
                        EvaluationOutcome::True => then_expr.possible_bool_outcomes(),
                        EvaluationOutcome::False => else_expr.possible_bool_outcomes(),
                        EvaluationOutcome::Error => nes![EvaluationOutcome::Error],
                    })
                    .collect(),
                ResidualKind::UnaryApp { op, arg } => match op {
                    ast::UnaryOp::Not => arg
                        .possible_bool_outcomes()
                        .nonempty_iter()
                        .map(|o| match o {
                            EvaluationOutcome::True => EvaluationOutcome::False,
                            EvaluationOutcome::False => EvaluationOutcome::True,
                            EvaluationOutcome::Error => EvaluationOutcome::Error,
                        })
                        .collect(),
                    ast::UnaryOp::Neg | ast::UnaryOp::IsEmpty => bool_with_possible_error(ty),
                },

                // TODO: Maybe implement is_error_free in terms of this more general function instead of vice versa?
                // If we wanted to do that, we would need to basically add a variant called NonBool, so Expr nodes would
                // return e.g. [NonBool] or [NonBool, Expr] corresponding to "error-free or not", and an operator like "like"
                // would turn the NonBool one into [True, False] (if the type matches).
                ResidualKind::Var(_)
                | ResidualKind::Is { .. }
                // TODO(luxas): Refine <error-free> like "*" => [True] (also in TPE) and <can-error> like "*" => [True, Error].
                | ResidualKind::Like { .. }
                | ResidualKind::BinaryApp { .. }
                | ResidualKind::ExtensionFunctionApp { .. }
                | ResidualKind::GetAttr { .. }
                | ResidualKind::HasAttr { .. }
                | ResidualKind::Set(_)
                | ResidualKind::Record(_) => bool_with_possible_error(ty),
            },
        }
    }
}

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
/// Describes the possible evaluation outcomes from evaluating a residual/expression.
pub enum EvaluationOutcome {
    /// Describes the residual evaluation to true
    True,
    /// Describes the residual evaluation to false
    False,
    /// Describes the residual evaluation to some error
    Error, // TODO: Split into RuntimeError, EntityNotExistError and TypeError?
}

/// Conversion from `Residual` to `Expr` so that we can use the concrete evaluator for re-authorization
impl From<Residual> for Expr {
    fn from(value: Residual) -> Expr {
        match value {
            Residual::Partial { kind, .. } => {
                let builder: ast::ExprBuilder<()> = ExprBuilder::with_data(());
                match kind {
                    ResidualKind::And { left, right } => {
                        builder.and(left.as_ref().clone().into(), right.as_ref().clone().into())
                    }
                    ResidualKind::BinaryApp { op, arg1, arg2 } => builder.binary_app(
                        op,
                        arg1.as_ref().clone().into(),
                        arg2.as_ref().clone().into(),
                    ),
                    ResidualKind::ExtensionFunctionApp { fn_name, args } => builder
                        .call_extension_fn(
                            fn_name,
                            args.as_ref().clone().into_iter().map(|arg| arg.into()),
                        )
                        .unwrap_infallible(),
                    ResidualKind::GetAttr { expr, attr } => {
                        builder.get_attr(expr.as_ref().clone().into(), attr)
                    }
                    ResidualKind::HasAttr { expr, attr } => {
                        builder.has_attr(expr.as_ref().clone().into(), attr)
                    }
                    ResidualKind::If {
                        test_expr,
                        then_expr,
                        else_expr,
                    } => builder.ite(
                        test_expr.as_ref().clone().into(),
                        then_expr.as_ref().clone().into(),
                        else_expr.as_ref().clone().into(),
                    ),
                    ResidualKind::Is { expr, entity_type } => {
                        builder.is_entity_type(expr.as_ref().clone().into(), entity_type)
                    }
                    ResidualKind::Like { expr, pattern } => {
                        builder.like(expr.as_ref().clone().into(), pattern)
                    }
                    ResidualKind::Or { left, right } => {
                        builder.or(left.as_ref().clone().into(), right.as_ref().clone().into())
                    }
                    #[expect(clippy::expect_used, reason = "record construction should succeed")]
                    ResidualKind::Record(map) => builder
                        .record(map.as_ref().clone().into_iter().map(|(k, v)| (k, v.into())))
                        .expect("should succeed"),
                    ResidualKind::Set(set) => {
                        builder.set(set.as_ref().clone().into_iter().map(|v| v.into()))
                    }
                    ResidualKind::UnaryApp { op, arg } => {
                        builder.unary_app(op, arg.as_ref().clone().into())
                    }
                    ResidualKind::Var(v) => builder.var(v),
                }
            }
            Residual::Concrete { value, .. } => value.into(),
            Residual::Error(_) => {
                let builder: ast::ExprBuilder<()> = ExprBuilder::with_data(());
                builder
                    .call_extension_fn(ERROR_NAME.clone(), std::iter::empty())
                    .unwrap_infallible()
            }
        }
    }
}

#[cfg(test)]
pub(super) mod test {
    use super::*;
    use crate::ast::SlotId;
    use crate::extensions::Extensions;
    use crate::parser::parse_expr;
    use crate::tpe::request::PartialRequest;
    use crate::tpe::test_utils::parse_partial_euid;
    use crate::validator::types::BoolType;
    use crate::validator::ValidatorSchema;
    use cool_asserts::assert_matches;
    use similar_asserts::assert_eq;

    #[track_caller]
    pub(crate) fn parse_typed_expr(expr_str: &str, slot_env: &SlotEnv) -> Expr<Option<Type>> {
        let schema = ValidatorSchema::from_cedarschema_str(r#"
            entity User in Organization { foo: Bool, str: String, num: Long, period: __cedar::duration, set: Set<String> } tags String;
            entity Organization;
            entity Document in Organization;
            action get appliesTo { principal: [User], resource: [Document] };"#,
            &Extensions::all_available(),
        )
        .unwrap()
        .0;

        let request = PartialRequest::new(
            parse_partial_euid("User"),
            r#"Action::"get""#.parse().unwrap(),
            parse_partial_euid("Document"),
            None,
            &schema,
        )
        .unwrap();

        crate::tpe::test_utils::parse_typed_expr(expr_str, &request, &schema, slot_env)
    }

    #[track_caller]
    fn parse_residual(expr_str: &str) -> Residual {
        let typed_expr = parse_typed_expr(expr_str, &SlotEnv::new());
        Residual::try_from_typed_expr(&typed_expr, &SlotEnv::new()).unwrap()
    }

    #[test]
    fn slot_to_residual() {
        let env = SlotEnv::from([
            (SlotId::principal(), r#"User::"alice""#.parse().unwrap()),
            (
                SlotId::resource(),
                r#"Organization::"org""#.parse().unwrap(),
            ),
        ]);

        assert_eq!(
            Expr::from(
                Residual::try_from_typed_expr(
                    &parse_typed_expr("principal == ?principal && resource in ?resource", &env),
                    &env
                )
                .unwrap()
            ),
            parse_expr(r#"principal == User::"alice" && resource in Organization::"org""#).unwrap()
        );
    }

    #[test]
    fn to_residual_missing_slot() {
        let env = SlotEnv::new();

        assert_matches!(
            Residual::try_from_typed_expr(&parse_typed_expr("principal in ?principal", &env), &env),
            Err(ExprToResidualError::UnlinkedSlotError(_))
        );

        assert_matches!(
            Residual::try_from_typed_expr(&parse_typed_expr("resource in ?resource", &env), &env),
            Err(ExprToResidualError::UnlinkedSlotError(_))
        );
    }

    #[test]
    fn test_can_error_assuming_well_formed() {
        // Most common LHS, the policy header
        assert_eq!(
            parse_residual(
                r#"
                principal is User &&
                principal in Organization::"foo" &&
                action == Action::"get" &&
                resource is Document &&
                resource in Organization::"foo"
                "#
            )
            .can_error_assuming_well_formed(),
            false
        );
        assert_eq!(
            parse_residual(r#"User::"jane" in [User::"foo", User::"jane"]"#)
                .can_error_assuming_well_formed(),
            false
        );
        assert_eq!(
            parse_residual(r#"principal has foo || principal.hasTag("foo")"#)
                .can_error_assuming_well_formed(),
            false
        );
        assert_eq!(
            parse_residual(r#"principal == resource && !(principal in Organization::"foo")"#)
                .can_error_assuming_well_formed(),
            false
        );
        assert_eq!(
            parse_residual(
                r#"
                if principal.hasTag("foo") then
                    principal in Organization::"foo"
                else principal in Organization::"bar"
                "#
            )
            .can_error_assuming_well_formed(),
            false
        );
        assert_eq!(
            parse_residual(
                r#"
                1 == 2 ||
                !("a" == "b") &&
                ["a", "b"].contains("a") &&
                !["a", "b"].containsAll(["a"]) &&
                ["a", "b"].containsAny(["a"])
                "#
            )
            .can_error_assuming_well_formed(),
            false
        );
        assert_eq!(
            parse_residual(r#"{a: true, b: false}["a"] && false"#).can_error_assuming_well_formed(),
            true
        );
        assert_eq!(
            parse_residual(r#"User::"jane".str like "jane-*""#).can_error_assuming_well_formed(),
            true
        );
        assert_eq!(
            parse_residual(
                r#"if principal.num > 0 then User::"jane".num >= 100 else User::"foo".num == 1"#
            )
            .can_error_assuming_well_formed(),
            true
        );
        assert_eq!(
            parse_residual(r#"principal.hasTag("foo") && principal.getTag("foo") == "bar""#)
                .can_error_assuming_well_formed(),
            true
        );
        assert_eq!(
            parse_residual(
                r#"
                !principal.set.isEmpty() && (
                    principal.set.contains("foo") ||
                    principal.set.containsAll(["foo", "bar"]) ||
                    principal.set.containsAny(["foo", "bar"])
                )"#
            )
            .can_error_assuming_well_formed(),
            true
        );
        assert_eq!(
            parse_residual(r#"principal.num + 1 == 100 || true"#).can_error_assuming_well_formed(),
            true
        );
        assert_eq!(
            parse_residual(r#"if principal.foo then principal.num - 1 == 100 else true"#)
                .can_error_assuming_well_formed(),
            true
        );
        assert_eq!(
            parse_residual(r#"principal.foo && principal.num * 2 == 100"#)
                .can_error_assuming_well_formed(),
            true
        );
        assert_eq!(
            parse_residual(r#"principal.foo || -principal.num == 100"#)
                .can_error_assuming_well_formed(),
            true
        );
        assert_eq!(
            parse_residual(r#"principal.num == 1 && principal.period < (if principal.foo then duration("1d") else duration("2d"))"#).can_error_assuming_well_formed(),
            true
        );
        // in reality, this specific function could most likely never error
        // in the future, we might want to be more precise about exactly what functions could produce errors
        assert_eq!(
            parse_residual(r#"principal.period.toDays() == 365"#).can_error_assuming_well_formed(),
            true
        );
        assert_eq!(
            Residual::Error(Type::Bool(BoolType::AnyBool)).can_error_assuming_well_formed(),
            true
        );
    }

    mod literal_uids {
        use similar_asserts::assert_eq;
        use std::collections::HashSet;

        use super::parse_residual;

        #[test]
        fn var() {
            assert_eq!(
                parse_residual("principal.foo").all_literal_uids(),
                HashSet::new()
            );
        }

        #[test]
        fn r#if() {
            assert_eq!(
                parse_residual(
                    r#"if User::"alice".foo then User::"bob".foo else User::"jane".foo"#
                )
                .all_literal_uids(),
                HashSet::from([
                    r#"User::"alice""#.parse().unwrap(),
                    r#"User::"bob""#.parse().unwrap(),
                    r#"User::"jane""#.parse().unwrap(),
                ])
            );
        }

        #[test]
        fn and() {
            assert_eq!(
                parse_residual(r#"User::"alice".foo && User::"jane".foo"#).all_literal_uids(),
                HashSet::from([
                    r#"User::"alice""#.parse().unwrap(),
                    r#"User::"jane""#.parse().unwrap(),
                ])
            );
        }

        #[test]
        fn set() {
            assert_eq!(
                parse_residual(r#"principal in [User::"alice", User::"jane"]"#).all_literal_uids(),
                HashSet::from([
                    r#"User::"alice""#.parse().unwrap(),
                    r#"User::"jane""#.parse().unwrap(),
                ])
            );
        }

        #[test]
        fn record() {
            assert_eq!(
                parse_residual(r#"(if principal.foo then {a: User::"alice", b: true} else {a: User::"jane", b: false}).a.foo"#).all_literal_uids(),
                HashSet::from([
                    r#"User::"alice""#.parse().unwrap(),
                    r#"User::"jane""#.parse().unwrap(),
                ])
            );
        }
    }

    /// Tests for [`Residual::possible_bool_outcomes`].
    ///
    /// The method over-approximates the set of outcomes that "evaluate this
    /// residual, then call [`Value::get_as_bool`] on the result" can produce.
    /// Two properties matter:
    ///
    /// * **Soundness**: every outcome that can really occur is in the returned
    ///   set. A violation is dangerous, because a caller could conclude that a
    ///   residual never errors when in fact it can.
    /// * **Completeness**: no outcome that cannot occur is in the returned set.
    ///   A violation only costs precision.
    ///
    /// The method is exact on the short-circuiting structure (`&&`, `||`, `if`,
    /// `!`) and approximate at the leaves, where it knows only the leaf's type
    /// and [`Residual::can_error_assuming_well_formed`]. It also does not track
    /// correlations between leaves: `principal.foo && !principal.foo` reports
    /// `True` as possible even though it never is.
    ///
    /// Besides the case-by-case tests below, soundness and completeness are
    /// checked two ways:
    ///
    /// * [`model_check`] compares the method against a brute-force reference
    ///   semantics over every combination of leaf outcomes, for thousands of
    ///   residual shapes.
    /// * [`concrete_evaluator`] checks the predictions against what the real
    ///   Cedar evaluator produces on a range of entity stores.
    mod possible_bool_outcomes {
        use super::super::*;
        use super::parse_residual;
        use crate::validator::types::BoolType;
        use similar_asserts::assert_eq;
        use std::collections::HashSet;

        use EvaluationOutcome::{Error as E, False as F, True as T};

        /// `possible_bool_outcomes` as a plain `HashSet`, for easy comparison.
        fn outcomes_of(residual: &Residual) -> HashSet<EvaluationOutcome> {
            residual.possible_bool_outcomes().iter().cloned().collect()
        }

        /// Assert that `expr_str`, parsed and typechecked against the test
        /// schema, has exactly the given possible outcomes.
        // TODO(luxas): Use assert_snapshot instead.
        #[track_caller]
        fn assert_outcomes(expr_str: &str, expected: impl IntoIterator<Item = EvaluationOutcome>) {
            let residual = parse_residual(expr_str);
            let expected: HashSet<EvaluationOutcome> = expected.into_iter().collect();
            assert_eq!(
                outcomes_of(&residual),
                expected,
                "unexpected outcomes for `{}` (type {:?})",
                expr_str,
                residual.ty()
            );
        }

        // ------------------------------------------------------------------
        // The three `Residual` variants
        // ------------------------------------------------------------------

        #[test]
        fn concrete_booleans() {
            assert_outcomes("true", [T]);
            assert_outcomes("false", [F]);
        }

        /// A concrete non-boolean value fails `get_as_bool`, which the method
        /// reports as `Error` rather than as "this is not a boolean".
        #[test]
        fn concrete_non_booleans() {
            assert_outcomes("1", [E]);
            assert_outcomes(r#""foo""#, [E]);
            assert_outcomes("[1, 2]", [E]);
            assert_outcomes("{a: 1}", [E]);
            assert_outcomes(r#"User::"alice""#, [E]);
        }

        /// An `Error` residual always errors, whatever type it carries.
        #[test]
        fn error_residual() {
            for ty in [
                Type::Bool(BoolType::AnyBool),
                Type::Bool(BoolType::True),
                Type::Bool(BoolType::False),
                Type::Long,
                Type::Never,
            ] {
                assert_eq!(outcomes_of(&Residual::Error(ty)), HashSet::from([E]));
            }
        }

        /// Partial residuals that are not of boolean type also fail
        /// `get_as_bool`.
        #[test]
        fn partial_non_booleans() {
            assert_outcomes("principal", [E]);
            assert_outcomes("principal.num", [E]);
            assert_outcomes("principal.str", [E]);
            assert_outcomes("principal.set", [E]);
            assert_outcomes("-principal.num", [E]);
            assert_outcomes("principal.num + 1", [E]);
            assert_outcomes("[principal.str]", [E]);
            assert_outcomes("{a: principal.num}", [E]);
            assert_outcomes("if principal.foo then 1 else 2", [E]);
        }

        // ------------------------------------------------------------------
        // Leaves: every kind other than And/Or/If/Not falls back to "whatever
        // the type allows, plus `Error` if the node can error"
        // ------------------------------------------------------------------

        /// Boolean leaves that the validator cannot pin to a single value and
        /// that cannot error.
        #[test]
        fn leaves_without_errors() {
            // `has` on an entity missing from the store is `false`, not an
            // error, so `HasAttr` only errors if its subexpression can.
            assert_outcomes("principal has foo", [T, F]);
            assert_outcomes("principal has str", [T, F]);
            assert_outcomes(r#"principal.hasTag("t")"#, [T, F]);
            // `in` consults the entity hierarchy but never errors.
            assert_outcomes(r#"principal in Organization::"o""#, [T, F]);
            assert_outcomes(r#"resource in Organization::"o""#, [T, F]);
            assert_outcomes(
                r#"principal in [Organization::"o", Organization::"p"]"#,
                [T, F],
            );
            assert_outcomes(r#"[true, false].contains(true)"#, [T, F]);
            assert_outcomes(r#"[1, 2].containsAll([1])"#, [T, F]);
            assert_outcomes(r#"[1, 2].containsAny([1])"#, [T, F]);
        }

        /// Boolean leaves that can error at runtime.
        #[test]
        fn leaves_with_errors() {
            // `GetAttr` errors if the entity is missing from the store.
            assert_outcomes("principal.foo", [T, F, E]);
            // Arithmetic can overflow.
            assert_outcomes("principal.num + 1 == 2", [T, F, E]);
            assert_outcomes("principal.num - 1 == 2", [T, F, E]);
            assert_outcomes("principal.num * 2 == 2", [T, F, E]);
            assert_outcomes("-principal.num == 2", [T, F, E]);
            // `getTag` errors if the entity is missing from the store.
            assert_outcomes(
                r#"principal.hasTag("t") && principal.getTag("t") == "x""#,
                [T, F, E],
            );
            // Extension functions are assumed to be able to error.
            assert_outcomes("principal.period.toDays() == 1", [T, F, E]);
            // `like`/`isEmpty`/`contains` inherit their subexpression's errors.
            assert_outcomes(r#"principal.str like "a*""#, [T, F, E]);
            assert_outcomes("principal.set.isEmpty()", [T, F, E]);
            assert_outcomes(r#"principal.set.contains("a")"#, [T, F, E]);
            // Sets and records propagate their elements' errors.
            assert_outcomes("[principal.foo, true].contains(true)", [T, F, E]);
            assert_outcomes("{a: principal.foo}.a", [T, F, E]);
        }

        /// The validator refines expressions it can prove constant to the
        /// singleton boolean types `Bool(True)`/`Bool(False)`, which lets the
        /// method rule out the other value.
        #[test]
        fn singleton_boolean_types() {
            // `principal` is declared to be a `User` by the request environment.
            assert_outcomes("principal is User", [T]);
            assert_outcomes("principal is Document", [F]);
            assert_outcomes("resource is Document", [T]);
            assert_outcomes("resource is User", [F]);
            // The action is concrete in the request environment.
            assert_outcomes(r#"action == Action::"get""#, [T]);
            // A `User` can never be a descendant of a `Document`.
            assert_outcomes(r#"principal in Document::"d""#, [F]);
            assert_outcomes("principal == resource", [F]);
            // Literal comparisons are folded by the typechecker.
            assert_outcomes("1 == 1", [T]);
            assert_outcomes(r#""a" == "b""#, [F]);
        }

        /// A singleton boolean type rules out the other value but *not* an
        /// error: the expression may still fail before producing any value.
        #[test]
        fn singleton_boolean_types_can_still_error() {
            // `{a: true}` is a record literal, so the attribute's type is
            // `Bool(True)` -- but `GetAttr` is still treated as able to error.
            assert_outcomes("{a: true}.a", [T, E]);
            assert_outcomes("{a: false}.a", [F, E]);
        }

        // ------------------------------------------------------------------
        // Short-circuiting operators
        // ------------------------------------------------------------------

        #[test]
        fn and_short_circuits() {
            // A `false` left side means the right side is never evaluated, so
            // its outcomes -- errors included -- cannot occur.
            assert_outcomes("false && principal.foo", [F]);
            assert_outcomes("false && principal.num + 1 == 2", [F]);
            // A `true` left side hands the outcome to the right side.
            assert_outcomes("true && principal.foo", [T, F, E]);
            assert_outcomes("true && true", [T]);
            assert_outcomes("true && false", [F]);
            // The left side is always evaluated, so its errors always survive.
            assert_outcomes("principal.foo && true", [T, F, E]);
            assert_outcomes("principal.foo && false", [F, E]);
            // Both sides contribute.
            assert_outcomes(
                r#"principal in Organization::"o" && principal.foo"#,
                [T, F, E],
            );
            assert_outcomes(
                r#"principal.foo && principal in Organization::"o""#,
                [T, F, E],
            );
            // A left side that is statically `true` is transparent.
            assert_outcomes("principal is User && principal.foo", [T, F, E]);
            // A left side that is statically `false` prunes the right side,
            // even when that right side can error.
            assert_outcomes("principal is Document && principal.foo", [F]);
        }

        #[test]
        fn or_short_circuits() {
            // A `true` left side means the right side is never evaluated.
            assert_outcomes("true || principal.foo", [T]);
            assert_outcomes("true || principal.num + 1 == 2", [T]);
            // A `false` left side hands the outcome to the right side.
            assert_outcomes("false || principal.foo", [T, F, E]);
            assert_outcomes("false || true", [T]);
            assert_outcomes("false || false", [F]);
            // The left side is always evaluated, so its errors always survive.
            assert_outcomes("principal.foo || true", [T, E]);
            assert_outcomes("principal.foo || false", [T, F, E]);
            // A left side that is statically `true` prunes the right side,
            // even when that right side can error.
            assert_outcomes("resource is Document || principal.foo", [T]);
            // A left side that is statically `false` is transparent.
            assert_outcomes("principal is Document || principal.foo", [T, F, E]);
        }

        #[test]
        fn if_takes_only_the_reachable_branches() {
            // A constant test prunes the other branch entirely.
            assert_outcomes("if true then false else principal.foo", [F]);
            assert_outcomes("if false then principal.foo else true", [T]);
            assert_outcomes("if true then principal.foo else true", [T, F, E]);
            assert_outcomes("if principal is Document then principal.foo else true", [T]);
            // A test that cannot error but can go either way reaches both
            // branches -- and contributes neither `True` nor `False` itself.
            assert_outcomes(
                r#"if principal in Organization::"o" then true else true"#,
                [T],
            );
            assert_outcomes(
                r#"if principal in Organization::"o" then false else false"#,
                [F],
            );
            assert_outcomes(
                r#"if principal in Organization::"o" then true else false"#,
                [T, F],
            );
            assert_outcomes(
                r#"if principal in Organization::"o" then principal.foo else false"#,
                [T, F, E],
            );
            // A test that can error contributes `Error`.
            assert_outcomes("if principal.foo then true else true", [T, E]);
            assert_outcomes("if principal.foo then false else false", [F, E]);
            assert_outcomes("if principal.foo then true else false", [T, F, E]);
        }

        #[test]
        fn not_negates_values_but_not_errors() {
            assert_outcomes("!true", [F]);
            assert_outcomes("!false", [T]);
            assert_outcomes("!principal.foo", [T, F, E]);
            assert_outcomes("!(principal is User)", [F]);
            assert_outcomes("!(principal is Document)", [T]);
            assert_outcomes("!(if principal.foo then true else true)", [F, E]);
            assert_outcomes("!(if principal.foo then false else false)", [T, E]);
            assert_outcomes("!!(if principal.foo then false else false)", [F, E]);
        }

        // ------------------------------------------------------------------
        // Realistic policy and TPE-output shapes
        // ------------------------------------------------------------------

        #[test]
        fn policy_scope_never_errors() {
            assert_outcomes(
                r#"
                principal is User &&
                principal in Organization::"foo" &&
                action == Action::"get" &&
                resource is Document &&
                resource in Organization::"foo"
                "#,
                [T, F],
            );
        }

        /// The `has`/`hasTag` guard idioms still report a possible error,
        /// because `can_error_assuming_well_formed` does not track that the
        /// guard rules the error out.
        #[test]
        fn guarded_access_is_over_approximated() {
            assert_outcomes("principal has foo && principal.foo", [T, F, E]);
            assert_outcomes(
                r#"principal.hasTag("t") && principal.getTag("t") == "x""#,
                [T, F, E],
            );
        }

        #[test]
        fn nested_expressions() {
            // `resource is Document` is statically `true`, so the `||` can only
            // be `True` or `Error`, and `principal is User` is transparent.
            assert_outcomes(
                r#"principal is User && (principal.foo || resource is Document)"#,
                [T, E],
            );
            assert_outcomes(
                r#"principal in Organization::"o" && (resource is Document || principal.foo)"#,
                [T, F],
            );
            assert_outcomes(
                r#"if principal is User then (if principal is Document then principal.foo else true) else principal.foo"#,
                [T],
            );
        }

        /// `Residual`s are also built programmatically, most notably by the TPE
        /// evaluator, which produces shapes the parser and typechecker would
        /// have folded away. These are the interesting ones.
        /// TODO(luxas): These are the exact cases we'd like to eliminate, to force Residuals
        /// to be build from only validated Exprs.
        mod hand_built {
            use super::*;
            use similar_asserts::assert_eq;

            fn concrete(b: bool) -> Residual {
                Residual::Concrete {
                    value: b.into(),
                    ty: Type::singleton_boolean(b),
                }
            }

            fn error() -> Residual {
                Residual::Error(Type::primitive_boolean())
            }

            fn partial(kind: ResidualKind) -> Residual {
                Residual::Partial {
                    kind,
                    ty: Type::primitive_boolean(),
                }
            }

            fn and(left: Residual, right: Residual) -> Residual {
                partial(ResidualKind::And {
                    left: Arc::new(left),
                    right: Arc::new(right),
                })
            }

            fn or(left: Residual, right: Residual) -> Residual {
                partial(ResidualKind::Or {
                    left: Arc::new(left),
                    right: Arc::new(right),
                })
            }

            /// A leaf whose possible outcomes are exactly `{True, False, Error}`.
            fn tfe() -> Residual {
                parse_residual("principal.foo")
            }

            /// The TPE evaluator rewrites `<residual> && <non-boolean>` into
            /// `<residual> && <error>`; the `&&` can then still be `False`.
            #[test]
            fn and_with_error_right() {
                assert_eq!(outcomes_of(&and(tfe(), error())), HashSet::from([F, E]));
            }

            /// Likewise `<residual> || <non-boolean>` becomes
            /// `<residual> || <error>`, which can still be `True`.
            #[test]
            fn or_with_error_right() {
                assert_eq!(outcomes_of(&or(tfe(), error())), HashSet::from([T, E]));
            }

            /// The TPE evaluator keeps `<residual> && false` (and
            /// `<residual> || true`) unfolded when the left side can error.
            #[test]
            fn and_false_right_or_true_right() {
                assert_eq!(
                    outcomes_of(&and(tfe(), concrete(false))),
                    HashSet::from([F, E])
                );
                assert_eq!(
                    outcomes_of(&or(tfe(), concrete(true))),
                    HashSet::from([T, E])
                );
            }

            /// An erroring left side is never rescued by the right side.
            #[test]
            fn error_left() {
                assert_eq!(outcomes_of(&and(error(), tfe())), HashSet::from([E]));
                assert_eq!(outcomes_of(&or(error(), tfe())), HashSet::from([E]));
                assert_eq!(
                    outcomes_of(&and(error(), concrete(true))),
                    HashSet::from([E])
                );
            }

            /// A non-boolean left side errors before the right side is reached.
            #[test]
            fn non_boolean_left() {
                let long = Residual::Concrete {
                    value: 1i64.into(),
                    ty: Type::primitive_long(),
                };
                assert_eq!(outcomes_of(&and(long.clone(), tfe())), HashSet::from([E]));
                assert_eq!(outcomes_of(&or(long, tfe())), HashSet::from([E]));
            }

            /// `possible_bool_outcomes` strictly refines
            /// `can_error_assuming_well_formed`: the latter is purely
            /// structural, so it reports an error for `false && <can error>`
            /// even though the right side is never evaluated.
            #[test]
            fn strictly_refines_can_error() {
                let residual = and(concrete(false), tfe());
                assert!(residual.can_error_assuming_well_formed());
                assert_eq!(outcomes_of(&residual), HashSet::from([F]));

                let residual = or(concrete(true), tfe());
                assert!(residual.can_error_assuming_well_formed());
                assert_eq!(outcomes_of(&residual), HashSet::from([T]));
            }
        }

        // ------------------------------------------------------------------
        // Known imprecision
        // ------------------------------------------------------------------

        /// The method treats each leaf occurrence as independent, so it cannot
        /// see that two occurrences of the same subexpression agree. These
        /// answers are sound but not tight.
        #[test]
        fn correlated_leaves_are_not_tracked() {
            // Never actually `True`.
            assert_outcomes("principal.foo && !principal.foo", [T, F, E]);
            // Never actually `False`.
            assert_outcomes("principal.foo || !principal.foo", [T, F, E]);
            // The guard makes the `GetAttr` safe, but `can_error_assuming_well_formed`
            // is purely structural and does not know that.
            // TODO(luxas): This is wrong, the error from the RHS comes from principal entity being missing.
            assert_outcomes("principal has foo && principal.foo", [T, F, E]);
        }

        // ------------------------------------------------------------------
        // Cross-check against `can_error_assuming_well_formed`
        // ------------------------------------------------------------------

        /// For boolean-typed residuals, a possible `Error` outcome always
        /// implies that the structural [`Residual::can_error_assuming_well_formed`]
        /// agrees. (The converse does not hold; see
        /// [`hand_built::strictly_refines_can_error`].)
        #[test]
        fn error_outcome_implies_can_error() {
            for expr_str in EXPRS {
                let residual = parse_residual(expr_str);
                if !matches!(residual.ty(), Type::Bool(_)) {
                    continue;
                }
                assert!(
                    !residual.possible_bool_outcomes().contains(&E)
                        || residual.can_error_assuming_well_formed(),
                    "`{}` reports a possible error that `can_error_assuming_well_formed` denies",
                    expr_str
                );
            }
        }

        /// Boolean expressions covering every `ResidualKind`, reused by
        /// [`error_outcome_implies_can_error`] and [`concrete_evaluator`].
        const EXPRS: &[&str] = &[
            "true",
            "false",
            "principal is User",
            "principal is Document",
            "resource is Document",
            r#"action == Action::"get""#,
            "principal == resource",
            "principal has foo",
            "principal has str",
            "principal.foo",
            r#"principal.hasTag("t")"#,
            r#"principal.hasTag("t") && principal.getTag("t") == "x""#,
            r#"principal in Organization::"o""#,
            r#"resource in Organization::"o""#,
            r#"principal in [Organization::"o", Organization::"p"]"#,
            r#"principal in Document::"d""#,
            r#"principal.str like "a*""#,
            r#"principal.str like "z*""#,
            "principal.num > 0",
            "principal.num + 1 > 0",
            "principal.num - 1 > 0",
            "principal.num * 2 > 0",
            "-principal.num > 0",
            "principal.set.isEmpty()",
            r#"principal.set.contains("a")"#,
            r#"principal.set.containsAll(["a"])"#,
            r#"principal.set.containsAny(["a", "z"])"#,
            "principal.period.toDays() == 1",
            "[principal.foo, true].contains(true)",
            "{a: principal.foo}.a",
            "{a: true}.a",
            "!principal.foo",
            "!(principal is User)",
            "false && principal.foo",
            "true && principal.foo",
            "principal.foo && true",
            "principal.foo && false",
            "principal is Document && principal.foo",
            "principal is User && principal.foo",
            "true || principal.foo",
            "false || principal.foo",
            "principal.foo || true",
            "principal.foo || false",
            "resource is Document || principal.foo",
            "principal is Document || principal.foo",
            "if principal.foo then true else true",
            "if principal.foo then false else false",
            "if principal.foo then true else false",
            r#"if principal in Organization::"o" then true else true"#,
            r#"if principal in Organization::"o" then principal.foo else false"#,
            "if true then principal.foo else true",
            "if false then principal.foo else true",
            "principal has foo && principal.foo",
            r#"principal is User && principal in Organization::"foo" && action == Action::"get" && resource is Document && resource in Organization::"foo""#,
            r#"principal is User && (principal.foo || resource is Document)"#,
            r#"principal in Organization::"o" && (resource is Document || principal.foo)"#,
            "principal.foo && !principal.foo",
            "principal.foo || !principal.foo",
        ];

        /// Checks `possible_bool_outcomes` against a brute-force reference
        /// semantics.
        ///
        /// Each residual is built from leaves whose own outcome sets are known
        /// and asserted up front. The reference then enumerates every
        /// combination of leaf outcomes, evaluates the tree under Cedar's
        /// short-circuiting rules, and collects the results. The method must
        /// return *exactly* that set: a missing outcome is unsoundness, an
        /// extra one is incompleteness.
        ///
        /// The reference treats each leaf *occurrence* as an independent
        /// choice, which is the same assumption the method makes -- it does no
        /// correlation reasoning. That assumption only ever adds behaviors, so
        /// agreeing with the reference implies soundness with respect to real
        /// Cedar evaluation as well.
        mod model_check {
            use super::*;
            use similar_asserts::assert_eq;

            /// A residual shape over the leaf table in [`leaf_table`].
            #[derive(Clone, Debug)]
            enum Shape {
                Leaf(usize),
                Not(Box<Shape>),
                And(Box<Shape>, Box<Shape>),
                Or(Box<Shape>, Box<Shape>),
                If(Box<Shape>, Box<Shape>, Box<Shape>),
            }

            /// The same shape with each leaf occurrence given a position, so
            /// that a single assignment vector can drive the reference
            /// semantics regardless of which branches short-circuit away.
            #[derive(Debug)]
            enum Node {
                Leaf { pos: usize },
                Not(Box<Node>),
                And(Box<Node>, Box<Node>),
                Or(Box<Node>, Box<Node>),
                If(Box<Node>, Box<Node>, Box<Node>),
            }

            /// One residual realizing each non-empty subset of the three
            /// outcomes, paired with that subset.
            fn leaf_table() -> Vec<(Residual, Vec<EvaluationOutcome>)> {
                vec![
                    (
                        Residual::Concrete {
                            value: true.into(),
                            ty: Type::singleton_boolean(true),
                        },
                        vec![T],
                    ),
                    (
                        Residual::Concrete {
                            value: false.into(),
                            ty: Type::singleton_boolean(false),
                        },
                        vec![F],
                    ),
                    (Residual::Error(Type::primitive_boolean()), vec![E]),
                    (
                        parse_residual(r#"resource in Organization::"o""#),
                        vec![T, F],
                    ),
                    (
                        parse_residual("if principal.foo then true else true"),
                        vec![T, E],
                    ),
                    (
                        parse_residual("if principal.foo then false else false"),
                        vec![F, E],
                    ),
                    (parse_residual("principal.foo"), vec![T, F, E]),
                ]
            }

            /// Build the residual for a shape. The `ty` of the compound nodes
            /// is irrelevant -- `possible_bool_outcomes` derives their outcomes
            /// from their children, not from their type -- so a plain `Bool` is
            /// used throughout.
            fn to_residual(
                shape: &Shape,
                table: &[(Residual, Vec<EvaluationOutcome>)],
            ) -> Residual {
                let partial = |kind| Residual::Partial {
                    kind,
                    ty: Type::primitive_boolean(),
                };
                match shape {
                    Shape::Leaf(i) => table[*i].0.clone(),
                    Shape::Not(a) => partial(ResidualKind::UnaryApp {
                        op: ast::UnaryOp::Not,
                        arg: Arc::new(to_residual(a, table)),
                    }),
                    Shape::And(a, b) => partial(ResidualKind::And {
                        left: Arc::new(to_residual(a, table)),
                        right: Arc::new(to_residual(b, table)),
                    }),
                    Shape::Or(a, b) => partial(ResidualKind::Or {
                        left: Arc::new(to_residual(a, table)),
                        right: Arc::new(to_residual(b, table)),
                    }),
                    Shape::If(t, x, y) => partial(ResidualKind::If {
                        test_expr: Arc::new(to_residual(t, table)),
                        then_expr: Arc::new(to_residual(x, table)),
                        else_expr: Arc::new(to_residual(y, table)),
                    }),
                }
            }

            /// Number the leaf occurrences left to right, recording each
            /// occurrence's set of possible outcomes in `leaf_sets`.
            fn index(
                shape: &Shape,
                table: &[(Residual, Vec<EvaluationOutcome>)],
                leaf_sets: &mut Vec<Vec<EvaluationOutcome>>,
            ) -> Node {
                match shape {
                    Shape::Leaf(i) => {
                        let pos = leaf_sets.len();
                        leaf_sets.push(table[*i].1.clone());
                        Node::Leaf { pos }
                    }
                    Shape::Not(a) => Node::Not(Box::new(index(a, table, leaf_sets))),
                    Shape::And(a, b) => Node::And(
                        Box::new(index(a, table, leaf_sets)),
                        Box::new(index(b, table, leaf_sets)),
                    ),
                    Shape::Or(a, b) => Node::Or(
                        Box::new(index(a, table, leaf_sets)),
                        Box::new(index(b, table, leaf_sets)),
                    ),
                    Shape::If(t, x, y) => Node::If(
                        Box::new(index(t, table, leaf_sets)),
                        Box::new(index(x, table, leaf_sets)),
                        Box::new(index(y, table, leaf_sets)),
                    ),
                }
            }

            /// Cedar's short-circuiting semantics, written out directly. This
            /// is the reference the method is checked against, so it
            /// deliberately shares no code with it.
            fn eval(node: &Node, assignment: &[EvaluationOutcome]) -> EvaluationOutcome {
                match node {
                    Node::Leaf { pos } => assignment[*pos].clone(),
                    Node::Not(a) => match eval(a, assignment) {
                        T => F,
                        F => T,
                        E => E,
                    },
                    // `&&` evaluates its right side only when the left is true.
                    Node::And(a, b) => match eval(a, assignment) {
                        T => eval(b, assignment),
                        F => F,
                        E => E,
                    },
                    // `||` evaluates its right side only when the left is false.
                    Node::Or(a, b) => match eval(a, assignment) {
                        T => T,
                        F => eval(b, assignment),
                        E => E,
                    },
                    // `if` evaluates exactly one branch.
                    Node::If(t, x, y) => match eval(t, assignment) {
                        T => eval(x, assignment),
                        F => eval(y, assignment),
                        E => E,
                    },
                }
            }

            /// Every outcome reachable by some assignment of outcomes to leaves.
            fn exact_outcomes(
                node: &Node,
                leaf_sets: &[Vec<EvaluationOutcome>],
            ) -> HashSet<EvaluationOutcome> {
                let mut outcomes = HashSet::new();
                // Odometer over the product of the leaves' outcome sets.
                let mut counter = vec![0usize; leaf_sets.len()];
                loop {
                    let assignment: Vec<EvaluationOutcome> = counter
                        .iter()
                        .zip(leaf_sets)
                        .map(|(i, set)| set[*i].clone())
                        .collect();
                    outcomes.insert(eval(node, &assignment));

                    let mut carry = 0;
                    while carry < counter.len() {
                        counter[carry] += 1;
                        if counter[carry] < leaf_sets[carry].len() {
                            break;
                        }
                        counter[carry] = 0;
                        carry += 1;
                    }
                    if carry == counter.len() {
                        return outcomes;
                    }
                }
            }

            #[track_caller]
            fn check(shape: &Shape, table: &[(Residual, Vec<EvaluationOutcome>)]) {
                let mut leaf_sets = Vec::new();
                let node = index(shape, table, &mut leaf_sets);
                let expected = exact_outcomes(&node, &leaf_sets);
                let actual = outcomes_of(&to_residual(shape, table));
                assert_eq!(actual, expected, "mismatch for shape {:?}", shape);
            }

            /// The leaf table must actually realize the outcome sets it claims,
            /// since everything else is checked relative to it.
            #[test]
            fn leaf_table_is_accurate() {
                for (residual, expected) in leaf_table() {
                    let expected: HashSet<EvaluationOutcome> = expected.into_iter().collect();
                    assert_eq!(
                        outcomes_of(&residual),
                        expected,
                        "leaf table entry is wrong for {:?}",
                        residual
                    );
                }
            }

            /// Every shape of depth at most one over the whole leaf table.
            #[test]
            fn exhaustive_depth_one() {
                let table = leaf_table();
                let n = table.len();
                let leaf = |i: usize| Box::new(Shape::Leaf(i));

                for i in 0..n {
                    check(&Shape::Leaf(i), &table);
                    check(&Shape::Not(leaf(i)), &table);
                    for j in 0..n {
                        check(&Shape::And(leaf(i), leaf(j)), &table);
                        check(&Shape::Or(leaf(i), leaf(j)), &table);
                        for k in 0..n {
                            check(&Shape::If(leaf(i), leaf(j), leaf(k)), &table);
                        }
                    }
                }
            }

            /// A deterministic xorshift generator, so failures reproduce.
            struct Rng(u64);

            impl Rng {
                fn next(&mut self) -> u64 {
                    self.0 ^= self.0 << 13;
                    self.0 ^= self.0 >> 7;
                    self.0 ^= self.0 << 17;
                    self.0
                }

                fn below(&mut self, n: usize) -> usize {
                    (self.next() % (n as u64)) as usize
                }
            }

            /// `budget` bounds the number of leaf occurrences, which bounds the
            /// size of the assignment space the reference has to enumerate.
            fn gen(rng: &mut Rng, depth: usize, budget: usize, n_leaves: usize) -> Shape {
                if depth == 0 || budget <= 1 {
                    return Shape::Leaf(rng.below(n_leaves));
                }
                match rng.below(6) {
                    0 | 1 => Shape::Leaf(rng.below(n_leaves)),
                    2 => Shape::Not(Box::new(gen(rng, depth - 1, budget, n_leaves))),
                    3 => Shape::And(
                        Box::new(gen(rng, depth - 1, budget / 2, n_leaves)),
                        Box::new(gen(rng, depth - 1, budget - budget / 2, n_leaves)),
                    ),
                    4 => Shape::Or(
                        Box::new(gen(rng, depth - 1, budget / 2, n_leaves)),
                        Box::new(gen(rng, depth - 1, budget - budget / 2, n_leaves)),
                    ),
                    _ => Shape::If(
                        Box::new(gen(rng, depth - 1, budget / 3, n_leaves)),
                        Box::new(gen(rng, depth - 1, budget / 3, n_leaves)),
                        Box::new(gen(rng, depth - 1, budget - 2 * (budget / 3), n_leaves)),
                    ),
                }
            }

            /// Randomly generated deeper shapes, to reach nestings the
            /// exhaustive pass cannot.
            #[test]
            fn randomized_deep_shapes() {
                let table = leaf_table();
                let mut rng = Rng(0x2545_F491_4F6C_DD1D);
                for _ in 0..5000 {
                    check(&gen(&mut rng, 4, 6, table.len()), &table);
                }
            }
        }

        /// Checks the predictions against what the real Cedar evaluator does.
        ///
        /// [`model_check`] verifies the short-circuiting logic against a
        /// reference semantics, but takes the leaves' outcome sets as given.
        /// This test closes that gap from the other end: it converts each
        /// residual back to an [`Expr`], evaluates it concretely against
        /// several entity stores, and requires the observed outcome to be one
        /// the method predicted. Any leaf approximation that is too narrow --
        /// an operation that can error where `can_error_assuming_well_formed`
        /// says it cannot, or a singleton boolean type that does not hold --
        /// shows up here as a failure.
        /// TODO(luxas): Put this into the DRT infrastructure as well, always ensure this function agrees with what then happens.
        mod concrete_evaluator {
            use super::*;
            use crate::ast::{Context, Request, RequestSchemaAllPass};
            use crate::entities::{Entities, EntityJsonParser, TCComputation};
            use crate::evaluator::Evaluator;
            use crate::extensions::Extensions;
            use similar_asserts::assert_eq;

            fn request() -> Request {
                Request::new(
                    (r#"User::"alice""#.parse().unwrap(), None),
                    (r#"Action::"get""#.parse().unwrap(), None),
                    (r#"Document::"doc""#.parse().unwrap(), None),
                    Context::empty(),
                    Some(&RequestSchemaAllPass),
                    Extensions::all_available(),
                )
                .unwrap()
            }

            fn entities(json: serde_json::Value) -> Entities {
                let parser: EntityJsonParser<'_, '_> = EntityJsonParser::new(
                    None,
                    Extensions::all_available(),
                    TCComputation::ComputeNow,
                );
                parser
                    .from_json_value(json)
                    .unwrap_or_else(|e| panic!("{:?}", miette::Report::new(e)))
            }

            /// Entity stores chosen to drive each expression down a different
            /// path: attributes present and absent, `alice` in and out of the
            /// organization, tags present and absent, entities missing from the
            /// store entirely, and values that make arithmetic overflow.
            fn stores() -> Vec<(&'static str, Entities)> {
                vec![
                    (
                        "complete",
                        entities(serde_json::json!([
                            {
                                "uid": { "type": "User", "id": "alice" },
                                "attrs": {
                                    "foo": true,
                                    "str": "abc",
                                    "num": 1,
                                    "period": { "__extn": { "fn": "duration", "arg": "1d" } },
                                    "set": ["a"],
                                },
                                "parents": [{ "type": "Organization", "id": "o" }],
                                "tags": { "t": "x" },
                            },
                            {
                                "uid": { "type": "Document", "id": "doc" },
                                "attrs": {},
                                "parents": [{ "type": "Organization", "id": "o" }],
                            },
                            { "uid": { "type": "Organization", "id": "o" }, "attrs": {}, "parents": [] },
                        ])),
                    ),
                    (
                        "other values",
                        entities(serde_json::json!([
                            {
                                "uid": { "type": "User", "id": "alice" },
                                "attrs": {
                                    "foo": false,
                                    "str": "zzz",
                                    "num": 0,
                                    "period": { "__extn": { "fn": "duration", "arg": "2d" } },
                                    "set": [],
                                },
                                "parents": [],
                                "tags": {},
                            },
                            { "uid": { "type": "Document", "id": "doc" }, "attrs": {}, "parents": [] },
                        ])),
                    ),
                    (
                        "overflowing num",
                        entities(serde_json::json!([
                            {
                                "uid": { "type": "User", "id": "alice" },
                                "attrs": {
                                    "foo": true,
                                    "str": "abc",
                                    "num": i64::MAX,
                                    "period": { "__extn": { "fn": "duration", "arg": "1d" } },
                                    "set": ["a"],
                                },
                                "parents": [{ "type": "Organization", "id": "o" }],
                                "tags": { "t": "y" },
                            },
                            { "uid": { "type": "Organization", "id": "o" }, "attrs": {}, "parents": [] },
                        ])),
                    ),
                    // Nothing is in the store, so every attribute and tag
                    // access errors.
                    ("empty", entities(serde_json::json!([]))),
                ]
            }

            /// Evaluate `expr` and classify the result the same way
            /// `possible_bool_outcomes` describes: evaluate, then
            /// `get_as_bool`.
            fn observe(evaluator: &Evaluator<'_>, expr: &Expr) -> EvaluationOutcome {
                match evaluator.interpret_inline_policy(expr) {
                    Ok(value) => match value.get_as_bool() {
                        Ok(true) => T,
                        Ok(false) => F,
                        Err(_) => E,
                    },
                    Err(_) => E,
                }
            }

            #[test]
            fn predictions_cover_what_the_evaluator_produces() {
                let request = request();
                let stores = stores();
                let mut witnessed_overall = HashSet::new();

                for expr_str in EXPRS {
                    let residual = parse_residual(expr_str);
                    let predicted = outcomes_of(&residual);
                    let expr = Expr::from(residual);

                    for (store_name, store) in &stores {
                        let evaluator =
                            Evaluator::new(request.clone(), store, Extensions::all_available());
                        let observed = observe(&evaluator, &expr);
                        assert!(
                            predicted.contains(&observed),
                            "`{}` evaluated to {:?} against the `{}` entity store, \
                             but `possible_bool_outcomes` predicted only {:?}",
                            expr_str,
                            observed,
                            store_name,
                            predicted,
                        );
                        witnessed_overall.insert(observed);
                    }
                }

                // Sanity check that the stores actually exercise all three
                // outcomes, rather than the assertion above passing vacuously.
                assert_eq!(witnessed_overall, HashSet::from([T, F, E]));
            }
        }
    }

    fn assert_eq_expr(expr_str: &str) {
        let e: Expr = expr_str.parse().unwrap();
        let residual = parse_residual(expr_str);
        let e2 = Expr::from(residual);
        println!("e: {}", e);
        println!("e2: {}", e2);
        assert_eq!(e, e2);
    }

    #[test]
    fn to_expr() {
        assert_eq_expr(r#"User::"alice".foo && User::"jane".foo"#);
        assert_eq_expr(r#"User::"alice".foo || User::"jane".foo"#);
        assert_eq_expr(r#"[User::"jane".foo].contains(User::"jane".foo)"#);
        assert_eq_expr(r#"User::"alice" has foo"#);
        assert_eq_expr(r#"(if User::"alice".foo then User::"bob" else User::"jane").foo"#);
        assert_eq_expr(r#""foo" like "bar""#);
        assert_eq_expr(r#"principal in [User::"alice", User::"jane"]"#);
        assert_eq_expr(
            r#"(if principal.foo then {a: User::"alice", b: true} else {a: User::"jane", b: false}).a.foo"#,
        );
    }
}
