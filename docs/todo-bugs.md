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
**Impact: Low · Effort: Low**

`class_implements_or_extends` matches interfaces by both short name and
FQN (`iface_short == target_short || iface == target_fqn`).  Two
interfaces in different namespaces with the same short name (e.g.
`App\Logger` and `Vendor\Logger`) could produce false positives.
Similarly, `seen_names` in `find_implementors` deduplicates by short
name, so two classes with the same short name in different namespaces
could shadow each other.

**Fix:** always compare fully-qualified names by resolving both sides
before comparison.

---

## 2. GTD fires on parameter variable names and class declaration names
**Impact: Medium · Effort: Low**

Go-to-definition fires on parameter variable names (`$supplier`, `$country`)
and class declaration names (`class Foo`), navigating to the same location —
the cursor is already at the definition. This is noisy and unexpected:
clicking a parameter name or a class declaration name should either do
nothing or offer a different action (e.g. find references).

### Current behaviour

- **Parameter names:** Ctrl+Click on `$supplier` in a method signature
  jumps to… `$supplier` in the same method signature. The `VarDefSite`
  with `kind: Parameter` is correctly recorded, and `find_var_definition`
  returns it — so the "definition" is the cursor's own position.

- **Class declarations:** Ctrl+Click on `Foo` in `class Foo {` jumps to
  the same `Foo` token. The `SymbolMap` records a `ClassDeclaration`
  span, and `resolve_definition` resolves it to the same file and offset.

### Fix

In the definition handler, after resolving the definition location, check
whether the target location is the same as (or within a few bytes of) the
cursor position. If so, return `None` — there is no useful jump to make.

Alternatively, suppress at the `SymbolKind` level:
- For `Variable` spans where `var_def_kind_at` returns `Some(Parameter)`,
  skip definition.
- For `ClassDeclaration` spans, skip definition.

### Tests to update

Several existing definition tests assert that parameter names and class
declarations produce a definition result pointing to themselves. These should
expect `None` instead.

---

## 3. Relationship classification matches short name only
**Impact: Low · Effort: Low**

`classify_relationship` in `virtual_members/laravel.rs` strips the
return type down to its short name (via `short_name`) and matches
against a hardcoded list (`HasMany`, `BelongsTo`, etc.). This means
any class whose short name collides with a Laravel relationship class
(e.g. a custom `App\Relations\HasMany` that does not extend
Eloquent's) would be incorrectly classified as a relationship.

The fix would be to resolve the return type to its FQN (using the
class loader or use-map) and verify it lives under
`Illuminate\Database\Eloquent\Relations\` (or extends a class that
does) before classifying. The short-name-only path could remain as a
fast-path fallback when the FQN is already in the
`Illuminate\Database\Eloquent\Relations` namespace.