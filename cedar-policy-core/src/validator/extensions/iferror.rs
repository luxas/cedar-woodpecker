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

//! Typing of the `iferror` extension: `iferror : (Bool, Bool) -> Bool`.
//!
//! Typing is strict, like every other function: `iferror(principal.opt, false)`
//! without a `has` guard is rejected as an unguarded optional access — the
//! operator coalesces evaluation errors, not validation errors.

use crate::extensions::iferror;
use crate::validator::extension_schema::{ExtensionFunctionType, ExtensionSchema};
use crate::validator::types::Type;

/// Construct the extension schema
pub fn extension_schema() -> ExtensionSchema {
    let ext = iferror::extension();
    let fun_tys = ext.funcs().map(|f| {
        debug_assert!(f
            .return_type()
            .map(|ty| Type::primitive_boolean().is_consistent_with(ty))
            .unwrap_or(false));
        ExtensionFunctionType::new(
            f.name().clone(),
            vec![Type::primitive_boolean(), Type::primitive_boolean()],
            Type::primitive_boolean(),
            None,
            f.is_variadic(),
        )
    });
    ExtensionSchema::new(ext.name().clone(), fun_tys, std::iter::empty())
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn extension_schema_correctness() {
        let _ = extension_schema();
    }
}
