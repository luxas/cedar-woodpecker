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

//! Typechecking a bare expression, and compiling it into a tree of terms that
//! mirrors its boolean structure.

use std::collections::BTreeSet;
use std::sync::Arc;

use cedar_policy::{RequestEnv, Schema};
use cedar_policy_core::ast::{BinaryOp, Expr, ExprKind, Literal, UnaryOp};
use cedar_policy_core::extensions::iferror::is_iferror;
use cedar_policy_core::tpe::residual::EvaluationOutcome;
use cedar_policy_core::typechecked::{TypecheckError, Typecheckable};
use cedar_policy_core::validator::types::Type;

use super::err::{EvaluationError, Result};
use crate::symcc::env::to_validator_request_env;
use crate::symcc::term::{Term, TermPrim};
use crate::symcc::term_type::TermType;
use crate::symcc::type_abbrevs::EntityType;
use crate::symcc::CompileError;
use crate::symccopt::compiler::{
    compile, compile_and, compile_if, compile_not, compile_or, CompileResult, Footprint,
};
use crate::SymEnv;

/// The typed form of `expr` in `req_env` (strict mode), as
/// [`Typecheckable::typechecked`] gives it: for a bare [`Expr`] the
/// typechecker is run (core wraps `expr` in a `permit` policy whose condition
/// is `true && (true && (true && expr))` and returns that condition, typed;
/// the evaluator's own `true && x` rule removes the wrapper again); for a
/// [`cedar_policy::TypecheckedPolicy`] on the schema it was built with, the
/// cached typed condition.
///
/// The typechecker may rewrite the expression (`false && x` becomes `false`,
/// `if c then a else b` with `c` provably true becomes `if c then a else a`);
/// the result is semantically equivalent to `expr` but not necessarily
/// structurally identical.
pub(super) fn typecheck<T: Typecheckable + ?Sized>(
    expr: &T,
    req_env: &RequestEnv,
    schema: &Schema,
) -> Result<Arc<Expr<Option<Type>>>> {
    let validator_env = to_validator_request_env(req_env, schema.as_ref())
        .ok_or_else(|| EvaluationError::RequestEnvNotFound(req_env.clone()))?;
    expr.typechecked(&validator_env, schema.as_ref())
        .map_err(|e| match e {
            TypecheckError::NotWellTyped { errs } => EvaluationError::NotWellTyped { errs },
            TypecheckError::MissingType | TypecheckError::UnboundSlot(_) => {
                EvaluationError::NotWellTyped { errs: vec![] }
            }
        })
}

/// Like [`typecheck`], but reports typechecking failures as
/// [`EvaluationError::AssumptionNotWellTyped`].
pub(super) fn typecheck_assumption(
    expr: &Expr,
    req_env: &RequestEnv,
    schema: &Schema,
) -> Result<Arc<Expr<Option<Type>>>> {
    typecheck(expr, req_env, schema).map_err(|e| match e {
        EvaluationError::NotWellTyped { errs } => EvaluationError::AssumptionNotWellTyped {
            env: req_env.clone(),
            errs,
        },
        other => other,
    })
}

/// Requires the root of a typed expression to be boolean.
pub(super) fn require_boolean(typed: &Expr<Option<Type>>) -> Result<()> {
    match typed.data() {
        Some(Type::Bool(_)) => Ok(()),
        Some(ty) => Err(EvaluationError::NotBoolean { ty: ty.clone() }),
        None => Err(EvaluationError::NotWellTyped { errs: vec![] }),
    }
}

/// [`crate::expr_util::with_data`], with its errors mapped into
/// [`EvaluationError`].
pub(super) fn with_data<T: Clone, U: Clone + Default>(expr: &Expr<T>, data: U) -> Result<Expr<U>> {
    crate::expr_util::with_data(expr, data).map_err(rebuild_err)
}

/// [`crate::expr_util::erase`], with its errors mapped into
/// [`EvaluationError`].
pub(super) fn erase<T: Clone>(expr: &Expr<T>) -> Result<Expr<()>> {
    crate::expr_util::erase(expr).map_err(rebuild_err)
}

pub(super) fn rebuild_err(e: crate::expr_util::RebuildError) -> EvaluationError {
    match e {
        crate::expr_util::RebuildError::Unsupported(what) => EvaluationError::Internal(what),
        crate::expr_util::RebuildError::RecursionLimit => EvaluationError::RecursionLimit,
    }
}

/// The boolean structure of an expression, with every node compiled to a term.
pub(super) struct Node {
    /// The typed expression this node was built from.
    pub expr: Arc<Expr<Option<Type>>>,
    /// The term for the whole sub-expression, of type `Option<Bool>`.
    pub term: Term,
    /// What kind of node this is.
    pub kind: NodeKind,
    /// The existence facts that hold when this node evaluates to `true`,
    /// to `false`, or to either.
    pub facts: Facts,
}

/// A node's existence facts by what it evaluated to.
#[derive(Debug, Clone, Default)]
pub(super) struct Facts {
    on_true: Vec<Existence>,
    on_false: Vec<Existence>,
    any: Vec<Existence>,
}

impl Facts {
    /// The facts when the node evaluated to `truth` (`None`: to either).
    pub fn get(&self, truth: Option<bool>) -> &[Existence] {
        match truth {
            Some(true) => &self.on_true,
            Some(false) => &self.on_false,
            None => &self.any,
        }
    }
}

/// [`Node::facts`] of a structure node from its children's: an `&&` that is
/// true ran both operands to `true`, an `||` that is false both to `false`;
/// otherwise only the left operand (or the `if` test) is known to have run.
fn structure_facts(kind: &NodeKind) -> Facts {
    let of = |n: &Node, truth: Option<bool>| n.facts.get(truth).to_vec();
    let both = |l: &Node, r: &Option<Box<Node>>, b: bool| {
        let mut v = of(l, Some(b));
        if let Some(r) = r {
            v.extend(of(r, Some(b)));
        }
        v
    };
    match kind {
        NodeKind::And(l, r) => Facts {
            on_true: both(l, r, true),
            on_false: of(l, None),
            any: of(l, None),
        },
        NodeKind::Or(l, r) => Facts {
            on_true: of(l, None),
            on_false: both(l, r, false),
            any: of(l, None),
        },
        NodeKind::Not(c) => Facts {
            on_true: of(c, Some(false)),
            on_false: of(c, Some(true)),
            any: of(c, None),
        },
        NodeKind::If(c, _, _) => Facts {
            on_true: of(c, None),
            on_false: of(c, None),
            any: of(c, None),
        },
        // an atom computes its own facts (see `build_tree`)
        NodeKind::Atom { .. } => Facts::default(),
    }
}

/// Node kinds. A child that is `None` was never compiled because the sibling
/// on its left compiled to a literal that makes it dead (`compile_and` and
/// friends do not look at the right operand in that case); the evaluator
/// never visits such a child either.
pub(super) enum NodeKind {
    And(Box<Node>, Option<Box<Node>>),
    Or(Box<Node>, Option<Box<Node>>),
    If(Box<Node>, Option<Box<Node>>, Option<Box<Node>>),
    Not(Box<Node>),
    /// Anything that is not boolean structure. Compiled as a whole, with its
    /// existence-sensitive accesses in evaluation order. `keep` marks a
    /// `has`/`hasTag` guard the evaluator never folds to `true` (see
    /// [`kept_guards`]).
    Atom {
        checks: Vec<Existence>,
        keep: bool,
    },
}

/// The access a `has`/`hasTag` guard `g` makes safe: `<receiver>.<attr>` /
/// `<receiver>.getTag(<tag>)`, types erased for the structural comparison.
fn guard_access<T: Clone>(g: &Expr<T>) -> Option<Expr<()>> {
    match g.expr_kind() {
        ExprKind::HasAttr { expr, attr } => Some(Expr::get_attr(erase(expr).ok()?, attr.clone())),
        ExprKind::BinaryApp {
            op: BinaryOp::HasTag,
            arg1,
            arg2,
        } => Some(Expr::get_tag(erase(arg1).ok()?, erase(arg2).ok()?)),
        _ => None,
    }
}

/// Whether the guard `g` is needed for its access to validate: a `has` of an
/// attribute the schema declares optional on the receiver's type (entity
/// attributes from the symbolic entities, record attributes from the
/// receiver's term type — an `Option`-typed field either way), or any
/// `hasTag` (every `getTag` needs its guard).
fn optional_guard(g: &Expr<Option<Type>>, sym_env: &SymEnv) -> bool {
    match g.expr_kind() {
        ExprKind::HasAttr { expr, attr } => {
            let Ok(erased) = erase(expr) else {
                return false;
            };
            let Ok(res) = compile(&erased, sym_env) else {
                return false;
            };
            let TermType::Option { ty } = res.term.type_of() else {
                return false;
            };
            let rty = match &*ty {
                TermType::Entity { ety } => match sym_env.entities.attrs(ety).map(|f| f.out_type())
                {
                    Some(TermType::Record { rty }) => rty.clone(),
                    _ => return false,
                },
                TermType::Record { rty } => rty.clone(),
                _ => return false,
            };
            matches!(rty.get(attr.as_str()), Some(ty) if ty.is_option_type())
        }
        ExprKind::BinaryApp {
            op: BinaryOp::HasTag,
            ..
        } => true,
        _ => false,
    }
}

/// The guards of `root` the evaluator keeps (plan 5): a `has`/`hasTag`
/// that is [`optional_guard`] and whose access occurs in the scope it
/// guards — the right operand of an `&&` whose left operand contains it, or
/// the `then` branch of an `if` whose test does (the only places the
/// validator's capabilities flow). Any sub-expression of the left/test
/// counts, not only the `&&`-spine: keeping a guard that guards nothing
/// only loses a fold. Such a guard, when its outcomes are `{True}`, is
/// returned as itself with those outcomes rather than the literal, so the
/// access it guards still validates in the result.
pub(super) fn kept_guards(root: &Expr<Option<Type>>, sym_env: &SymEnv) -> Vec<Expr<()>> {
    let mut out: Vec<Expr<()>> = Vec::new();
    for n in root.subexpressions() {
        let (guarding, guarded) = match n.expr_kind() {
            ExprKind::And { left, right } => (left, right),
            ExprKind::If {
                test_expr,
                then_expr,
                ..
            } => (test_expr, then_expr),
            _ => continue,
        };
        for g in guarding.subexpressions() {
            let Some(access) = guard_access(g) else {
                continue;
            };
            if out.iter().any(|k| g.eq_shape(k)) || !optional_guard(g, sym_env) {
                continue;
            }
            if guarded.subexpressions().any(|s| s.eq_shape(&access)) {
                if let Ok(erased) = erase(g) {
                    out.push(erased);
                }
            }
        }
    }
    out
}

/// Marks the atoms of `node` that are among `kept` (structurally).
pub(super) fn mark_kept(node: &mut Node, kept: &[Expr<()>]) {
    match &mut node.kind {
        NodeKind::Atom { keep, .. } => {
            *keep = kept.iter().any(|k| node.expr.eq_shape(k));
        }
        NodeKind::Not(c) => mark_kept(c, kept),
        NodeKind::And(l, r) | NodeKind::Or(l, r) => {
            mark_kept(l, kept);
            if let Some(r) = r {
                mark_kept(r, kept);
            }
        }
        NodeKind::If(c, a, b) => {
            mark_kept(c, kept);
            if let Some(a) = a {
                mark_kept(a, kept);
            }
            if let Some(b) = b {
                mark_kept(b, kept);
            }
        }
    }
}

/// An access inside an atom whose concrete result depends on the receiver
/// entity *existing*, which the symbolic encoding does not model (every
/// entity type's attribute map is a total function): `getAttr` and `getTag`
/// error on a missing entity, `has` of a required attribute is `false`.
#[derive(Debug, Clone)]
pub(super) struct Existence {
    /// The receiver, of type `Option<Entity>`.
    pub receiver: Term,
    /// Its entity type.
    pub ety: EntityType,
    /// The outcomes the *atom* may take when the receiver is missing (empty
    /// for an access that only serves as a fact): `Error` for a plain
    /// `getAttr`/`getTag`; `False` for a required `has` that is the atom;
    /// under `iferror(e, d)`'s first argument, `d`'s literal value (or, for a
    /// `has` there, either boolean); everything else — a nested `has`, or
    /// an `iferror` whose fallback is not a literal — makes the atom's value
    /// unknowable from its term, so all three.
    pub adds: Vec<EvaluationOutcome>,
    /// Whether the receiver is known to exist whenever the enclosing
    /// expression evaluates as assumed (see [`existence_checks`]).
    pub fact: bool,
}

/// Where an access sits with respect to `iferror`: plainly, or under the
/// first argument of one — whose fallback, when the `iferror` is the atom
/// itself and the fallback a boolean literal, is what the atom takes when
/// the access hits a missing entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Plain,
    Coalesced(Option<bool>),
}

/// Whether the symbolic value of `x` may differ from its concrete value
/// *without* an error on a missing entity — an `iferror` (its first
/// argument's error is coalesced symbolically only) or a `has` (literally
/// `true` for a required attribute) anywhere inside. No fact is derived from
/// an access on such a receiver: symbolically it may denote a different
/// entity than the one concretely accessed.
fn phantom<T>(x: &Expr<T>) -> bool {
    match x.expr_kind() {
        ExprKind::HasAttr { .. } => true,
        ExprKind::ExtensionFunctionApp { fn_name, .. } if is_iferror(fn_name) => true,
        _ => children(x).into_iter().any(phantom),
    }
}

fn bool_literal<T>(x: &Expr<T>) -> Option<bool> {
    match x.expr_kind() {
        ExprKind::Lit(Literal::Bool(b)) => Some(*b),
        _ => None,
    }
}

/// Collects the existence-sensitive accesses of `typed` into `out`, in
/// evaluation order (a receiver's own accesses before the access on it).
/// Receivers are compiled with the atom's compiler; only entity-typed ones
/// count.
///
/// `strict` says whether `typed` is evaluated whenever the enclosing
/// expression is (not under an `if` branch or the right operand of an
/// `&&`/`||` of unknown value); `truth` what the enclosing evaluation is
/// assumed to have produced (`Some(b)`: the value `b`; `None`: some value,
/// or nothing is assumed); `root` whether `typed` is the atom itself.
/// A `fact` is an access whose receiver must exist under that assumption:
/// a strict plain `getAttr`/`getTag` (it evaluated without error), or a
/// `has`/`hasTag` known to be `true` (only a present entity has attributes
/// or tags) — on a receiver that is not [`phantom`].
pub(super) fn existence_checks(
    typed: &Expr<Option<Type>>,
    sym_env: &SymEnv,
    strict: bool,
    truth: Option<bool>,
    root: bool,
    out: &mut Vec<Existence>,
) {
    walk(typed, sym_env, strict, Mode::Plain, truth, root, out);
}

fn walk(
    typed: &Expr<Option<Type>>,
    sym_env: &SymEnv,
    strict: bool,
    mode: Mode,
    truth: Option<bool>,
    root: bool,
    out: &mut Vec<Existence>,
) {
    use EvaluationOutcome::{Error, False, True};
    // A receiver that does not compile sits in a dead operand of an atom
    // whose own compilation succeeded, or in an atom that fails anyway;
    // either way it is not existence-sensitive.
    let receiver = |x: &Expr<Option<Type>>| -> Option<(Term, EntityType)> {
        let res = compile(&erase(x).ok()?, sym_env).ok()?;
        match res.term.type_of() {
            TermType::Option { ty } => match &*ty {
                TermType::Entity { ety } => Some((res.term, ety.clone())),
                _ => None,
            },
            _ => None,
        }
    };
    let required = |ety: &EntityType, attr: &str| -> bool {
        match sym_env.entities.attrs(ety).map(|f| f.out_type()) {
            Some(TermType::Record { rty }) => {
                matches!(rty.get(attr), Some(ty) if !ty.is_option_type())
            }
            _ => false,
        }
    };
    let all = || vec![True, False, Error];
    // the outcomes a `getAttr`/`getTag` on a missing entity gives the atom
    let erroring_adds = || match mode {
        Mode::Plain => vec![Error],
        Mode::Coalesced(Some(b)) => vec![if b { True } else { False }],
        Mode::Coalesced(None) => all(),
    };
    let plain_strict = strict && mode == Mode::Plain;
    match typed.expr_kind() {
        ExprKind::GetAttr { expr, .. } => {
            walk(expr, sym_env, strict, mode, None, false, out);
            if let Some((receiver, ety)) = receiver(expr) {
                out.push(Existence {
                    receiver,
                    ety,
                    adds: erroring_adds(),
                    fact: plain_strict && !phantom(expr),
                });
            }
        }
        ExprKind::HasAttr { expr, attr } => {
            walk(expr, sym_env, strict, mode, None, false, out);
            if let Some((receiver, ety)) = receiver(expr) {
                let adds = if required(&ety, attr) {
                    match mode {
                        Mode::Plain if root => vec![False],
                        Mode::Coalesced(Some(_)) => vec![True, False],
                        Mode::Plain | Mode::Coalesced(None) => all(),
                    }
                } else {
                    vec![]
                };
                let fact = plain_strict && truth == Some(true) && !phantom(expr);
                if !adds.is_empty() || fact {
                    out.push(Existence {
                        receiver,
                        ety,
                        adds,
                        fact,
                    });
                }
            }
        }
        ExprKind::BinaryApp {
            op: BinaryOp::GetTag,
            arg1,
            arg2,
        } => {
            walk(arg1, sym_env, strict, mode, None, false, out);
            walk(arg2, sym_env, strict, mode, None, false, out);
            if let Some((receiver, ety)) = receiver(arg1) {
                out.push(Existence {
                    receiver,
                    ety,
                    adds: erroring_adds(),
                    fact: plain_strict && !phantom(arg1),
                });
            }
        }
        ExprKind::BinaryApp {
            op: BinaryOp::HasTag,
            arg1,
            arg2,
        } => {
            walk(arg1, sym_env, strict, mode, None, false, out);
            walk(arg2, sym_env, strict, mode, None, false, out);
            if plain_strict && truth == Some(true) && !phantom(arg1) {
                if let Some((receiver, ety)) = receiver(arg1) {
                    out.push(Existence {
                        receiver,
                        ety,
                        adds: vec![],
                        fact: true,
                    });
                }
            }
        }
        ExprKind::ExtensionFunctionApp { fn_name, args } if is_iferror(fn_name) => {
            if let [e, d] = args.as_slice() {
                // `e`'s error is coalesced (symbolically its value survives);
                // `d` runs only when `e` errs
                let fallback = if root { bool_literal(d) } else { None };
                walk(
                    e,
                    sym_env,
                    false,
                    Mode::Coalesced(fallback),
                    None,
                    false,
                    out,
                );
                walk(d, sym_env, false, mode, None, false, out);
            } else {
                for arg in args.iter() {
                    walk(arg, sym_env, strict, mode, None, false, out);
                }
            }
        }
        ExprKind::And { left, right } => {
            // both operands ran when the `&&` is true; `true && r` is `r`
            let (rs, rt) = if truth == Some(true) {
                (strict, Some(true))
            } else if bool_literal(left) == Some(true) {
                (strict, truth)
            } else {
                (false, None)
            };
            walk(
                left,
                sym_env,
                strict,
                mode,
                if truth == Some(true) { truth } else { None },
                false,
                out,
            );
            walk(right, sym_env, rs, mode, rt, false, out);
        }
        ExprKind::Or { left, right } => {
            let (rs, rt) = if truth == Some(false) {
                (strict, Some(false))
            } else if bool_literal(left) == Some(false) {
                (strict, truth)
            } else {
                (false, None)
            };
            walk(
                left,
                sym_env,
                strict,
                mode,
                if truth == Some(false) { truth } else { None },
                false,
                out,
            );
            walk(right, sym_env, rs, mode, rt, false, out);
        }
        ExprKind::UnaryApp {
            op: UnaryOp::Not,
            arg,
        } => walk(arg, sym_env, strict, mode, truth.map(|b| !b), false, out),
        ExprKind::If {
            test_expr,
            then_expr,
            else_expr,
        } => {
            walk(test_expr, sym_env, strict, mode, None, false, out);
            walk(then_expr, sym_env, false, mode, None, false, out);
            walk(else_expr, sym_env, false, mode, None, false, out);
        }
        _ => {
            for child in children(typed) {
                walk(child, sym_env, strict, mode, None, false, out);
            }
        }
    }
}

/// The facts of `typed` under `truth`: the accesses whose receivers must
/// exist when `typed` evaluates as `truth` says.
pub(super) fn existence_facts(
    typed: &Expr<Option<Type>>,
    sym_env: &SymEnv,
    truth: Option<bool>,
) -> Vec<Existence> {
    let mut checks = Vec::new();
    existence_checks(typed, sym_env, true, truth, true, &mut checks);
    checks.retain(|c| c.fact);
    checks
}

/// The direct children of `e`, in evaluation order.
fn children<T>(e: &Expr<T>) -> Vec<&Expr<T>> {
    match e.expr_kind() {
        ExprKind::If {
            test_expr,
            then_expr,
            else_expr,
        } => vec![test_expr, then_expr, else_expr],
        ExprKind::And { left, right } | ExprKind::Or { left, right } => vec![left, right],
        ExprKind::UnaryApp { arg, .. } => vec![arg],
        ExprKind::BinaryApp { arg1, arg2, .. } => vec![arg1, arg2],
        ExprKind::ExtensionFunctionApp { args, .. } => args.iter().collect(),
        ExprKind::GetAttr { expr, .. }
        | ExprKind::HasAttr { expr, .. }
        | ExprKind::Like { expr, .. }
        | ExprKind::Is { expr, .. } => vec![expr],
        ExprKind::Set(elems) => elems.iter().collect(),
        ExprKind::Record(fields) => fields.values().collect(),
        _ => vec![],
    }
}

impl Node {
    fn compile_result(&self) -> CompileResult {
        CompileResult {
            term: self.term.clone(),
            footprint: Footprint::empty(),
        }
    }
}

/// If `term` (of type `Option<Bool>`) is a literal, the outcome it denotes.
pub(super) fn term_literal(term: &Term) -> Option<EvaluationOutcome> {
    match term {
        Term::Some(inner) => match **inner {
            Term::Prim(TermPrim::Bool(true)) => Some(EvaluationOutcome::True),
            Term::Prim(TermPrim::Bool(false)) => Some(EvaluationOutcome::False),
            _ => None,
        },
        Term::None(_) => Some(EvaluationOutcome::Error),
        _ => None,
    }
}

/// Compiles a typed boolean expression into a [`Node`] tree, accumulating the
/// footprint of every atom into `footprint`.
///
/// Every node's term is exactly what a single top-level
/// `symccopt::compiler::compile` would produce for that sub-expression,
/// because the same functions are applied in the same order; this just keeps
/// the intermediate results.
pub(super) fn build_tree(
    typed: &Arc<Expr<Option<Type>>>,
    sym_env: &SymEnv,
    footprint: &mut BTreeSet<Term>,
) -> Result<Node> {
    if typed.data().is_none() {
        return Err(EvaluationError::NotWellTyped { errs: vec![] });
    }
    let is_bool = matches!(typed.data(), Some(Type::Bool(_)));
    let (term, kind, facts) = match typed.expr_kind() {
        ExprKind::And { left, right } => {
            let l = build_tree(left, sym_env, footprint)?;
            let (r, r_cr, r_err) = split(build_tree(right, sym_env, footprint));
            let res = compile_and(l.compile_result(), r_cr).map_err(|ce| sub_err(ce, r_err))?;
            let kind = NodeKind::And(Box::new(l), r);
            let facts = structure_facts(&kind);
            (res.term, kind, facts)
        }
        ExprKind::Or { left, right } => {
            let l = build_tree(left, sym_env, footprint)?;
            let (r, r_cr, r_err) = split(build_tree(right, sym_env, footprint));
            let res = compile_or(l.compile_result(), r_cr).map_err(|ce| sub_err(ce, r_err))?;
            let kind = NodeKind::Or(Box::new(l), r);
            let facts = structure_facts(&kind);
            (res.term, kind, facts)
        }
        ExprKind::If {
            test_expr,
            then_expr,
            else_expr,
        } if is_bool => {
            let c = build_tree(test_expr, sym_env, footprint)?;
            let (a, a_cr, a_err) = split(build_tree(then_expr, sym_env, footprint));
            let (b, b_cr, b_err) = split(build_tree(else_expr, sym_env, footprint));
            let res = compile_if(c.compile_result(), a_cr, b_cr)
                .map_err(|ce| sub_err(ce, a_err.or(b_err)))?;
            let kind = NodeKind::If(Box::new(c), a, b);
            let facts = structure_facts(&kind);
            (res.term, kind, facts)
        }
        ExprKind::UnaryApp {
            op: UnaryOp::Not,
            arg,
        } => {
            let x = build_tree(arg, sym_env, footprint)?;
            let res = compile_not(x.compile_result()).map_err(crate::Error::from)?;
            let kind = NodeKind::Not(Box::new(x));
            let facts = structure_facts(&kind);
            (res.term, kind, facts)
        }
        _ => {
            let res = compile(&erase(typed)?, sym_env).map_err(crate::Error::from)?;
            footprint.extend(res.footprint);
            let mut checks = Vec::new();
            existence_checks(typed, sym_env, true, None, true, &mut checks);
            let facts = Facts {
                on_true: existence_facts(typed, sym_env, Some(true)),
                on_false: existence_facts(typed, sym_env, Some(false)),
                any: existence_facts(typed, sym_env, None),
            };
            (
                res.term,
                NodeKind::Atom {
                    checks,
                    keep: false,
                },
                facts,
            )
        }
    };
    Ok(Node {
        expr: typed.clone(),
        term,
        kind,
        facts,
    })
}

/// Splits the result of building a child node into the pieces the
/// `compile_*` functions and the [`NodeKind`] need.
///
/// `compile_and` and friends take the right operand as a `Result` and only
/// inspect it when the left operand is not a literal, so a child that failed
/// to build is passed as a placeholder error; if the compiler does inspect it,
/// the child's real error is reported (see [`sub_err`]).
type Split = (
    Option<Box<Node>>,
    std::result::Result<CompileResult, CompileError>,
    Option<EvaluationError>,
);

fn split(child: Result<Node>) -> Split {
    match child {
        Ok(node) => {
            let cr = node.compile_result();
            (Some(Box::new(node)), Ok(cr), None)
        }
        Err(e) => (None, Err(CompileError::TypeError), Some(e)),
    }
}

/// The error to report when compiling a structure node fails: the error of a
/// child that failed to build, if any, else the compiler's own.
fn sub_err(ce: CompileError, child_err: Option<EvaluationError>) -> EvaluationError {
    child_err.unwrap_or_else(|| EvaluationError::SymCC(crate::Error::from(ce)))
}
