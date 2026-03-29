# PHPantom — Mago Crate Migration

This document describes the migration from hand-rolled PHP parsing
subsystems to upstream Mago crates. The goal is to replace fragile,
maintenance-heavy internal code with well-tested, upstream-maintained
libraries — improving correctness and robustness while reducing the
long-term maintenance burden.

> **Guiding principle:** Correctness and robustness win over raw
> performance. We accept modest overhead from structured
> representations in exchange for eliminating entire classes of
> edge-case bugs in string-based type manipulation.

## Crates to adopt

| Crate              | Replaces                                               | Effort      | Status |
| ------------------ | ------------------------------------------------------ | ----------- | ------ |
| `mago-docblock`    | Manual docblock parsing scattered across the codebase   | Medium-High | ✅ Done |
| `mago-type-syntax` | `src/docblock/{type_strings,generics,shapes,callable_types,conditional}.rs` + string-based type pipeline | Very High | M4 |
| `mago-names`       | `src/parser/use_statements.rs` + `use_map` resolution   | Medium-High | M3 |

`mago-docblock` is fully integrated — all modules that benefit from
structured parsing use `DocblockInfo` / `TagInfo`. The remaining
raw-text docblock code in the codebase operates on individual lines
for surgical text editing and is better served by direct string
manipulation.

A fifth crate, `mago-reporting`, comes in as a transitive dependency
of `mago-semantics` and `mago-names`. It does not replace any
PHPantom code but will appear in `Cargo.toml`.

### Crates explicitly ruled out

| Crate              | Reason                                                                   |
| ------------------ | ------------------------------------------------------------------------ |
| `mago-codex`       | Replaces `ClassInfo` model with one that cannot carry `LaravelMetadata`. |
| `mago-semantics`   | 12K false positives on Laravel; no way to inject our type context.       |
| `mago-linter`      | Same problem; `Integration::Laravel` is surface-level only.              |
| `mago-fingerprint` | Requires `mago-names` for limited value; `signature_eq` already works.   |

---

## M3. Migrate to `mago-names`

**What it replaces:** `src/parser/use_statements.rs` (130 lines),
the `use_map: DashMap<String, HashMap<String, String>>` on `Backend`,
and the lazy name-resolution helpers in `src/resolution.rs` that
manually look up the use map.

**Why:** `mago-names` resolves every identifier in a PHP file to its
fully-qualified name in a single pass. This is more correct than our
lazy approach for edge cases: names that resolve differently depending
on whether they appear in a type hint vs. a `new` expression vs. a
function call (PHP's different name resolution rules for classes,
functions, and constants). It also provides `is_imported()` which
tells us whether a name came from a `use` statement — useful for
auto-import code actions and unused-import diagnostics.

**What it does NOT replace:** Cross-file resolution
(`find_or_load_class`, PSR-4 resolution, classmap lookup, stub
loading). Those stay in `src/resolution.rs`. `mago-names` handles
only the within-file syntactic resolution (use statements + namespace
context → FQN).

**Risk:** Medium-high. The `use_map` is read from many places. The
arena lifetime for `ResolvedNames` must outlive the consumers.
Requires restructuring how we store per-file name resolution data.

### Steps

1. ✅ **Add `mago-names` to `Cargo.toml`.**
   This also brings in `foldhash` as a transitive dependency.
   *Done — resolved to v1.15.2.*

2. ✅ **Run the name resolver in `update_ast_inner`.**
   After parsing the `Program`, call
   `mago_names::resolver::NameResolver::new(&arena).resolve(program)`
   to produce a `ResolvedNames`. This happens in the same arena as
   the parse.
   *Done — resolver runs right after `parse_file_content`, while
   the arena is still alive.*

3. ✅ **Store resolved names per file.**
   Used Option A (copy to owned storage).  Added
   `resolved_names: Arc<RwLock<HashMap<String, Arc<OwnedResolvedNames>>>>`
   to `Backend`.  Populated in `update_ast_inner` for files open in the
   editor.  Not populated for vendor/stub files loaded via
   `parse_and_cache_content_versioned` (those files are never queried
   by byte offset).  Shared via `Arc::clone` in
   `clone_for_diagnostic_worker`, cleaned up in `clear_file_maps`.

4. ✅ **Build an `OwnedResolvedNames` wrapper.**
   Created `src/names.rs` with `OwnedResolvedNames` providing:
   - `get(offset) -> Option<&str>` — FQN lookup by byte offset
   - `is_imported(offset) -> bool` — was the name from a `use` stmt
   - `iter()` — iterate all `(offset, fqn, imported)` triples

   `to_use_map()` was originally planned but dropped: it cannot
   reproduce aliases (e.g. `use App\Models\User as U` → the
   resolved FQN is `App\Models\User` but the import key is `U`,
   and `resolved_names` does not store the original short text).

   Also added `FileContext::resolve_name_at(name, offset)` as a
   convenience method that tries `resolved_names` first and falls
   back to the legacy `resolve_to_fqn` logic.

5. ✅ **Replace `use_map` reads incrementally.**

   **Migrated to byte-offset lookups (`resolved_names`):**
   - `src/diagnostics/unknown_classes.rs` — `ClassReference` FQN
     resolution and `is_imported` check
   - `src/diagnostics/deprecated.rs` — `ClassReference` FQN resolution
   - `src/definition/resolve.rs` — `FunctionCall`, `ConstantReference`,
     and `ClassReference` resolution via `ctx.resolve_name_at()`
   - `src/definition/implementation.rs` — `resolve_class_implementation`
   - `src/highlight/mod.rs` — `ClassReference` FQN resolution
   - `src/references/mod.rs` — `ClassReference` and `FunctionCall`
     resolution (same-file via `ctx.resolve_name_at()`)
   - `src/rename/mod.rs` — `ClassReference` FQN resolution
   - `src/type_hierarchy.rs` — `ClassReference`, `ClassDeclaration`,
     and `SelfStaticParent` resolution
   - Cross-file reference scanning in `src/references/mod.rs`
     (`find_class_references`, `find_function_references`) — primary
     resolution via `resolved_names.get(span.start)`, lazy `use_map`
     fallback only for offsets not tracked by mago-names (docblock-
     sourced spans whose byte offsets point into comment text).

   **Switched to `self.file_use_map(uri)` helper** (still reads the
   legacy `use_map`, but funnelled through a single method for future
   replacement):
   - `src/diagnostics/deprecated.rs`
   - `src/diagnostics/unknown_classes.rs`
   - `src/diagnostics/unknown_functions.rs`
   - `src/diagnostics/unknown_members.rs`
   - `src/diagnostics/unused_imports.rs`
   - `src/code_actions/import_class.rs`
   - `src/code_actions/phpstan/add_override.rs`
   - `src/code_actions/phpstan/add_throws.rs`
   - `src/code_actions/replace_deprecated.rs`

   **Permanently `use_map`-dependent** (cannot migrate to
   `resolved_names` — see rationale below each group):

   *No byte offset available:*
   - `src/resolution.rs` — `resolve_class_name`, `resolve_function_name`
     are called by loader closures with a bare name string (e.g. a
     parent class name extracted from `ClassInfo`, a type string from
     a docblock).  No source position exists for these lookups.
   - `src/completion/` — loader closures (`class_loader`,
     `function_loader`) thread through `FileContext.use_map` for the
     same reason.  Additionally, class-name completion, auto-import,
     and docblock generation need the full declared-import table to
     avoid inserting duplicate `use` statements.

   *Needs full declared-import table:*
   - `src/diagnostics/unused_imports.rs` — must see imports that are
     declared but *not* referenced; `resolved_names` only contains
     names that *are* referenced.
   - `src/code_actions/import_class.rs` — must check whether a `use`
     statement already exists for a class before inserting one.
   - `src/rename/mod.rs` (`build_class_rename_edit`) — needs the
     full import table to detect aliases and collisions when
     rewriting `use` statements.

   **Key finding:** The legacy `use_map` cannot be fully removed
   because (a) many consumers resolve names without byte offsets
   (loader closures, docblock type strings), and (b) several features
   require the full set of *declared* imports, not just *referenced*
   ones.  The `use_map` must remain until a proper declared-imports
   structure (possibly from `mago-names` scope data or a dedicated
   AST walk) is introduced.

6. **Deprecate and remove `use_map`.** *(blocked — see step 5)*
   Full removal requires a declared-imports data structure that
   captures all `use` statements (including unused ones and aliases)
   independently of `resolved_names`.  Until then, `use_map` stays
   as the canonical import table.  Potential approaches:
   - Extract declared imports from `mago-names` scope data (if
     exposed in a future version).
   - Build a lightweight `DeclaredImports` struct from the AST
     `Statement::Use` nodes during `update_ast_inner`, replacing
     the current `extract_use_items` call.

7. ✅ **Keep `namespace_map` for now.**
   The per-file namespace is still needed for PSR-4 resolution and
   class index construction. `mago-names` doesn't expose the file's
   namespace as a standalone value, so keep `namespace_map` or extract
   the namespace from the AST directly (it's trivial — first
   `Statement::Namespace` node).

8. **Update unused-import diagnostics.** *(deferred)*
   `mago-names` provides `is_imported()` for each resolved name. An
   unused import is a `use` statement whose imported names never
   appear in `ResolvedNames` with `imported = true`. This may
   simplify the current `unused_imports.rs` logic.  However, as noted
   in step 5, `resolved_names` only tracks *referenced* names, so
   detecting *unreferenced* imports requires cross-referencing with
   the declared-import list (still sourced from `use_map`).

9. ✅ **Run the full test suite.**
   All 2735 unit tests + 254 fixture tests pass after each step.

### Interaction with M4

M4 (mago-type-syntax) does NOT depend on mago-names. Type expression
parsing is purely syntactic — it takes a string and returns a type
AST. However, once both M3 and M4 are complete, the combination
enables a powerful pattern: resolve an identifier's FQN via
mago-names, then parse its docblock type via mago-type-syntax, and
work with fully-resolved structured types throughout. This is
especially valuable for the Laravel provider, where a relationship
return type like `HasMany<Post, $this>` needs both FQN resolution
(what is `Post`?) and type structure (what are the generic args?).

---

## M4. Migrate to `mago-type-syntax`

**What it replaces:** The string-based type pipeline — approximately
4,700 lines across:

- `src/docblock/type_strings.rs` (~630 lines — `split_type_token`,
  `split_union_depth0`, `clean_type`, `base_class_name`,
  `replace_self_in_type`, etc.)
- `src/docblock/generics.rs` (~230 lines — `parse_generic_args`,
  `extract_generic_value_type`, etc.)
- `src/docblock/shapes.rs` (~340 lines — `parse_array_shape`,
  `parse_object_shape`, etc.)
- `src/docblock/callable_types.rs` (~290 lines —
  `extract_callable_return_type`, `extract_callable_param_types`, etc.)
- `src/docblock/conditional.rs` (~215 lines —
  `extract_conditional_return_type`, `parse_conditional_expr`)
- Scattered `split_type_token` / `split_union_depth0` calls
  throughout `src/hover/`, `src/completion/`, `src/resolution.rs`,
  and `src/symbol_map/docblock.rs`.

**Why:** Every type in the system is `Option<String>`. Consumers
decompose these strings with hand-written depth-tracking parsers
(counting `<>`, `{}`, `()` nesting) at every use site. This is
fragile, repetitive, and makes it impossible to add features like
conditional-type evaluation, generic type substitution, or type
compatibility checks without yet more string surgery.

`mago-type-syntax` provides `PhpType` — a structured enum that
represents unions, intersections, generics, callables, shapes,
conditionals, etc. as a tree. One parse at extraction time; pattern
matching everywhere else.

**Risk:** Very high blast radius. Every struct that carries a type
field (`ParameterInfo::type_hint`, `MethodInfo::return_type`,
`PropertyInfo::type_hint`, `ConditionalReturnType::Concrete`, etc.)
is affected. The phased approach below is designed to make this
manageable.

### Phase 1: Introduce the type representation ✅

**Goal:** Define `PhpType`, implement `PhpType::parse()` and
`PhpType::to_string()`, and prove they round-trip correctly against
the existing string pipeline. No existing code changes.

**Status:** Complete. Implemented in `src/php_type.rs` (964 lines,
33 tests). All CI checks pass.

**Implementation notes:**

- File is `src/php_type.rs` (not `src/types/php_type.rs` as
  originally planned — avoids restructuring `types.rs` into a
  directory module).
- Module registered as `pub mod php_type` in `lib.rs`.
- `mago-type-syntax = "1.14"` added to `Cargo.toml`.
- The enum has 17 variants (Named, Nullable, Union, Intersection,
  Generic, Array, ArrayShape, ObjectShape, Callable, Conditional,
  ClassString, InterfaceString, KeyOf, ValueOf, IntRange,
  IndexAccess, Literal, Raw) plus helper structs `ShapeEntry` and
  `CallableParam`.
- Callable variant stores `kind: String` to distinguish callable,
  Closure, pure-callable, pure-Closure.
- Conditional variant stores `negated: bool` for `is not` syntax.
- Union/Intersection trees from mago's binary AST are flattened
  into `Vec<PhpType>`.
- All 34 mago keyword types map to `Named("keyword")`.
- Unhandled mago variants (int-mask, int-mask-of, properties-of,
  alias-reference, member-reference, negated, posited) fall back
  to `Raw(ty.to_string())`.
- Display works around mago Display bugs for class-string, key-of,
  value-of (double angle brackets in mago's output).
- 33 tests: 14 round-trip, 2 error-handling, 15 structural
  verification (flattening, field values, etc.).

Original plan steps (all done):

1. ✅ Add `mago-type-syntax` to `Cargo.toml`.
2. ✅ Create `src/php_type.rs` with `PhpType` enum.
3. ✅ Implement `PhpType::parse(s: &str) -> PhpType`.
4. ✅ Implement `Display for PhpType`.
5. ✅ Write round-trip tests.

### Phase 2: Dual representation on core types ✅

**Goal:** Add `_parsed: Option<PhpType>` fields alongside existing
string fields on the core types. Populate them at extraction time.
No consumers change yet.

**Status:** Complete. All four core types carry a parsed field
populated via `PhpType::parse()` at every construction site. All CI
checks pass (cargo test, clippy, clippy --tests, fmt, php -l).

**Implementation notes:**

- `MethodInfo::return_type_parsed: Option<PhpType>` populated in
  `src/parser/classes.rs` (`extract_class_like_members`),
  `src/docblock/virtual_members.rs` (`extract_method_tags`), and
  `MethodInfo::virtual_method`.
- `ParameterInfo::type_hint_parsed: Option<PhpType>` populated in
  `src/parser/mod.rs` (`extract_parameters`),
  `src/parser/classes.rs` (extra `@param` tags),
  `src/parser/functions.rs` (extra `@param` tags), and
  `src/docblock/virtual_members.rs` (`extract_method_tag_params`).
- `PropertyInfo::type_hint_parsed: Option<PhpType>` populated in
  `src/parser/mod.rs` (`extract_property_info`),
  `src/parser/classes.rs` (promoted constructor properties),
  `src/completion/types/resolution.rs` (array-shape entries),
  `src/virtual_members/phpdoc.rs` (`@property` tags), and
  `PropertyInfo::virtual_property`.
- `ConstantInfo::type_hint_parsed: Option<PhpType>` populated in
  `src/parser/classes.rs` (class constants and enum cases).
- Test construction sites use `None` for the parsed field since
  they don't exercise type-structural consumers yet.
- The `_parsed` field is placed after the corresponding string
  field in struct definitions and before it in struct literals
  (to avoid borrow-after-move when the string field uses
  shorthand initialization).

Original plan steps (all done):

1. ✅ Add `return_type_parsed: Option<PhpType>` to `MethodInfo`.
2. ✅ Add `type_hint_parsed: Option<PhpType>` to `ParameterInfo`.
3. ✅ Add `type_hint_parsed: Option<PhpType>` to `PropertyInfo`.
4. ✅ Add `type_hint_parsed: Option<PhpType>` to `ConstantInfo`.
5. ✅ Populate these fields in `src/parser/classes.rs` and
   `src/parser/functions.rs` by calling `PhpType::parse()` on the
   existing string value.

At this point, every extracted type has both representations. Old
code keeps reading the string field; new code can read the parsed
field.

### Phase 3: Migrate consumers to structured types

**Goal:** Replace string-based type manipulation with `PhpType`
pattern matching, one module at a time. After each module is
migrated, its string-field reads are removed.

Modules in recommended migration order (least dependencies first):

1. ✅ **`src/hover/`** — Type display and structural operations.

   **Status:** Complete. Structural type operations migrated to
   `PhpType`; display formatting kept on `shorten_type_string` to
   preserve callable parameter names and source-level
   parenthesization. All 236 hover integration tests pass.

   **What changed:**

   - `build_variable_hover_body` uses `PhpType::parse()` +
     `union_members()` instead of `split_top_level_union` (deleted).
   - `build_variable_hover_body` uses `PhpType::is_scalar()` instead
     of `docblock::type_strings::is_scalar`.
   - `resolve_type_namespace` replaced by
     `resolve_type_namespace_structured` which uses
     `PhpType::base_name()` instead of string surgery.
   - `build_var_annotation` and `build_param_return_section` use
     `PhpType::equivalent()` instead of `types_equivalent` for
     type comparison.
   - Template bound display (3 sites) uses
     `PhpType::parse(bound).shorten()` instead of
     `shorten_type_string(bound)`.
   - `shorten_type_string` and `types_equivalent` kept as exports
     for `completion/builder.rs` and other modules not yet migrated.

   **Design decision:** `PhpType::shorten().to_string()` drops
   callable parameter names (`$item`) and changes union spacing
   (`|` → ` | `). For display in hover popups, the old
   `shorten_type_string` is kept because it preserves the original
   format character-by-character. `PhpType` is used only for
   structural operations (union splitting, equivalence checks,
   scalar detection, base-name extraction).

   **New `PhpType` helper methods** added in this step:
   - `shorten()` — produce a new `PhpType` with all FQNs shortened
   - `is_scalar()` — whether a type is a built-in / non-class type
   - `base_name()` — extract the base class name (if any)
   - `union_members()` — return top-level union members as a vec
   - `equivalent()` — compare two types ignoring namespace differences

2. ✅ **`src/completion/`** — Type matching for member access.

   **Status:** Complete. All `extract_generic_value_type`,
   `extract_generic_key_type`, and several `clean_type` call sites
   migrated to `PhpType` methods. All 3,400+ tests pass.

   **What changed:**

   - `src/hover/variable_type.rs` — foreach value/key extraction
     uses `PhpType::extract_value_type(true)` /
     `PhpType::extract_key_type(true)` instead of
     `docblock::types::extract_generic_value_type` /
     `extract_generic_key_type`.
   - `src/completion/variable/foreach_resolution.rs` — 4 call sites
     migrated from `extract_generic_value_type` /
     `extract_generic_key_type` to `PhpType::extract_value_type` /
     `extract_key_type`.
   - `src/completion/variable/raw_type_inference.rs` — 8 call sites
     migrated: all `extract_generic_value_type` calls, plus
     `clean_type`/`is_scalar` in `extract_array_map_element_type`
     replaced with `PhpType::parse().base_name()`.
   - `src/completion/variable/rhs_resolution.rs` — 4 call sites:
     `classify_template_binding` and `resolve_rhs_array_access`
     use `PhpType::base_name()` and `extract_value_type`.
     `resolve_rhs_property_access` uses `PhpType::base_name()`.
   - `src/completion/variable/resolution.rs` — 1 call site migrated
     (`resolve_arg_raw_type` uses `PhpType::extract_value_type`).
   - `src/completion/call_resolution.rs` — 1 call site migrated.
   - `src/completion/source/helpers.rs` — `walk_array_segments_and_resolve`
     uses `PhpType::extract_value_type` for element access and
     `PhpType::is_scalar()` for the final type check. Two
     `resolve_lhs_to_class` sites kept on `clean_type` for now
     (they handle unions/nullable types that `base_name()` can't
     collapse).

   **New `PhpType` helper methods** added in this step:
   - `extract_value_type(skip_scalar)` — extract the value type from
     generics/arrays (last param, or 2nd for Generator)
   - `extract_key_type(skip_scalar)` — extract the key type from
     2+-param generics
   - `extract_element_type()` — convenience for
     `extract_value_type(false)`
   - `intersection_members()` — return top-level intersection members

   **Design decision:** `clean_type` is a Swiss-army-knife function
   that strips `?`, leading `\`, trailing punctuation, extracts
   non-null from unions, and strips generics. It cannot be replaced
   by a single `PhpType` method. Call sites where `clean_type` is
   used purely for base-name extraction were migrated to
   `PhpType::base_name()`. Call sites where `clean_type` handles
   union collapsing (e.g. `User|null` → `User`) were kept on
   `clean_type` since `base_name()` returns `None` for unions.

3. **`src/resolution.rs`** — `resolve_type_string`. Replace the
   string-surgery approach (split on `|`, recurse, rejoin) with
   tree traversal on `PhpType`.

4. **`src/docblock/types.rs` and sub-modules** — The old string
   parsers. Once all consumers use `PhpType`, these become dead code
   and can be deleted.

5. **`src/symbol_map/docblock.rs`** — `emit_type_spans`. Replace the
   423-line recursive string decomposer with `PhpType` tree
   traversal + span emission. (The tag-level migration is already
   done; this is the type-level migration.)

6. **`src/diagnostics/`** — Type compatibility checks. Pattern match
   on `PhpType` variants instead of string prefix checks.

7. **`src/code_actions/`** — Type-aware refactorings. Use `PhpType`
   for type comparison, docblock generation, etc. Currently,
   `ResolvedType::type_strings_joined` joins all resolved types
   with `|`, which flattens intersection types (`A&B`) into unions
   (`A|B`). With `PhpType::Intersection` this is preserved.

### Phase 4: Migrate the Laravel provider

The Laravel provider (`src/laravel/`) has its own type manipulation
for Eloquent models, relationships, collections, and facades.

1. **Eloquent attribute types** — Replace string-based cast-type
   mapping with `PhpType` construction.

2. **Relationship return types** — Replace the string template
   `"HasMany<{model}>"` with `PhpType::Generic("HasMany",
   [PhpType::Named(model)])`.

3. **Collection generics** — Replace `format!("Collection<{},
   {}>", key, value)` with `PhpType::Generic` construction.

4. **Facade accessor resolution** — The `getFacadeAccessor` →
   class lookup produces a class name string. This stays as a string
   (it's a class name, not a type expression), but the *return type*
   it produces can be `PhpType::Named`.

### Phase 5: Remove string type fields

Once all consumers read `_parsed` fields:

1. Remove `return_type: Option<String>` from `MethodInfo` (rename
   `return_type_parsed` → `return_type`).
2. Remove `type_hint: Option<String>` from `ParameterInfo`,
   `PropertyInfo`, `ConstantInfo` (rename similarly).
3. Remove `native_return_type: Option<String>` /
   `native_type_hint: Option<String>` — these become
   `PhpType::Named` values populated from the AST hint.
4. Delete `src/docblock/type_strings.rs`, `generics.rs`, `shapes.rs`,
   `callable_types.rs` — the old string parsers.
5. Delete `ConditionalReturnType` enum (replaced by
   `PhpType::Conditional`).
6. Run the full test suite.

---

## Testing strategy

Each migration step (M3, M4) must pass the **full existing test
suite** before merging. This is the primary safety net.

Additional testing per migration:

| Migration | Extra tests |
| --------- | ----------- |
| M3 | Snapshot tests comparing `use_map`-based resolution with `OwnedResolvedNames`-based resolution across the fixture corpus. |
| M4 | Round-trip tests (`PhpType::parse(s).to_string() == s`) for every type string in the test suite. Per-module migration tests comparing old string-based output with new `PhpType`-based output. |

For M4 specifically, the dual-representation phase (Phase 2) enables
**shadow testing**: compute the result both ways and assert they
match, before removing the old path. This catches regressions without
blocking progress.

---

## Version alignment

All Mago crates should be pinned to the same release. At the time of
writing, the latest version is **1.15.x**. When upgrading, update all
Mago crates in a single commit and run the test suite.

The `mago-docblock` crate is already present in `Cargo.toml`. When
adding `mago-type-syntax` and `mago-names`, align them to the same
version.

---

## What this enables

Once M3 + M4 are complete:

- **Structured types everywhere.** No more string surgery for type
  manipulation. Generic substitution, conditional evaluation, and
  type compatibility checks become tree operations.

- **Correct name resolution.** Every identifier resolves to its FQN
  in a single pass. Auto-import and unused-import diagnostics become
  straightforward.

- **Foundation for advanced features.** Laravel Eloquent attribute
  completion, Blade template support, and PHPStan-level type
  inference all require structured types and correct name resolution.
  Building them on the new foundation avoids double work.

- **Reduced maintenance burden.** ~6,500+ lines of hand-written
  parsers replaced by well-tested upstream crates.