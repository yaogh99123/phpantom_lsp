# PhpType Migration — File-by-File Tracking Tree

This document tracks every file in `src/` and whether it still has
PhpType integration gaps.

**⚠️ Do NOT replace this document with a summary like "no outstanding
items" or "all tasks complete." This is a persistent tracking tree,
not a status note. The structure must survive across sessions so that
future scans only need to check branches that still have open items.
When all checklist items are gone the document still stays — it
becomes the record of what was audited and confirmed clean.**

## How to use this document

- **Completing an item:** remove the `- [ ]` line (and only that
  line) from the open-items section. If the file section has no
  remaining items, move the file path to the "Confirmed clean" list
  at the bottom.
- **Completing a directory:** when every file in a directory section
  has been moved to the clean list, remove the directory heading.
- **Adding new items:** if a scan discovers a new gap, add it under
  the appropriate file/directory heading with its sprint task ID.
- **Never delete** the "Confirmed clean" section or this header.
  They prevent redundant re-scanning.

---

## Outstanding items

No outstanding items. All PM21 categories have been completed or
confirmed clean.


## Confirmed clean files (removed from tracking)

These files have been audited and confirmed to have no PhpType
integration gaps. They are listed here as a record so future scans
skip them.

- `src/php_type.rs` — PhpType itself; convenience constructors (`list`, `generic_array`, `generic_array_val`, `empty_sentinel`) and predicates (`is_named`, `is_named_ci`) added; `to_native_hint_typed()` uses convenience constructors for concrete arms, `Named`/`Generic` arms use `native_scalar_name()` → `PhpType::Named(n.to_string())` which is correct
- `src/types.rs` — `backed_type` migrated to `BackedEnumType` enum
- `src/lib.rs`, `src/main.rs`, `src/server.rs`, `src/config.rs`
- `src/names.rs`
- `src/util.rs` — `is_subtype_of` is now a private implementation detail of `is_subtype_of_typed`; all external callers migrated to the typed API
- `src/resolution.rs` — `find_or_load_class` delegates to `find_or_load_class_typed`, which uses `base_name()` directly
- `src/subject_extraction.rs`, `src/subject_expr.rs`
- `src/classmap_scanner.rs`, `src/composer.rs`, `src/stubs.rs`
- `src/phar.rs`, `src/phpstan.rs`
- `src/analyse.rs`, `src/fix.rs`
- `src/formatting.rs`, `src/folding.rs`, `src/selection_range.rs`
- `src/semantic_tokens.rs`
- `src/inlay_hints.rs`, `src/code_lens.rs`
- `src/document_symbols.rs`, `src/document_links.rs`
- `src/workspace_symbols.rs`, `src/type_hierarchy.rs`
- `src/scope_collector/mod.rs`
- `src/highlight/mod.rs`
- `src/rename/mod.rs`
- `src/references/mod.rs`
- `src/symbol_map/` (all files — `SelfStaticParentKind` enum replaces `keyword: String`)
- `src/hover/` (all files)
- `src/diagnostics/` (all files including `undefined_variables.rs`)
- `src/completion/resolver.rs`
- `src/completion/resolve.rs`
- `src/completion/target.rs`, `src/completion/named_args.rs`
- `src/completion/use_edit.rs`, `src/completion/mod.rs`
- `src/completion/array_shape.rs` — returns `Option<PhpType>`
- `src/completion/phpdoc/helpers.rs`, `src/completion/phpdoc/mod.rs`
- `src/completion/source/` (all files)
- `src/completion/context/` (all files)
- `src/completion/context/namespace_completion.rs` — deals with namespace strings, not types
- `src/completion/phpdoc/context.rs` — `is_type_keyword()` delegates to `crate::php_type::is_keyword_type()`
- `src/completion/phpdoc/generation.rs` — uses `PhpType::generic_array_val()` for bare array enrichment
- `src/completion/builder.rs` — `attribute_placeholder()` uses `PhpType` predicates directly
- `src/completion/types/conditional.rs` — `try_resolve_with_template_default()` uses structural `PhpType` matching
- `src/completion/types/narrowing.rs` — uses `PhpType::empty_sentinel()` constructor and `is_empty_sentinel()` predicate
- `src/completion/call_resolution.rs` — no PhpType integration gaps
- `src/completion/variable/closure_resolution.rs` — `inferred_type_is_more_specific()` accepts `&PhpType`; `extract_model_from_builder()` uses `is_named()`; `build_receiver_self_type()` uses `PhpType::Named` destructuring; class lookups use `base_name()`; variadic wrapping uses `PhpType::list()`
- `src/completion/variable/rhs_resolution.rs` — `classify_template_binding()` accepts `Option<&PhpType>`; `extract_array_type_at_position()` accepts `&PhpType`; `resolve_property_with_hint()` threads `PhpType` directly; `classify_from_php_type()` uses `is_named()`; `resolve_rhs_static_call()` uses `base_name()`
- `src/completion/variable/resolution.rs` — no string intermediaries; `merge_push_type()` uses `PhpType::list()`; `merge_keyed_type()` uses `PhpType::generic_array()`/`generic_array_val()`; `is_int_like_key()` has typed wrapper
- `src/completion/variable/raw_type_inference.rs` — uses `PhpType::list()` constructor
- `src/completion/handler.rs` — uses `PhpType::mixed().to_string()` for display fallback
- `src/virtual_members/mod.rs`, `src/virtual_members/phpdoc.rs` — `extract_property_tags` returns `Option<PhpType>`
- `src/virtual_members/laravel/patches.rs` — uses `is_bare_array()`
- `src/virtual_members/laravel/builder.rs` — clones `PhpType` directly
- `src/virtual_members/laravel/mod.rs` — class lookups use `base_name()` instead of `to_string()`
- `src/virtual_members/laravel/relationships.rs` — `extract_related_type_typed()` returns `Option<&PhpType>`; `build_property_type()` accepts `Option<&PhpType>`
- `src/code_actions/mod.rs`, `src/code_actions/cursor_context.rs`
- `src/code_actions/change_visibility.rs`
- `src/code_actions/extract_constant.rs`
- `src/code_actions/extract_variable.rs`
- `src/code_actions/extract_function.rs` — `build_docblock_for_extraction()` passes `&PhpType` directly to `enrichment_plain()`; `resolve_param_types()` returns `(String, PhpType, PhpType)` tuples
- `src/code_actions/generate_constructor.rs` — `QualifyingProperty::type_hint` is `Option<PhpType>`
- `src/code_actions/generate_getter_setter.rs` — `AccessorProperty::type_hint` is `Option<PhpType>`
- `src/code_actions/generate_property_hooks.rs`
- `src/code_actions/implement_methods.rs` — `format_return_type` uses `shorten_php_type_direct` directly
- `src/code_actions/import_class.rs`
- `src/code_actions/inline_variable.rs`
- `src/code_actions/promote_constructor_param.rs`
- `src/code_actions/remove_unused_import.rs`
- `src/code_actions/replace_deprecated.rs`
- `src/code_actions/simplify_null.rs`
- `src/code_actions/phpstan/mod.rs`
- `src/code_actions/phpstan/add_iterable_type.rs` — remaining `Named(n) if n.eq_ignore_ascii_case(iterable_type)` is a dynamic comparison, not a keyword check
- `src/code_actions/phpstan/add_override.rs`
- `src/code_actions/phpstan/add_return_type_will_change.rs`
- `src/code_actions/phpstan/add_throws.rs`
- `src/code_actions/phpstan/fix_phpdoc_type.rs`
- `src/code_actions/phpstan/fix_prefixed_class.rs`
- `src/code_actions/phpstan/fix_return_type.rs` — `CurrentReturnType` stores `Option<PhpType>`; `infer_array_literal_type()` uses `PhpType::equivalent()` for dedup and `PhpType::generic_array()`/`list()` constructors
- `src/code_actions/phpstan/remove_unused_return_type.rs` — `remove_type_from_union()` returns `Option<PhpType>`
- `src/code_actions/phpstan/ignore.rs`
- `src/code_actions/phpstan/new_static.rs`
- `src/code_actions/phpstan/remove_assert.rs`
- `src/code_actions/phpstan/remove_override.rs`
- `src/code_actions/phpstan/remove_throws.rs`
- `src/code_actions/phpstan/remove_unreachable.rs`
- `src/code_actions/update_docblock.rs` — `DocParam` has `type_parsed: PhpType`; `DocReturn` has `type_parsed: PhpType`; `is_type_contradiction()` accepts `&PhpType`
- `src/inheritance.rs` — `is_key_like_bound()` uses `is_array_key()`, `is_int()`, `is_string_type()` predicates
- `src/docblock/tags.rs` — `is_compatible_refinement_typed()` uses structural `PhpType` predicates; `should_override_type_typed()` uses `PhpType` predicates
- `src/docblock/virtual_members.rs` — `parse_method_tag_params` parses once and clones
- `src/parser/classes.rs` — `extract_custom_collection_from_new_collection()` uses `base_name()`
- `src/definition/member/mod.rs` — `find_scope_on_builder_model()` uses `base_name()`
- `src/signature_help.rs` — uses `PhpType::mixed().to_string()` for display fallback
- All `*_tests.rs` sibling files