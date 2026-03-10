# PHPantom — Refactoring

Technical debt and internal cleanup tasks. This document is a gate check
between sprints: before starting a new sprint, populate it with anything
that should be cleaned up first, then iterate until the list is empty.

> **Housekeeping:** When a task is completed, remove it from this
> document entirely. Do not strike through or mark as done. When the
> document is empty (no tasks remaining), the gate is clear and the
> next sprint can begin.

## How to use this document

1. **End of sprint.** After completing a sprint, review the codebase for
   technical debt introduced during the sprint. Add items here.
2. **Pre-sprint review.** Before starting the next sprint, scan for
   structural issues that would make the new work harder. Add items here.
3. **Clear the gate.** Work through every item. Remove each one as it is
   completed. The next sprint begins when no items remain.

Not every piece of debt needs to go here. Only add items that would
actively hinder upcoming work or that have accumulated enough friction
to justify a focused cleanup pass. Small fixes that can be done inline
during feature work should just be done inline.

---

## 1. Canonicalize FQN representation

**Effort: High (2-4 weeks)**

Class names flow through the system in three forms that are never
clearly distinguished:

- **Short name** — `HtmlCast` (no namespace, from `use` imports or
  same-file references)
- **FQN without prefix** — `App\Casts\HtmlCast` (fully qualified but
  no leading `\`)
- **FQN with prefix** — `\App\Casts\HtmlCast` (PHP's absolute syntax)

There is no single canonical representation. Every call site that
receives a class name must guess which form it is in and defensively
`strip_prefix('\\')` or `starts_with('\\')` before comparing. As of
this writing there are ~96 `strip_prefix('\\')` calls scattered across
the codebase. Each one is a potential bug if a new code path forgets
to normalize, and the pattern has caused recurring issues:

- `resolve_name` in `ast_update.rs` prepending the file namespace to
  an already-qualified string literal from `$casts`
- Short names failing to match FQNs in `depends_on_any` cache eviction
- Type comparison failures when one side has `\` and the other does not

### Proposed approach

Pick one canonical form and enforce it at the boundaries. Two options:

**Option A: Always store without leading `\`.** Normalize at every
ingestion point (parser extraction, `resolve_name`, class loader
return values). All internal comparisons become simple string equality.
Display code adds `\` back only when rendering for the user. This is
the lower-risk option because it matches what most of the codebase
already assumes.

**Option B: Introduce a `Fqn` newtype.** A `struct Fqn(String)` that
guarantees the inner string is always a fully-qualified name without
leading `\`. Construction goes through a `Fqn::new()` that normalizes.
Replace `String` in `ClassInfo::name`, `parent_class`, `interfaces`,
`used_traits`, `mixins`, method/property type hints, etc. This is the
safer long-term option because the type system prevents mixing forms,
but it touches nearly every struct and function signature in the
codebase.

A pragmatic middle ground: start with Option A (normalize at
boundaries, remove scattered `strip_prefix` calls), then later
introduce the `Fqn` newtype for the highest-value fields
(`ClassInfo::name`, `parent_class`, `interfaces`, `used_traits`) where
type confusion causes the worst bugs.

### Scope

- Audit every field in `ClassInfo`, `MethodInfo`, `PropertyInfo`,
  `ParameterInfo`, and `FunctionInfo` that holds a class/type name.
  Document which form each field uses today.
- Pick canonical form and normalize at ingestion (parser, AST update,
  class loader, docblock extraction).
- Remove defensive `strip_prefix('\\')` calls at comparison sites.
  Each removal is a potential behavioural change, so each batch needs
  targeted tests.
- Update `resolve_name`, `resolve_class_name`, `resolve_type_string`
  to always produce the canonical form.
- Update `signature_eq`, `depends_on_any`, and cache key construction
  to rely on canonical form instead of ad-hoc normalization.