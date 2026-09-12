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

//! The `iferror` extension: `iferror(e, d)` coalesces an evaluation error
//! into a boolean.
//!
//! Semantics (both arguments and the result are `Bool`):
//!
//! | `e`                    | result                                             |
//! | ---------------------- | -------------------------------------------------- |
//! | `true` / `false`       | that value; `d` is **not** evaluated               |
//! | a non-boolean value    | a type error (`iferror` is a *boolean* coalescer)   |
//! | an error               | `d`'s boolean value, or `d`'s own error            |
//!
//! So `iferror(e, false)` is exactly "`e` evaluates to `true`" as a Cedar
//! boolean, and it never errors when `e` is boolean-or-error (everything
//! that validates). `d` is evaluated only when `e` errors, `if`-style.
//!
//! An ordinary extension function receives its arguments already evaluated,
//! so it cannot see an argument's error: like `partial_evaluation`'s
//! `unknown`, `iferror` is registered here for parsing, validation and the
//! strict path, and every evaluator special-cases it before evaluating the
//! arguments ([`crate::evaluator`], [`crate::tpe::evaluator`]). The strict body
//! below is reached only when both arguments already evaluated without error,
//! where returning the first is the right answer.

use crate::ast::{CallStyle, Extension, ExtensionFunction, ExtensionOutputValue, Name, Value};
use crate::entities::SchemaType;
use crate::evaluator;
use std::sync::LazyLock;

/// Name of the extension and of its single function.
pub const EXTENSION_NAME: &str = "iferror";

/// The function name, as a [`Name`].
#[expect(clippy::expect_used, reason = "`iferror` is a valid identifier")]
pub static IFERROR_NAME: LazyLock<Name> = LazyLock::new(|| {
    Name::parse_unqualified_name(EXTENSION_NAME).expect("should be a valid identifier")
});

/// Whether `fn_name` is the `iferror` function.
pub fn is_iferror(fn_name: &Name) -> bool {
    *fn_name == *IFERROR_NAME
}

/// The strict body: both arguments evaluated without error, so the first
/// argument is the result (type-checked to be a boolean, like the fallback).
fn iferror_strict(first: &Value, second: &Value) -> evaluator::Result<ExtensionOutputValue> {
    let b = first.get_as_bool()?;
    let _ = second.get_as_bool()?;
    Ok(Value::from(b).into())
}

/// Construct the extension
pub fn extension() -> Extension {
    Extension::new(
        IFERROR_NAME.clone(),
        vec![ExtensionFunction::binary(
            IFERROR_NAME.clone(),
            CallStyle::FunctionStyle,
            Box::new(iferror_strict),
            SchemaType::Bool,
            (SchemaType::Bool, SchemaType::Bool),
        )],
        std::iter::empty(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extensions::Extensions;

    /// `iferror(a, b)` parses as a function-style call and pretty-prints back
    /// to the same expression.
    #[test]
    fn parses_and_prints() {
        use crate::ast::Expr;
        use std::str::FromStr;
        for src in [
            "iferror(true, false)",
            "!iferror(1 < 2, true) && iferror(false, true)",
        ] {
            let e = Expr::from_str(src).expect("parses");
            let printed = e.to_string();
            let again = Expr::from_str(&printed).expect("round trip parses");
            assert_eq!(e, again, "{src} -> {printed}");
        }
    }

    #[test]
    fn strict_body_returns_the_first_argument() {
        let ext = Extensions::all_available();
        let f = ext.func(&IFERROR_NAME).expect("registered");
        let call = |a: bool, b: bool| f.call(&[Value::from(a), Value::from(b)]);
        assert_eq!(
            call(true, false).unwrap(),
            crate::ast::PartialValue::from(true)
        );
        assert_eq!(
            call(false, true).unwrap(),
            crate::ast::PartialValue::from(false)
        );
        assert!(f.call(&[Value::from(1i64), Value::from(true)]).is_err());
        assert!(f.call(&[Value::from(true)]).is_err());
    }
}
