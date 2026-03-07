# PHPantom — Bug Fixes

Known bugs and incorrect behaviour. These are distinct from feature
requests — they represent cases where existing functionality produces
wrong results. Bugs should generally be fixed before new features at
the same impact tier.

Items are ordered by **impact** (descending), then **effort** (ascending).

| Label | Scale |
|---|---|
| **Impact** | **Critical**, **High**, **Medium-High**, **Medium**, **Low-Medium**, **Low** |
| **Effort** | **Low** (≤ 1 day), **Medium** (2-5 days), **Medium-High** (1-2 weeks), **High** (2-4 weeks), **Very High** (> 1 month) |

---

## 1. Short-name collisions in `find_implementors`
**Impact: Low · Effort: Low (fixed)**

**Status:** Fixed. `class_implements_or_extends` now compares
fully-qualified names when a namespace is available. The short-name
fallback is only used when FQN information is absent. `seen_fqns` in
`find_implementors` deduplicates by FQN (built from `name` +
`file_namespace`) instead of by short name.

---

## 2. GTD fires on parameter variable names and class declaration names
**Impact: Medium · Effort: Low (fixed)**

**Status:** Fixed. Three layers suppress self-referential jumps:

1. `resolve_from_symbol` returns `None` for `ClassDeclaration` and
   `MemberDeclaration` symbol kinds (the cursor is at the definition).
2. `lookup_var_def_kind_at` detects when the cursor is on a variable
   at its definition site (parameter, assignment LHS, foreach binding,
   catch binding) and returns `None` before `find_var_definition` runs.
3. A self-reference guard in `resolve_definition` suppresses jumps
   when the resolved location points back to the cursor position.

Tested with `test_goto_definition_parameter_at_definition_returns_none`
and related tests in `definition_variables.rs`.

---

## 3. Relationship classification matches short name only
**Impact: Low · Effort: Low (fixed)**

**Status:** Fixed. `classify_relationship` now checks whether a
namespace-qualified return type lives under
`Illuminate\Database\Eloquent\Relations\` before classifying.
Unqualified short names (the common case for body-inferred types and
use-imported docblock annotations) still match by short name only.
A custom `App\Relations\HasMany` is no longer misclassified.

---

## 4. Go-to-implementation misses transitive implementors
**Impact: Medium · Effort: Medium (fixed)**

**Status:** Fixed. `class_implements_or_extends` already walks the
parent class chain transitively (up to `MAX_INHERITANCE_DEPTH`) and
checks interface-extends chains recursively. Classes that extend a
concrete class which itself implements the target interface are found
correctly. Tested with `test_implementation_transitive_via_parent`,
`test_implementation_skips_abstract_subclasses`, and deep interface
inheritance chains.

---

## 5. Go-to-implementation Phase 5 should only walk user PSR-4 roots
**Impact: Low · Effort: Low (fixed)**

**Status:** Fixed. PSR-4 mappings now come exclusively from
`composer.json` (user code only). Vendor PSR-4 mappings are no longer
loaded (see §7), so Phase 5 inherently walks only user roots.

---

## 6. Go-to-definition does not check the classmap
**Impact: Medium · Effort: Low (fixed)**

**Status:** Fixed. `resolve_class_reference`, `resolve_self_static_parent`,
and `resolve_type_hint_string_to_location` now check the Composer classmap
(FQN → file path) between the class_index lookup and the PSR-4 fallback.
A cold Ctrl+Click on a vendor class resolves through the classmap without
needing vendor PSR-4 mappings.

---

## 7. Vendor PSR-4 mappings removed
**Impact: Low · Effort: Low (fixed)**

**Status:** Fixed. `parse_vendor_autoload_psr4` has been removed.
`parse_composer_json` no longer reads `vendor/composer/autoload_psr4.php`.
PSR-4 mappings come exclusively from the project's own `composer.json`
(`autoload.psr-4` and `autoload-dev.psr-4`). The `is_vendor` flag on
`Psr4Mapping` has been removed.

All resolution paths that could hit a vendor class now check the classmap
first (§6). If the classmap is missing or stale, vendor classes fail to
resolve visibly (fix: run `composer dump-autoload`). This reduces startup
time and memory for projects with large dependency trees.

**Note for Rename Symbol:** when rename support is implemented, the
handler should reject renames for symbols whose definition lives under
the vendor directory. The user cannot meaningfully rename third-party
code. Use `vendor_uri_prefix` to detect this and return an appropriate
error message.

---

## 13. Evict transiently-loaded files from ast_map after GTI and Find References
**Impact: Low · Effort: Low (fixed)**

**Status:** Fixed. `find_implementors` snapshots `ast_map` keys before
scanning and calls `evict_transient_entries` afterwards, removing any
`ast_map`, `symbol_maps`, `use_map`, and `namespace_map` entries that
were added during the scan and are not in `open_files`.
`ensure_workspace_indexed` does the same via
`evict_transient_ast_entries`, which preserves `symbol_maps` (since
those are the purpose of the indexing scan) but evicts the other maps.

---

## 14. Signature help fires on function definition sites
**Impact: Low · Effort: Low**

Signature help triggers when the cursor is inside the parameter list of
a function *definition* (e.g. `function unionDemo(string|int $value, ?User $maybe)`).
Method definitions already suppress this, but standalone function
definitions do not receive the same treatment.

The fix should mirror the method-definition suppression: detect that the
cursor is inside a function declaration's parameter list and return
`None` before attempting signature help resolution.