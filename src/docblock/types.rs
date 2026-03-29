//! Type cleaning and classification utilities for PHPDoc types.
//!
//! This module was split into focused submodules for navigability:
//!
//! - [`super::type_strings`]: Foundational type string manipulation (constants,
//!   splitting, cleaning, stripping, scalar checks, self/static replacement)
//! - [`super::generics`]: Generic argument parsing and iterable element/key extraction
//! - [`super::shapes`]: Array shape and object shape parsing
//! - [`super::callable_types`]: Callable/Closure return type and parameter extraction,
//!   Generator TSend/TValue extraction
//!
//! All public and crate-visible items are re-exported here so that existing
//! `use crate::docblock::types::*` and `use super::types::*` call sites
//! continue to work without modification.

// ─── Re-exports: type_strings ───────────────────────────────────────────────

pub(crate) use super::type_strings::PHPDOC_TYPE_KEYWORDS;
pub use super::type_strings::clean_type;
pub(crate) use super::type_strings::{split_generic_args, split_type_token};

// ─── Re-exports: generics ───────────────────────────────────────────────────

pub use super::generics::{
    extract_generic_key_type, extract_generic_value_type, extract_iterable_element_type,
};

// ─── Re-exports: shapes ─────────────────────────────────────────────────────

pub use super::shapes::{
    extract_array_shape_value_type, extract_object_shape_property_type, is_object_shape,
    parse_array_shape, parse_object_shape,
};

// ─── Re-exports: callable_types ─────────────────────────────────────────────

pub use super::callable_types::{
    extract_callable_param_types, extract_callable_return_type, extract_generator_send_type,
    extract_generator_value_type_raw,
};

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "types_tests.rs"]
mod tests;
