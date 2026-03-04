# PHPantom — Signature Help: Improvement Plan

Signature help is architecturally solid — dual-path detection (AST-based
`CallSite` lookup + text-based fallback), precomputed comma offsets for
active parameter tracking, content patching for unclosed parens, and
chain/constructor/first-class-callable resolution all work well.

The remaining work is almost entirely **presentation-layer wiring**: the
data needed for rich signature help already exists on `ParameterInfo`,
`MethodInfo`, and `FunctionInfo` (added during the hover overhaul), but
`build_signature` and `ResolvedCallableTarget` don't propagate it to the
LSP response yet.

Items are ordered by impact (descending), then effort (ascending).

---

<!-- ============================================================ -->
<!--  TIER 1 — WIRING (data exists, just needs plumbing)          -->
<!-- ============================================================ -->

## Tier 1 — Wire Existing Data

### 1. Per-parameter `@param` descriptions
**Impact: High · Effort: Trivial**

When the cursor is on parameter 2 of `array_map`, the editor should show
the description from `@param callable $callback Callback function to run
for each element` in a documentation popup beside the parameter label.

#### Current state

`ParameterInfo` already has `description: Option<String>`, populated by
`extract_param_description` in `docblock/tags.rs` (with HTML stripping
and multi-line continuation support).  Hover uses it in
`build_param_return_section`.

`build_signature` in `signature_help.rs` sets `documentation: None` on
every `ParameterInformation`.

#### Implementation

In `build_signature`, zip the `param_labels` iterator with the `params`
slice and populate `documentation` from `param.description`:

```rust
// signature_help.rs — build_signature
for (idx, (pl, param)) in param_labels.iter().zip(params.iter()).enumerate() {
    let start = offset as u32;
    let end = (offset + pl.len()) as u32;
    let doc = param.description.as_ref().map(|d| {
        Documentation::MarkupContent(MarkupContent {
            kind: MarkupKind::Markdown,
            value: d.clone(),
        })
    });
    param_infos.push(ParameterInformation {
        label: ParameterLabel::LabelOffsets([start, end]),
        documentation: doc,
    });
    // ... offset bookkeeping unchanged
}
```

#### Tests

- `signature_help.rs` unit test: `build_signature` with a `ParameterInfo`
  that has `description: Some("The callback.")` → assert
  `ParameterInformation.documentation` is `Some(MarkupContent { .. })`.
- `tests/signature_help.rs` integration test: define a function with a
  `@param` description, request signature help → assert `documentation`
  is present on the correct parameter.

---

### 2. Signature-level documentation (method/function docblock)
**Impact: High · Effort: Small**

When signature help fires, the editor should show the function/method's
docblock description at the top of the popup — e.g. "Perform a regular
expression match" for `preg_match`.

#### Current state

`MethodInfo` and `FunctionInfo` both have `description: Option<String>`,
`return_description: Option<String>`, and `link: Option<String>`.  Hover
renders all three.

`ResolvedCallableTarget` does **not** carry these fields, and
`SignatureInformation` has `documentation: None`.

#### Implementation

1. **Extend `ResolvedCallableTarget`** with three new fields:

   ```rust
   // types.rs
   pub(crate) struct ResolvedCallableTarget {
       pub label_prefix: String,
       pub parameters: Vec<ParameterInfo>,
       pub return_type: Option<String>,
       pub description: Option<String>,         // NEW
       pub return_description: Option<String>,  // NEW
       pub link: Option<String>,                // NEW
   }
   ```

2. **Populate in call_resolution.rs** — in `resolve_instance_method_callable`,
   `resolve_static_method_callable`, `function_to_callable`, and
   `resolve_constructor_callable`, copy the description/link fields from
   the resolved `MethodInfo` or `FunctionInfo`.

3. **Thread through `ResolvedCallable`** — add the same three fields to
   the local `ResolvedCallable` struct and its `From` impl.

4. **Build markdown in `resolve_signature`** — before calling
   `build_signature`, assemble a documentation string:

   ```rust
   let mut doc_parts: Vec<String> = Vec::new();
   if let Some(ref desc) = resolved.description {
       doc_parts.push(desc.clone());
   }
   if let Some(ref ret_desc) = resolved.return_description {
       doc_parts.push(format!("**return** {}", ret_desc));
   }
   if let Some(ref url) = resolved.link {
       doc_parts.push(format!("[{}]({})", url, url));
   }
   let sig_doc = if doc_parts.is_empty() {
       None
   } else {
       Some(Documentation::MarkupContent(MarkupContent {
           kind: MarkupKind::Markdown,
           value: doc_parts.join("\n\n"),
       }))
   };
   ```

5. **Pass to `build_signature`** — add a `documentation: Option<Documentation>`
   parameter (or set it on the returned `SignatureInformation` after the call).

#### Tests

- Unit test: `build_signature` with non-None documentation → assert
  `SignatureInformation.documentation` contains the expected markdown.
- Integration test: stub function `array_map` has a description → assert
  it appears in the signature help response.
- Integration test: user-defined method with `@return` description →
  assert it appears.
- Integration test: stub function with `@link` → assert the URL appears.

---

### 3. Default values in parameter labels
**Impact: Medium · Effort: Trivial**

Optional parameters should display their default value in the signature
label: `int $limit = 10` rather than bare `int $limit`.  This tells the
user the parameter is optional and what happens when they omit it.

#### Current state

`ParameterInfo` already has `default_value: Option<String>`, extracted
from the AST span in `parser/mod.rs`.  Hover's `format_params_inner`
already renders `= value` for optional params.

`format_param_label` in `signature_help.rs` ignores `default_value`.

#### Implementation

At the end of `format_param_label`, append the default value:

```rust
fn format_param_label(param: &ParameterInfo) -> String {
    let mut parts = Vec::new();
    if let Some(ref th) = param.type_hint {
        parts.push(th.clone());
    }
    if param.is_variadic {
        parts.push(format!("...{}", param.name));
    } else if param.is_reference {
        parts.push(format!("&{}", param.name));
    } else {
        parts.push(param.name.clone());
    }
    let base = parts.join(" ");
    if !param.is_required && !param.is_variadic {
        if let Some(ref dv) = param.default_value {
            return format!("{} = {}", base, dv);
        }
    }
    base
}
```

**Note:** This changes the label string length, which affects
`ParameterLabel::LabelOffsets`.  The existing offset calculation in
`build_signature` already uses `pl.len()` from the label, so no
additional changes are needed — the offsets track automatically.

#### Tests

- Unit test: `format_param_label` with `default_value: Some("10")` →
  assert result is `"int $limit = 10"`.
- Unit test: `format_param_label` with `default_value: Some("null")` →
  assert result is `"?string $name = null"`.
- Unit test: `format_param_label` with `default_value: None` and
  `is_required: false` → assert no ` = ...` suffix (no default known).
- Integration test: function with `function greet(string $name = 'World')` →
  assert label contains `= 'World'`.
- Update `build_signature_label` test to use params with default values
  and verify the full label.

---

<!-- ============================================================ -->
<!--  TIER 2 — NEW EXTRACTION                                     -->
<!-- ============================================================ -->

## Tier 2 — New Extraction Work

### 4. Attribute constructor signature help
**Impact: Medium · Effort: Medium**

PHP 8 attributes take constructor arguments:

```php
#[Route('/users', methods: ['GET'])]
class UserController {}
```

Signature help should fire inside the attribute's parentheses and show
the attribute class's `__construct` parameters — the same as `new Route(`.

#### Current state

`emit_call_site` in `symbol_map/extraction.rs` only handles
`CallExpression`, `ObjectCreationExpression`, and their variants.
`Attribute` nodes are not visited for call-site emission.

#### Implementation

1. **Emit `CallSite` for attributes** — in `symbol_map/extraction.rs`,
   add handling in the attribute extraction path.  When an `Attribute`
   node has an `argument_list`, emit a `CallSite` with:
   - `call_expression: format!("new {}", attr_name)` — so the existing
     constructor resolution path picks it up.
   - `args_start` / `args_end` from the attribute's argument list parens.
   - `comma_offsets` from the argument list's separator tokens.

2. **Resolve the attribute name** — the attribute name must be resolved
   through the file's use-map (same as class references).  The existing
   `CallSite` resolution in `resolve_callable_target` handles `new ClassName`
   and resolves it via the class loader, so this should work automatically.

3. **Edge case: nested attributes** (PHP 8.1) — `#[Outer(new Inner(...))]`
   should show `Inner`'s constructor when the cursor is inside `Inner(`.
   This should work naturally since `ObjectCreationExpression` inside
   attribute argument lists is already handled.

#### Tests

- Unit test: `extract_symbol_map` on `#[FooAttr($x, ` → assert a
  `CallSite` with `call_expression: "new FooAttr"` and correct
  `args_start` / `comma_offsets`.
- Integration test: define an attribute class with `__construct(string $path, array $methods)`,
  use it as `#[FooAttr(`, request signature help → assert the constructor
  parameters appear.
- Integration test: cursor on second parameter `#[FooAttr('/path', ` →
  assert `active_parameter` is 1.
- Integration test: nested `#[Outer(new Inner(` → assert Inner's
  constructor is shown.

---

### 5. Closure / arrow function parameter signature help
**Impact: Medium · Effort: Medium**

Signature help should work when invoking a variable that holds a closure
or arrow function:

```php
$format = fn(string $name, int $age): string => "$name ($age)";
$format('Alice', 30);  // ← signature help here
```

#### Current state

`extract_callable_target_from_variable` handles first-class callables
(`$fn = makePen(...)`) by scanning for the `(...)` suffix.  Closures
and arrow functions assigned to variables are not detected because they
don't end with `(...)`.

#### Implementation

1. **Detect closure/arrow assignments** — in
   `extract_callable_target_from_variable`, if the RHS does not end with
   `(...)`, check whether it starts with `function(` or `fn(`.  If so,
   return a synthetic identifier (e.g. `"__closure_at_L{line}"`) that
   the resolver can look up.

2. **Parse closure parameters** — alternatively, skip the
   `resolve_callable_target` pathway entirely.  When the variable is
   assigned a closure/arrow function, parse the parameters and return
   type directly from the AST of the assignment RHS.  Build the
   `ResolvedCallableTarget` inline without going through class
   resolution.

   This is the cleaner approach: closures don't have classes, so the
   existing class-based resolution is the wrong abstraction.  The
   `SymbolMap` already records `VarDefSite` for the assignment, and the
   AST is available.

3. **Label prefix** — use `$format` (the variable name) or the closure's
   inferred signature as the label prefix.

#### Tests

- Integration test: `$fn = fn(string $x): int => 0; $fn(` → assert
  signature help shows `string $x` with return type `int`.
- Integration test: `$fn = function(int $a, int $b): int { ... }; $fn('x', ` →
  assert `active_parameter` is 1.
- Integration test: `$fn = $obj->method(...)` (existing first-class
  callable path) → continues to work unchanged.

---

<!-- ============================================================ -->
<!--  TIER 3 — POLISH                                             -->
<!-- ============================================================ -->

## Tier 3 — Polish

### 6. Retrigger on `)` to dismiss
**Impact: Low · Effort: Trivial**

When the user types `)`, the signature help popup should dismiss.  Some
editors handle this automatically, but adding `)` to `retriggerCharacters`
ensures consistent behaviour across all clients.

#### Implementation

In `server.rs`, add `)` to `retrigger_characters`:

```rust
retrigger_characters: Some(vec![",".to_string(), ")".to_string()]),
```

The existing detection logic already returns `None` when the cursor is
outside the argument list, so the popup dismisses naturally.

---

### 7. Multiple overloaded signatures
**Impact: Low · Effort: Medium-High**

Some PHP functions have multiple signatures depending on argument count
or types.  For example, `array_map` can be called as:

```php
array_map(callable $callback, array $array): array
array_map(null, array ...$arrays): array
```

The LSP protocol supports returning multiple `SignatureInformation`
entries with an `activeSignature` index.  Today we return a single
signature.

#### Current state

phpstorm-stubs define multiple function entries (or parameter variants
annotated with `#[PhpStormStubsElementAvailable]`) for overloaded
functions.  Our PHP-version filtering selects one variant.  We don't
model true overloads.

#### Implementation

This is a deeper change:

1. When a function has multiple stub entries (or when a class has
   multiple `__construct` signatures for different PHP versions),
   collect all applicable signatures.
2. Return them all in the `signatures` array.
3. Set `activeSignature` based on argument-count matching: pick the
   first signature whose parameter count accommodates the current
   argument count.

**Deferred** — the single-signature approach covers 99% of real usage.

---

### 8. Named argument awareness in active parameter
**Impact: Low · Effort: Medium**

When the user types a named argument (`callback: ` in `array_map(callback: `),
the active parameter should highlight the `$callback` parameter regardless
of its positional index.

#### Current state

Active parameter is computed purely by counting commas before the cursor.
Named arguments are handled by the named-argument completion system
(`completion/named_args.rs`) but the signature help active-parameter
tracking doesn't consult argument names.

#### Implementation

1. In `detect_call_site_from_map`, after computing the comma-based
   `active` index, extract the text of the current argument segment.
2. If the segment matches `identifier:` (named argument syntax), look up
   which parameter index corresponds to that name.
3. Override `active_parameter` with the named parameter's index.

This requires access to the resolved parameters (to map name → index),
which isn't available in the detection layer.  The override could be
applied later in `resolve_signature`, after `resolve_callable` returns
the parameter list.

---

## Summary

| # | Item | Impact | Effort | Data Ready | Target |
|---|---|---|---|---|---|
| 1 | Per-parameter `@param` descriptions | High | Trivial | ✅ | Sprint 1 |
| 2 | Signature-level documentation | High | Small | ✅ | Sprint 1 |
| 3 | Default values in labels | Medium | Trivial | ✅ | Sprint 1 |
| 4 | Attribute constructor sig help | Medium | Medium | ❌ | Sprint 2 |
| 5 | Closure/arrow function sig help | Medium | Medium | ❌ | Sprint 2 |
| 6 | Retrigger on `)` | Low | Trivial | N/A | Sprint 1 |
| 7 | Multiple overloaded signatures | Low | Medium-High | ❌ | Backlog |
| 8 | Named argument active parameter | Low | Medium | ❌ | Backlog |

Sprint 1 (items 1–3, 6) is pure wiring — no new extraction, no new
resolution paths.  Estimated effort: half a day.

---

## 9. Language construct signature help and hover
**Impact: Low · Effort: Low**

PHP language constructs that use parentheses (`unset()`, `isset()`, `empty()`,
`eval()`, `exit()`, `die()`, `print()`, `list()`) are not function calls in the
AST. Mago parses them as dedicated statement/expression nodes (e.g.
`Statement::Unset`) with no `ArgumentList`, so no `CallSite` is emitted and
neither signature help nor hover fires inside their parentheses. The phpstorm-stubs
don't define them either since they are keywords, not functions.

Supporting them requires emitting synthetic `CallSite` entries from the
statement-level extraction in `symbol_map.rs` and adding hardcoded parameter
metadata (e.g. `unset(mixed ...$vars): void`) in `resolve_callable`. Hover would
need a similar hardcoded lookup.