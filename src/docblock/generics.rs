//! Generic type argument parsing and extraction.
//!
//! This submodule handles parsing generic type parameters from PHPDoc
//! type strings (e.g. `Collection<int, User>`) and extracting element
//! types from generic iterable annotations.

use crate::php_type::PhpType;

use super::type_strings::{clean_type, split_generic_args};

/// Extract the element (value) type from a generic iterable type annotation.
///
/// Handles the most common PHPDoc generic iterable patterns:
///   - `list<User>`              → `Some("User")`
///   - `array<User>`             → `Some("User")`
///   - `array<int, User>`        → `Some("User")`
///   - `iterable<User>`          → `Some("User")`
///   - `iterable<int, User>`     → `Some("User")`
///   - `User[]`                  → `Some("User")`
///   - `Collection<int, User>`   → `Some("User")` (any generic class)
///   - `?list<User>`             → `Some("User")` (nullable)
///   - `\Foo\Bar[]`              → `Some("Bar")`
///   - `Generator<int, User>`    → `Some("User")` (TValue = 2nd param)
///   - `Generator<int, User, mixed, void>` → `Some("User")` (TValue = 2nd param)
///
/// For PHP's `Generator<TKey, TValue, TSend, TReturn>`, the **value** (yield)
/// type is always the second generic parameter regardless of how many params
/// are provided.  For all other generic types the last parameter is used.
///
/// Returns `None` if the type is not a recognised generic iterable or the
/// element type is a scalar (e.g. `list<int>`).
pub fn extract_generic_value_type(raw_type: &str) -> Option<String> {
    let s = raw_type.strip_prefix('\\').unwrap_or(raw_type);
    let s = s.strip_prefix('?').unwrap_or(s);

    // ── Handle `Type[]` shorthand ───────────────────────────────────────
    if let Some(base) = s.strip_suffix("[]") {
        let cleaned = clean_type(base);
        if !cleaned.is_empty() && !PhpType::parse(&cleaned).is_scalar() {
            return Some(cleaned);
        }
        // e.g. `int[]` — no class element type
        return None;
    }

    // ── Handle `GenericType<…>` ─────────────────────────────────────────
    let angle_pos = s.find('<')?;
    let base_type = &s[..angle_pos];
    let inner = s.get(angle_pos + 1..)?.strip_suffix('>')?.trim();
    if inner.is_empty() {
        return None;
    }

    // ── Special-case `Generator<TKey, TValue, TSend, TReturn>` ──────────
    // The yield/value type is always the **second** generic parameter
    // (index 1).  When only one param is given (`Generator<User>`), it is
    // treated as the value type (consistent with single-param behaviour).
    let args = split_generic_args(inner);
    let value_part = if base_type == "Generator" {
        // The yield/value type is always the **second** generic parameter
        // (index 1).  When only one param is given (`Generator<User>`), it is
        // treated as the value type (consistent with single-param behaviour).
        args.get(1).or(args.last()).copied().unwrap_or(inner)
    } else {
        // Default: use the last generic parameter (works for array, list,
        // iterable, Collection, etc.).
        args.last().copied().unwrap_or(inner)
    };

    let cleaned = clean_type(value_part.trim());

    if cleaned.is_empty() || PhpType::parse(&cleaned).is_scalar() {
        return None;
    }
    Some(cleaned)
}

/// Extract the element (value) type from an iterable type annotation,
/// including scalar element types.
///
/// Unlike [`extract_generic_value_type`], which skips scalar element types
/// (because it is used for class-based completion), this function returns
/// the raw element type string regardless of whether it is a class or a
/// scalar.  This is needed for spread operator tracking where we merge
/// element types into a union and the final `list<…>` type is resolved
/// later.
///
/// # Supported patterns
///
/// - `User[]`                → `Some("User")`
/// - `int[]`                 → `Some("int")`
/// - `list<User>`            → `Some("User")`
/// - `array<int, User>`      → `Some("User")`
/// - `iterable<string>`      → `Some("string")`
/// - `Collection<int, User>` → `Some("User")`
/// - `?list<User>`           → `Some("User")`
/// - `\list<User>`           → `Some("User")`
/// - `string`                → `None` (not iterable)
/// - `Closure(): User`       → `None` (not iterable)
pub fn extract_iterable_element_type(raw_type: &str) -> Option<String> {
    let s = raw_type.strip_prefix('\\').unwrap_or(raw_type);
    let s = s.strip_prefix('?').unwrap_or(s);

    // Handle `Type[]` shorthand → element type is everything before `[]`.
    if let Some(base) = s.strip_suffix("[]") {
        let trimmed = base.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
        return None;
    }

    // Handle `GenericType<…>` — extract the last generic parameter.
    let angle_pos = s.find('<')?;
    let inner = s.get(angle_pos + 1..)?.strip_suffix('>')?.trim();
    if inner.is_empty() {
        return None;
    }

    let args = split_generic_args(inner);
    let last = args.last().copied().unwrap_or("").trim();
    if last.is_empty() {
        return None;
    }
    Some(last.to_string())
}

/// Extract the key type from a generic iterable type annotation.
///
/// Handles the most common PHPDoc generic iterable patterns:
///   - `array<int, User>`        → `Some("int")`
///   - `array<string, User>`     → `Some("string")`
///   - `iterable<string, User>`  → `Some("string")`
///   - `Collection<User, Order>` → `Some("User")` (first param of 2+ param generic)
///   - `Generator<int, User>`    → `None` (key is `int`, scalar)
///   - `Generator<Request, User, mixed, void>` → `Some("Request")` (TKey = 1st param)
///   - `list<User>`              → `None` (single-param list → key is always `int`, scalar)
///   - `User[]`                  → `None` (shorthand → key is always `int`, scalar)
///   - `array<User>`             → `None` (single-param array → key is `int`, scalar)
///
/// For PHP's `Generator<TKey, TValue, TSend, TReturn>`, the key type is the
/// first generic parameter — which is the same as the default behaviour, so
/// no special-casing is needed.
///
/// Returns `None` if the type is not a recognised generic iterable with an
/// explicit key type, or if the key type is a scalar (e.g. `int`, `string`).
pub fn extract_generic_key_type(raw_type: &str) -> Option<String> {
    let s = raw_type.strip_prefix('\\').unwrap_or(raw_type);
    let s = s.strip_prefix('?').unwrap_or(s);

    // ── `Type[]` shorthand — key is always int (scalar) ─────────────────
    if s.ends_with("[]") {
        return None;
    }

    // ── Handle `GenericType<…>` ─────────────────────────────────────────
    let angle_pos = s.find('<')?;
    let inner = s.get(angle_pos + 1..)?.strip_suffix('>')?.trim();
    if inner.is_empty() {
        return None;
    }

    // Only two-or-more-parameter generics have an explicit key type.
    // Single-parameter generics (e.g. `list<User>`, `array<User>`) have
    // an implicit `int` key which is scalar — nothing to resolve.
    let args = split_generic_args(inner);
    if args.len() < 2 {
        return None;
    }
    let key_part = args[0];
    let cleaned = clean_type(key_part.trim());

    if cleaned.is_empty() || PhpType::parse(&cleaned).is_scalar() {
        return None;
    }
    Some(cleaned)
}
