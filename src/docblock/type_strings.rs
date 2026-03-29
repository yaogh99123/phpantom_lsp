//! Type string manipulation utilities for raw docblock text.
//!
//! This submodule provides helpers for tokenising and normalising raw
//! type strings extracted from docblocks: splitting type tokens,
//! splitting unions/intersections at depth 0, cleaning trailing
//! punctuation, and splitting generic arguments on commas.
//!
//! These functions operate on unparsed docblock text and are used by
//! the other `docblock` submodules (generics, shapes, callable types)
//! and a few external call sites. Type-level operations (scalar
//! classification, nullable handling, self/static replacement) have
//! been migrated to `PhpType` methods in `php_type.rs`.

/// All built-in type keywords offered in PHPDoc type completion contexts.
///
/// Includes primitive PHP types (`int`, `string`, `array`, …), PHPDoc-only
/// pseudo-types (`mixed`, `class-string`, `non-empty-string`, etc.) and
/// the special `self` / `static` keywords.  Kept here as a single source
/// of truth so the list is maintained in one place rather than duplicated
/// in the completion handler.
pub(crate) const PHPDOC_TYPE_KEYWORDS: &[&str] = &[
    // ── Primitive types ─────────────────────────────────────────────
    "int",
    "integer",
    "float",
    "double",
    "string",
    "bool",
    "boolean",
    "void",
    "never",
    "null",
    "false",
    "true",
    "array",
    "callable",
    "iterable",
    "resource",
    // ── Additional PHP built-in types ───────────────────────────────
    "object",
    "mixed",
    "self",
    "static",
    // ── PHPStan / PHPDoc extended types ─────────────────────────────
    // Integer refinements
    "positive-int",
    "negative-int",
    "non-negative-int",
    "non-positive-int",
    "non-zero-int",
    "int-mask",
    "int-mask-of",
    // String refinements
    "non-empty-string",
    "non-falsy-string",
    "truthy-string",
    "literal-string",
    "non-empty-literal-string",
    "numeric-string",
    "callable-string",
    "lowercase-string",
    "uppercase-string",
    "non-empty-lowercase-string",
    "non-empty-uppercase-string",
    // Array / list refinements
    "list",
    "non-empty-list",
    "non-empty-array",
    "associative-array",
    // Class-string variants
    "class-string",
    "interface-string",
    "trait-string",
    "enum-string",
    // Scalar / mixed variants
    "scalar",
    "numeric",
    "empty-scalar",
    "non-empty-scalar",
    "non-empty-mixed",
    "number",
    "empty",
    // Object / callable variants
    "callable-object",
    "callable-array",
    // Resource variants
    "closed-resource",
    "open-resource",
    // Key / value utility types
    "array-key",
    "key-of",
    "value-of",
    // Never aliases
    "no-return",
    "noreturn",
    "never-return",
    "never-returns",
];

/// Split off the first type token from `s`, respecting `<…>` and `{…}`
/// nesting (the latter is needed for PHPStan array shape syntax like
/// `array{name: string, age: int}`).
///
/// Returns `(type_token, remainder)` where `type_token` is the full type
/// (e.g. `Collection<int, User>` or `array{name: string}`) and
/// `remainder` is whatever follows.
pub(crate) fn split_type_token(s: &str) -> (&str, &str) {
    let mut angle_depth = 0i32;
    let mut brace_depth = 0i32;
    let mut paren_depth = 0i32;
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut prev_char = '\0';

    for (i, c) in s.char_indices() {
        // Handle string literals inside array shape keys — skip everything
        // inside quotes so that `{`, `}`, `,`, `:` etc. are not
        // misinterpreted as structural delimiters.
        if in_single_quote {
            if c == '\'' && prev_char != '\\' {
                in_single_quote = false;
            }
            prev_char = c;
            continue;
        }
        if in_double_quote {
            if c == '"' && prev_char != '\\' {
                in_double_quote = false;
            }
            prev_char = c;
            continue;
        }

        match c {
            '\'' if brace_depth > 0 => in_single_quote = true,
            '"' if brace_depth > 0 => in_double_quote = true,
            '<' => angle_depth += 1,
            '>' if angle_depth > 0 => {
                angle_depth -= 1;
                // If we just closed the outermost `<`, the type ends here
                // (but only when we're not also inside braces or parens).
                // Continue consuming any union/intersection suffix so
                // that `Collection<int, User>|null` stays one token.
                if angle_depth == 0 && brace_depth == 0 && paren_depth == 0 {
                    let end = i + c.len_utf8();
                    let end = consume_union_intersection_suffix(s, end);
                    return (&s[..end], &s[end..]);
                }
            }
            '{' => brace_depth += 1,
            '}' => {
                brace_depth -= 1;
                // If we just closed the outermost `{`, the type ends here
                // (but only when we're not also inside angle brackets or parens).
                // Continue consuming any union/intersection suffix so
                // that `array{id: int}|null` stays one token.
                if brace_depth == 0 && angle_depth == 0 && paren_depth == 0 {
                    let end = i + c.len_utf8();
                    let end = consume_union_intersection_suffix(s, end);
                    return (&s[..end], &s[end..]);
                }
            }
            '(' => paren_depth += 1,
            ')' => {
                paren_depth -= 1;
                // After closing the outermost `(…)`, check whether a
                // callable return-type follows (`: ReturnType`).  If so,
                // consume the `: ` and the return-type token as part of
                // this token.
                if paren_depth == 0 && angle_depth == 0 && brace_depth == 0 {
                    let after_paren = i + c.len_utf8();
                    let rest = &s[after_paren..];
                    let rest_trimmed = rest.trim_start();
                    if let Some(after_colon) = rest_trimmed.strip_prefix(':') {
                        let after_colon = after_colon.trim_start();
                        if !after_colon.is_empty() {
                            // Consume the return-type token.
                            let (ret_tok, _remainder) = split_type_token(after_colon);
                            // Compute the end offset: start of `after_colon`
                            // relative to `s` + length of ret_tok.
                            let colon_start_in_s =
                                s.len() - rest.len() + (rest.len() - rest_trimmed.len()) + 1;
                            let ret_start_in_s = colon_start_in_s
                                + (after_colon.as_ptr() as usize
                                    - s[colon_start_in_s..].as_ptr() as usize);
                            let mut end = ret_start_in_s + ret_tok.len();

                            // After a callable return type, continue
                            // consuming union/intersection suffixes so
                            // that `(Closure(Builder): mixed)|null`
                            // is kept as one token.
                            end = consume_union_intersection_suffix(s, end);

                            return (&s[..end], &s[end..]);
                        }
                    }
                    // After a bare parenthesized group (no callable
                    // return type), continue consuming any
                    // union/intersection suffix.  This handles DNF
                    // types like `(A&B)|C` and grouped callables
                    // like `(Closure(X): Y)|null`.
                    let end = consume_union_intersection_suffix(s, after_paren);
                    return (&s[..end], &s[end..]);
                }
            }
            c if c.is_whitespace() && angle_depth == 0 && brace_depth == 0 && paren_depth == 0 => {
                return (&s[..i], &s[i..]);
            }
            _ => {}
        }
        prev_char = c;
    }
    (s, "")
}

/// After a parenthesized type group or callable return type, consume
/// any `|Type` or `&Type` continuation so the full union/intersection
/// is kept as a single token.
///
/// `pos` is the byte offset just past the already-consumed portion of
/// `s`.  Returns the updated end offset after consuming zero or more
/// `|`/`&`-separated type parts.
fn consume_union_intersection_suffix(s: &str, pos: usize) -> usize {
    let mut end = pos;
    loop {
        let rest = &s[end..];
        // Allow optional whitespace before the operator, but only if
        // the operator is `|` or `&` (not a plain space which would
        // signal the start of the next token like a parameter name).
        let rest_trimmed = rest.trim_start();
        let first = rest_trimmed.chars().next();
        if first == Some('|') || first == Some('&') {
            // `&$var` is a by-reference parameter, not an intersection.
            if first == Some('&') && rest_trimmed.as_bytes().get(1) == Some(&b'$') {
                break;
            }
            // Skip the operator character.
            let after_op = &rest_trimmed[1..];
            let after_op = after_op.trim_start();
            if after_op.is_empty() {
                break;
            }
            // Consume the next type token.
            let (tok, _) = split_type_token(after_op);
            if tok.is_empty() {
                break;
            }
            // Compute the absolute end position from the consumed
            // token.  `after_op` is a sub-slice of `s`, so pointer
            // arithmetic gives us the byte offset.
            let tok_start_in_s = after_op.as_ptr() as usize - s.as_ptr() as usize;
            end = tok_start_in_s + tok.len();
        } else {
            break;
        }
    }
    end
}

/// Split a type string on `|` at nesting depth 0, respecting `<…>`,
/// `(…)`, and `{…}` nesting.
///
/// Returns a `Vec` with at least one element.  If there is no `|` at
/// depth 0, the returned vector contains the entire input as a single
/// element.
///
/// # Examples
///
/// - `"Foo|null"` → `["Foo", "null"]`
/// - `"Collection<int|string, User>|null"` → `["Collection<int|string, User>", "null"]`
/// - `"array{name: string|int}|null"` → `["array{name: string|int}", "null"]`
/// - `"Foo"` → `["Foo"]`
pub(crate) fn split_union_depth0(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth_angle = 0i32;
    let mut depth_paren = 0i32;
    let mut depth_brace = 0i32;
    let mut start = 0;

    for (i, c) in s.char_indices() {
        match c {
            '<' => depth_angle += 1,
            '>' => depth_angle -= 1,
            '(' => depth_paren += 1,
            ')' => depth_paren -= 1,
            '{' => depth_brace += 1,
            '}' => depth_brace -= 1,
            '|' if depth_angle == 0 && depth_paren == 0 && depth_brace == 0 => {
                parts.push(&s[start..i]);
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(&s[start..]);
    parts
}

/// Clean a raw type string from a docblock, **preserving** generic
/// parameters so that downstream resolution can apply generic
/// substitution.
///
/// Specifically this function:
///   - Strips leading `\` (PHP fully-qualified prefix)
///   - Strips trailing punctuation (`.`, `,`) that could leak from
///     docblock descriptions
///   - Handles `TypeName|null` → `TypeName` (using depth-0 splitting so
///     that `Collection<int|string, User>|null` is handled correctly)
///
/// Generic parameters like `<int, User>` are **not** stripped.  Use
/// `PhpType::parse(s).base_name()` when you need just the unparameterised
/// class name.
pub fn clean_type(raw: &str) -> String {
    // Preserve the leading `\` — it marks the type as a fully-qualified
    // name (FQN).  Stripping it would make the name look relative,
    // causing `resolve_type_string` to incorrectly prepend the current
    // file's namespace (e.g. `\Illuminate\Builder` would become
    // `App\Models\Illuminate\Builder`).  Downstream consumers
    // (`type_hint_to_classes`, `resolve_name`, `resolve_class_name`)
    // all handle `\`-prefixed names correctly.
    let s = raw;

    // Strip trailing punctuation that could leak from docblocks
    // (e.g. trailing `.` or `,` in descriptions).
    // Be careful not to strip `,` or `.` that is inside `<…>`.
    let s = s.trim_end_matches(['.', ',']);

    // Handle `TypeName|null` → extract the non-null part, using depth-0
    // splitting so that `|` inside `<…>` is not mistaken for a union
    // separator.
    let parts = split_union_depth0(s);
    if parts.len() > 1 {
        let non_null: Vec<&str> = parts
            .into_iter()
            .map(|p| p.trim())
            .filter(|p| !p.eq_ignore_ascii_case("null"))
            .collect();

        if non_null.len() == 1 {
            return non_null[0].to_string();
        }
        // Multiple non-null parts → keep as union
        if non_null.len() > 1 {
            return non_null.join("|");
        }
    }

    s.to_string()
}

/// Split generic arguments on commas at depth 0, respecting `<…>`,
/// `(…)`, and `{…}` nesting.
///
/// Returns trimmed, non-empty segments. This is the single shared
/// implementation used by `parse_generic_args`, `extract_generics_tag`,
/// `apply_substitution`, and the generic-key/value extraction helpers.
pub(crate) fn split_generic_args(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth_angle = 0i32;
    let mut depth_paren = 0i32;
    let mut depth_brace = 0i32;
    let mut start = 0;

    for (i, ch) in s.char_indices() {
        match ch {
            '<' => depth_angle += 1,
            '>' => depth_angle -= 1,
            '(' => depth_paren += 1,
            ')' => depth_paren -= 1,
            '{' => depth_brace += 1,
            '}' => depth_brace -= 1,
            ',' if depth_angle == 0 && depth_paren == 0 && depth_brace == 0 => {
                parts.push(s[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    let last = s[start..].trim();
    if !last.is_empty() {
        parts.push(last);
    }
    parts
}
