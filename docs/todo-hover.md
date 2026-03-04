# PHPantom — Hover: Improvement Plan

Hover is functional and covers all major symbol types (methods,
properties, constants, classes, variables, functions, `self`/`static`/
`parent`, template parameters, first-class callables). The remaining
work is presentation-layer enrichment — surfacing data we already have
(or can cheaply derive) to make hover output more informative.

Items are ordered by impact (descending), then effort (ascending).

---

## 1. Deprecation message text
**Impact: High · Effort: Low**

Today `is_deprecated` is a bare `bool` on `MethodInfo`, `PropertyInfo`,
`ConstantInfo`, `FunctionInfo`, and `ClassInfo`. Hover renders a generic
`**@deprecated**` with no explanation. When a library writes
`@deprecated Use collect() instead`, the user can't see what to use.

**Change:**

- Replace `is_deprecated: bool` with `deprecation_message: Option<String>`
  on all five structs. `None` = not deprecated; `Some("")` = deprecated
  without message; `Some("Use collect() instead")` = deprecated with
  message.
- In `has_deprecated_tag` (docblock/tags.rs), extract the text after
  `@deprecated` and return it.
- Update `virtual_method` / `virtual_property` constructors to use
  `deprecation_message: None`.
- In hover rendering, display:
  - `**@deprecated** Use collect() instead` when a message is present.
  - `**@deprecated**` when the message is empty.
- Update all `is_deprecated` checks across the codebase (completion
  strikethrough, diagnostics future use, etc.) to check
  `deprecation_message.is_some()`.

**Structs affected:** `MethodInfo`, `PropertyInfo`, `ConstantInfo`,
`FunctionInfo`, `ClassInfo`.

---

## 2. Constant value display
**Impact: High · Effort: Low**

Class constants don't show their value in hover. `const STATUS_ACTIVE;`
is far less useful than `const STATUS_ACTIVE = 'active';`. Enum cases
already show their backed value — regular constants should too.

**Change:**

- Add `value: Option<String>` to `ConstantInfo` (the `enum_value` field
  already exists for enum cases — generalise it or add a parallel field
  for regular constants).
- During parsing (`extract_class_like_members`, the
  `ClassLikeMember::ClassConstant` arm), extract the initialiser
  expression source text the same way `enum_value` is extracted for
  backed enum cases.
- In `hover_for_constant`, render the value:
  `const STATUS_ACTIVE = 'active';` for regular constants,
  `case Pending = 'pending';` for backed enum cases (already works).

---

## 3. Member origin indicators
**Impact: Medium · Effort: Low-Medium**

When hovering a method or property it's useful to know at a glance
whether it overrides a parent, implements an interface, or is a virtual
member synthesized from `@method`, `@property`, `@mixin`, or a framework
provider. Today all members look identical.

**Change:**

- After resolving the method's owning class in `hover_for_method`,
  check whether:
  - The parent class (if any) has a method with the same name → override.
  - Any implemented interface has a method with the same name → implements.
  - The member is virtual (`is_virtual` — see below).
- Render a subtle line above the code block (not inside the PHP block):
  - `↑ overrides **ParentClass**` when overriding a parent method.
  - `◆ implements **InterfaceName**` when implementing an interface method.
  - `👻 virtual` for synthesized members.
  - Lines combine when multiple apply (e.g. override + implements).
- Use the class loader to check the parent and interfaces. If the loader
  fails (class not found), omit the indicator silently.
- Apply the same logic in `hover_for_property` and `hover_for_constant`
  where applicable (properties can be virtual, constants can come from
  interfaces).

**Add `is_virtual: bool` to `MethodInfo`, `PropertyInfo`, and
`ConstantInfo`.**  Today virtual members are detected heuristically
(`name_offset == 0` combined with `native_return_type.is_none()` or
`native_type_hint.is_none()`). An explicit field is clearer and lets
us clean up related logic:

- `virtual_method` / `virtual_property` constructors set
  `is_virtual: true`; all parsing paths set `is_virtual: false`.
- `resolve_member_definition_with` and `resolve_function_definition`
  currently bail out when `name_offset == 0` as a proxy for "no
  source location". Replace with `if member.is_virtual { return None; }`
  which expresses intent directly.
- `find_member_position_in_class` can use `is_virtual` to skip the
  AST-offset fast path and go straight to text/docblock search for
  virtual members, instead of relying on `name_offset` being zero.

**Note:** The class loader is already available in the caller
(`hover_from_symbol`). The cost is one or two additional class lookups
per hover — cheap given the resolved-class cache.

---

## 4. Enum case listing in enum hover
**Impact: Low-Medium · Effort: Low**

When hovering an enum name, show the cases. Unlike a full class member
listing (which can be enormous), enum cases are a bounded, relevant set
— they *are* the enum's API.

**Change:**

- In `hover_for_class_info`, when `cls.kind == ClassLikeKind::Enum`,
  collect all `ConstantInfo` entries where `is_enum_case == true`.
- Render them inside the PHP code block after the `enum` signature:

  ```text
  ```php
  <?php
  namespace App\Enums;
  enum Status: string {
      case Pending = 'pending';
      case Active = 'active';
      case Cancelled = 'cancelled';
  }
  ```
  ```

- Cap at a reasonable limit (e.g. 30 cases) with `// and N more…` if
  exceeded.
- Regular class constants on an enum are *not* shown — only cases.

---

## 5. Trait hover shows public method signatures
**Impact: Low-Medium · Effort: Low**

Unlike a class (where the member list can be massive), a trait's public
methods are exactly what the user cares about — they define what `use
TraitName;` adds to the class. Showing them in hover is genuinely useful.

**Change:**

- In `hover_for_class_info`, when `cls.kind == ClassLikeKind::Trait`,
  collect public methods and render minimal signatures inside the PHP
  code block. Use **native types only** and **short (unqualified) class
  names** — the goal is a scannable summary, not a type reference:

  ```text
  ```php
  <?php
  namespace App\Concerns;
  trait HasSlug {
      public function getSlug(): string;
      public static function findBySlug(string $slug): static;
  }
  ```
  ```

- One line per method, no bodies, no docblock types, no FQNs.
  `function save(array $options): bool;` not
  `function save(array<string, mixed> $options): \App\Models\Model;`.
- Cap at 30 methods with `// and N more…` if exceeded.
- Include public properties and constants if any, same minimal style.

---