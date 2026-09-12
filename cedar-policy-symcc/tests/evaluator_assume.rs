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

//! Tests for the symbolic evaluator's assumptions about concrete and partial
//! entity data and requests. The concrete evaluator is the oracle wherever
//! the data is complete.

#![expect(clippy::unwrap_used, clippy::panic, reason = "test code")]

mod utils;

use std::collections::HashMap;
use std::str::FromStr;

use cedar_policy::{Context, Entities, EntityUid, Expression, Request, RequestEnv, Schema};
use cedar_policy_core::ast::Expr;
use cedar_policy_core::evaluator::Evaluator as ConcreteEvaluator;
use cedar_policy_core::extensions::Extensions;
use cedar_policy_core::tpe::entities::{PartialEntities, PartialEntity};
use cedar_policy_core::tpe::residual::EvaluationOutcome;
use cedar_policy_symcc::evaluator::{
    literal_of, request_env_of, EvaluationError, EvaluationMetadata, Evaluator,
};
use cedar_policy_symcc::solver::LocalSolver;
use cedar_policy_symcc::CedarSymCompiler;
use nonempty_collections::{nes, NESet};

use utils::{req_env_from_strs, schema_from_cedarstr};

const SCHEMA: &str = r#"
entity Group in [Group];
entity User in [Group] { name: String, active: Bool, age: Long, nick?: String, flag?: Bool } tags Long;
entity Document { protected: Bool, owner: User, level: Long };
action view appliesTo { principal: User, resource: Document, context: { admin: Bool } };
action edit appliesTo { principal: User, resource: Document, context: { admin: Bool } };
"#;

const ENTITIES: &str = r#"[
  { "uid": { "type": "User", "id": "alice" },
    "attrs": { "name": "alice", "active": true, "age": 30, "nick": "al" },
    "parents": [ { "type": "Group", "id": "eng" } ],
    "tags": { "t1": 1, "t2": 2 } },
  { "uid": { "type": "User", "id": "carol" },
    "attrs": { "name": "carol", "active": false, "age": 5 },
    "parents": [] },
  { "uid": { "type": "Group", "id": "eng" }, "attrs": {}, "parents": [ { "type": "Group", "id": "all" } ] },
  { "uid": { "type": "Group", "id": "all" }, "attrs": {}, "parents": [] },
  { "uid": { "type": "Document", "id": "doc" },
    "attrs": { "protected": false, "owner": { "__entity": { "type": "User", "id": "alice" } }, "level": 3 },
    "parents": [] }
]"#;

fn schema() -> Schema {
    schema_from_cedarstr(SCHEMA)
}

fn entities(schema: &Schema) -> Entities {
    Entities::from_json_str(ENTITIES, Some(schema)).unwrap()
}

fn request(schema: &Schema, action: &str, admin: bool) -> Request {
    let context = Context::from_json_str(&format!(r#"{{ "admin": {admin} }}"#), None).unwrap();
    Request::new(
        EntityUid::from_str(r#"User::"alice""#).unwrap(),
        EntityUid::from_str(&format!(r#"Action::"{action}""#)).unwrap(),
        EntityUid::from_str(r#"Document::"doc""#).unwrap(),
        context,
        Some(schema),
    )
    .unwrap()
}

fn expr(text: &str) -> Expr {
    Expression::from_str(text).unwrap().as_ref().clone()
}

fn evaluator(schema: &Schema) -> Evaluator<LocalSolver> {
    let compiler = CedarSymCompiler::new(LocalSolver::cvc5().unwrap()).unwrap();
    Evaluator::new(compiler, schema).unwrap()
}

fn outcomes(set: &[EvaluationOutcome]) -> NESet<EvaluationOutcome> {
    let mut iter = set.iter().cloned();
    let mut res = nes![iter.next().unwrap()];
    for o in iter {
        res.insert(o);
    }
    res
}

use EvaluationOutcome::{Error as E, False as F, True as T};

/// The concrete evaluator's verdict on a boolean expression.
fn concrete(text: &str, request: &Request, entities: &Entities) -> bool {
    let ev = ConcreteEvaluator::new(
        request.as_ref().clone(),
        entities.as_ref(),
        Extensions::all_available(),
    );
    match ev.interpret(&expr(text), &HashMap::new()) {
        Ok(v) => v.get_as_bool().unwrap(),
        Err(e) => panic!("concrete evaluation of `{text}` failed: {e}"),
    }
}

async fn eval(
    ev: &mut Evaluator<LocalSolver>,
    text: &str,
    env: &RequestEnv,
) -> Expr<EvaluationMetadata> {
    let e = expr(text);
    let result = ev
        .evaluate(&e, env, Vec::new())
        .await
        .unwrap_or_else(|err| panic!("evaluating `{text}`: {err}"));
    assert!(
        ev.check_equivalent(&e, &result, env, Vec::new())
            .await
            .unwrap(),
        "result `{result}` is not equivalent to `{text}` under the assumptions"
    );
    result
}

#[tokio::test]
async fn concrete_env_folds_everything() {
    let schema = schema();
    let entities = entities(&schema);
    let request = request(&schema, "view", true);
    let env = request_env_of(&request, &schema).unwrap();
    assert_eq!(
        env,
        req_env_from_strs("User", r#"Action::"view""#, "Document")
    );

    let mut ev = evaluator(&schema);
    ev.assume_entities(&entities).unwrap();
    ev.assume_request(&request).unwrap();

    for text in [
        "principal.active",
        "principal.age > 18",
        "principal.age > 40",
        r#"principal has nick && principal.nick == "al""#,
        "principal has flag",
        r#"principal in Group::"eng""#,
        r#"principal in Group::"all""#,
        r#"principal in Group::"other""#,
        r#"principal == User::"alice""#,
        r#"principal == User::"carol""#,
        "resource.owner == principal",
        "resource.owner.age + 1 > 30",
        "!resource.protected && resource.level < 5",
        r#"principal.hasTag("t1") && principal.getTag("t1") == 1"#,
        r#"principal.hasTag("nope")"#,
        r#"principal.hasTag("t2") && principal.getTag("t2") > principal.age"#,
        "context.admin",
        r#"User::"carol".active || User::"carol".age < principal.age"#,
        r#"Group::"eng" in Group::"all""#,
    ] {
        let result = eval(&mut ev, text, &env).await;
        let expected = concrete(text, &request, &entities);
        assert_eq!(
            literal_of(&result),
            Some(expected),
            "`{text}` should fold to {expected}, got `{result}`"
        );
    }
}

#[tokio::test]
async fn unknown_entities_stay_symbolic() {
    let schema = schema();
    let entities = entities(&schema);
    let mut ev = evaluator(&schema);
    ev.assume_entities(&entities).unwrap();
    let env = req_env_from_strs("User", r#"Action::"view""#, "Document");

    // `User::"bob"` is not in the store: nothing is known about it — not
    // even that it exists, so the access may error.
    let r = eval(&mut ev, r#"User::"bob".active"#, &env).await;
    assert_eq!(r.data().outcomes(), &outcomes(&[T, F, E]));
    // The request is not assumed either, so `principal` is symbolic ...
    let r = eval(&mut ev, "principal.active", &env).await;
    assert_eq!(r.data().outcomes(), &outcomes(&[T, F, E]));
    // ... but pinning it to a known entity folds.
    let r = ev
        .evaluate(
            &expr("principal.active"),
            &env,
            vec![expr(r#"principal == User::"carol""#)],
        )
        .await
        .unwrap();
    assert_eq!(literal_of(&r), Some(false));
}

#[tokio::test]
async fn partial_entities() {
    let schema = schema();
    let entities = entities(&schema);
    let env = req_env_from_strs("User", r#"Action::"view""#, "Document");

    // Keep everything about alice except her attributes.
    let full = PartialEntities::from_concrete(entities.as_ref().clone(), schema.as_ref()).unwrap();
    let partial = PartialEntities::from_entities(
        full.entities().map(|e| {
            if e.uid().to_string() == r#"User::"alice""# {
                PartialEntity::new(
                    e.uid().clone(),
                    None,
                    e.ancestors().cloned(),
                    e.tags().cloned(),
                    schema.as_ref(),
                )
                .unwrap()
            } else {
                e.clone()
            }
        }),
        schema.as_ref(),
    )
    .unwrap();

    let mut ev = evaluator(&schema);
    ev.assume_partial_entities(partial);
    let alice = vec![expr(r#"principal == User::"alice""#)];

    let r = ev
        .evaluate(&expr("principal.active"), &env, alice.clone())
        .await
        .unwrap();
    assert_eq!(
        r.data().outcomes(),
        &outcomes(&[T, F]),
        "attributes are unknown"
    );
    let r = ev
        .evaluate(&expr(r#"principal in Group::"all""#), &env, alice.clone())
        .await
        .unwrap();
    assert_eq!(literal_of(&r), Some(true), "ancestors are known");
    let r = ev
        .evaluate(&expr(r#"principal.hasTag("t1")"#), &env, alice)
        .await
        .unwrap();
    assert_eq!(literal_of(&r), Some(true), "tags are known");
}

#[tokio::test]
async fn request_in_another_env_is_impossible() {
    let schema = schema();
    let request = request(&schema, "view", false);
    let mut ev = evaluator(&schema);
    ev.assume_request(&request).unwrap();

    let r = eval(
        &mut ev,
        "context.admin",
        &req_env_from_strs("User", r#"Action::"view""#, "Document"),
    )
    .await;
    assert_eq!(literal_of(&r), Some(false));

    let err = ev
        .evaluate(
            &expr("context.admin"),
            &req_env_from_strs("User", r#"Action::"edit""#, "Document"),
            Vec::new(),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, EvaluationError::UnsatisfiableAssumptions),
        "{err}"
    );
}

#[tokio::test]
async fn invalid_data_is_rejected() {
    let schema = schema();
    let mut ev = evaluator(&schema);
    // `Entities` built without a schema can violate it.
    let bad = Entities::from_json_str(
        r#"[{ "uid": { "type": "User", "id": "x" }, "attrs": { "name": 1 }, "parents": [] }]"#,
        None,
    )
    .unwrap();
    let err = ev.assume_entities(&bad).unwrap_err();
    assert!(matches!(err, EvaluationError::InvalidEntities(_)), "{err}");
}

/// The symbolic store has no notion of a missing entity (its attribute maps
/// are total), so the evaluator tracks existence itself: an access on an
/// entity nothing proves present may error (`getAttr`, `getTag`) or be
/// `false` (`has` of a required attribute). An entity of an assumed store
/// exists, and so does the receiver of an access that an assumption — or the
/// trail — says evaluated without error.
#[tokio::test]
async fn missing_entities() {
    let schema = schema();
    let env = req_env_from_strs("User", r#"Action::"view""#, "Document");

    // Nothing assumed: every entity access may hit a missing entity.
    let mut ev = evaluator(&schema);
    for (text, expected) in [
        // a literal UID nobody vouches for (`check_never_errors` says this
        // never errors; the concrete evaluator errors on an empty store)
        (r#"User::"bob".name == "bob""#, &[T, F, E][..]),
        ("principal.active", &[T, F, E]),
        // `has` of a required attribute is `false`, not an error, on a
        // missing entity; of an optional one it is unconstrained anyway
        ("principal has age", &[T, F]),
        ("principal has nick", &[T, F]),
        // `hasTag` is `false` on a missing entity (no error); when true it
        // proves the entity exists, so the guarded `getTag` cannot error
        (r#"principal.hasTag("t1")"#, &[T, F]),
        (
            r#"principal.hasTag("t1") && principal.getTag("t1") == 1"#,
            &[T, F],
        ),
        // phantom values: the symbolic term keeps a value where the
        // concrete evaluation errors (`iferror` coalesces only symbolically
        // — concretely the fallback runs) or differs (a nested required
        // `has` is literally true), so the atom cannot fold
        (
            "iferror(principal.active == principal.active, false)",
            &[T, F],
        ),
        ("(principal has age) == false", &[T, F, E]),
        ("iferror(principal.active, true) == true", &[T, F, E]),
        // a nested receiver: the document or its owner may be missing
        (r#"resource.owner.name == "alice""#, &[T, F, E]),
        // `iferror` coalesces the missing-entity error too
        ("iferror(principal.active, false)", &[T, F]),
    ] {
        let r = eval(&mut ev, text, &env).await;
        assert_eq!(r.data().outcomes(), &outcomes(expected), "{text}");
    }

    // An assumed store: its entities exist, an unknown UID still may not.
    let mut ev = evaluator(&schema);
    ev.assume_entities(&entities(&schema)).unwrap();
    let r = eval(&mut ev, r#"User::"bob".name == "bob""#, &env).await;
    assert_eq!(r.data().outcomes(), &outcomes(&[T, F, E]));
    let r = eval(&mut ev, r#"User::"alice".name == "bob""#, &env).await;
    assert_eq!(literal_of(&r), Some(false));
    let r = eval(&mut ev, r#"User::"carol" has age"#, &env).await;
    assert_eq!(literal_of(&r), Some(true));
    // pinning the request pins the owner chain through the store
    let r = ev
        .evaluate(
            &expr(r#"resource.owner.name == "alice""#),
            &env,
            vec![expr(
                r#"principal == User::"alice" && resource == Document::"doc""#,
            )],
        )
        .await
        .unwrap();
    assert_eq!(literal_of(&r), Some(true));

    // An assumption that an access evaluated proves its receiver exists ...
    let mut ev = evaluator(&schema);
    ev.assume_expr(expr("principal.age > 18"));
    let r = eval(&mut ev, "principal.active", &env).await;
    assert_eq!(r.data().outcomes(), &outcomes(&[T, F]));
    let r = eval(&mut ev, "principal has age", &env).await;
    assert_eq!(literal_of(&r), Some(true));
    // ... and so does a `has` known to be true (only a present entity has
    // attributes) ...
    let mut ev = evaluator(&schema);
    ev.assume_expr(expr("principal has nick"));
    let r = eval(&mut ev, "principal.active", &env).await;
    assert_eq!(r.data().outcomes(), &outcomes(&[T, F]));
    // ... but `iferror`'s first argument proves nothing (the assumption says
    // it is true or errors), nor does an access on a phantom receiver — one
    // the symbolic and the concrete evaluation may resolve differently
    let mut ev = evaluator(&schema);
    ev.assume_expr(expr("iferror(principal.active, true)"));
    let r = eval(&mut ev, "principal.active", &env).await;
    assert_eq!(r.data().outcomes(), &outcomes(&[T, E]));
    // (symbolically `principal has age` is literally true, so the `if`
    // folds to `principal`; concretely a missing principal takes the other
    // branch — a fact about `principal` here would be wrong)
    let mut ev = evaluator(&schema);
    ev.assume_expr(expr(
        r#"(if principal has age then principal else User::"bob").name == "x""#,
    ));
    let r = eval(&mut ev, "principal.active", &env).await;
    assert_eq!(r.data().outcomes(), &outcomes(&[T, F, E]));

    // The trail: the right operand runs only after the left evaluated.
    let mut ev = evaluator(&schema);
    let r = eval(&mut ev, "principal.active && principal.age > 18", &env).await;
    assert_eq!(r.data().outcomes(), &outcomes(&[T, F, E]));
    let cedar_policy_core::ast::ExprKind::And { left, right } = r.expr_kind() else {
        panic!("expected an `&&`, got `{r}`");
    };
    assert_eq!(left.data().outcomes(), &outcomes(&[T, F, E]));
    assert_eq!(right.data().outcomes(), &outcomes(&[T, F]));
}
