# PHPantom — Bug Fixes

#### B21. Builder `__call` return type drops chain type for dynamic `where{Column}` calls

| | |
|---|---|
| **Impact** | Medium |
| **Effort** | Medium |

Eloquent's `Builder::__call()` intercepts calls like `whereColumnName()`
and forwards them to `where('column_name', ...)`. PHPantom correctly
suppresses the "unknown member" diagnostic because `Builder` has `__call`,
but the return type of the magic call is lost. This breaks every
subsequent link in the chain.

**Reproducer:**

```php
// SubcategoryView has scopeWhereLanguage but NOT scopeWhereSubcategoryId.
// whereSubcategoryId() is a dynamic where{Column} via Builder::__call.
$view = SubcategoryView::whereLanguage($lang)
    ->whereSubcategoryId($id)   // accepted (Builder has __call), but return type lost
    ->first();                  // diagnostic: "subject type could not be resolved"
```

**Expected:** When `__call` is invoked on an Eloquent `Builder<TModel>`,
the return type should be `Builder<TModel>` (i.e. `$this`), preserving
the generic parameter so the chain continues resolving.

**Root cause:** `has_magic_method_for_access` in `unknown_members.rs`
correctly detects `__call` and suppresses the diagnostic, but the
call-resolution pipeline in `call_resolution.rs` does not attempt to
derive a return type from `__call`. For Eloquent builders specifically,
any unrecognised instance method call should return `$this` (the
builder), since nearly all `Builder` methods are fluent.

**Where to fix:**
- `src/completion/call_resolution.rs` — when resolving a method call
  that is not found on the class but the class has `__call`, check
  whether the class is an Eloquent `Builder` (or extends one). If so,
  return `$this` / the builder type as the call's return type instead
  of giving up.
- Alternatively, add a fallback in `resolve_method_return_types_with_args`
  that checks for `__call` and uses its declared return type (or `$this`
  for known builder classes).

**Impact in shared codebase:** ~5 diagnostics (direct chain breaks after
dynamic `where{Column}` calls, plus downstream cascading failures).

**Discovered in:** analyze-triage iteration 10.