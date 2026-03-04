# PHPantom — Roadmap

This is the master index for all planned work. Each row links to the
domain document that contains the full specification. Items are
sequenced by **sprint priority** — what to build next to widen the type
intelligence lead, then close the LSP feature gap, maximising coverage
with each step.

| Label | Scale |
|---|---|
| **Impact** | **Critical**, **High**, **Medium-High**, **Medium**, **Low-Medium**, **Low** |
| **Effort** | **Low** (≤ 1 day), **Medium** (2-5 days), **Medium-High** (1-2 weeks), **High** (2-4 weeks), **Very High** (> 1 month) |

## Domain Documents

| Document | Scope |
|---|---|
| [Type Inference](todo-type-inference.md) | Generic resolution, conditional return types, type narrowing, stub attribute handling |
| [Completion](todo-completion.md) | Completion-specific improvements (enum return types, array shapes, expected values) |
| [Diagnostics](todo-diagnostics.md) | `@deprecated` warnings, resolution-failure diagnostics, unused `use` dimming, suppression intelligence |
| [Code Actions](todo-actions.md) | Implement missing methods, null coalescing simplification, extract function, switch→match |
| [LSP Features](todo-lsp-features.md) | Find references, document highlighting, document/workspace symbols, rename, code lens, inlay hints, PHPDoc generation, partial result streaming |
| [Hover](todo-hover.md) | Deprecation messages, constant values, member origin indicators, enum case listing, trait method summaries |
| [Signature Help](todo-signatureHelp.md) | Parameter descriptions, signature-level docs, default values, attribute/closure support |
| [Laravel](todo-laravel.md) | Model property gaps, relationship methods, type narrowing, custom builders |
| [Blade](todo-blade.md) | Preprocessor, component support, cross-file view intelligence |
| [Testing](todo-testing.md) | Fixture runner, Phpactor test mining, benchmarks |
| [Bug Fixes](todo-bugs.md) | Incorrect behaviour that should be fixed regardless of feature priority |

---

## Sprint 1 — Widen the intelligence lead & polish

Type intelligence depth is PHPantom's defining advantage. This sprint
deepens that lead with the two highest-value type inference gaps, wires
up the signature help data that already exists, and ships the two
cheapest diagnostics for maximum visual impact.

| # | Item | Effort | Domain | Doc Link |
|---|---|---|---|---|
| 1 | Function-level `@template` generic resolution | Medium | Type Inference | [todo-type-inference.md §2](todo-type-inference.md#2-function-level-template-generic-resolution) |
| 2 | Inherited docblock type propagation | Medium | Type Inference | [todo-type-inference.md §4](todo-type-inference.md#4-inherited-docblock-type-propagation) |
| 3 | Per-parameter `@param` descriptions in signature help | Trivial | Signature Help | [todo-signatureHelp.md §1](todo-signatureHelp.md#1-per-parameter-param-descriptions) |
| 4 | Signature-level documentation | Small | Signature Help | [todo-signatureHelp.md §2](todo-signatureHelp.md#2-signature-level-documentation-methodfunction-docblock) |
| 5 | Default values in signature help parameter labels | Trivial | Signature Help | [todo-signatureHelp.md §3](todo-signatureHelp.md#3-default-values-in-parameter-labels) |
| 6 | `@deprecated` usage diagnostics | Low | Diagnostics | [todo-diagnostics.md §1](todo-diagnostics.md#1-deprecated-usage-diagnostics) |
| 7 | Unused `use` dimming | Low | Diagnostics | [todo-diagnostics.md §4](todo-diagnostics.md#4-unused-use-dimming) |

**Why this order:** Items 1–2 are the highest-value type inference gaps.
Function-level `@template` unlocks `collect()` and every generic helper
function — huge for Laravel. Inherited docblock propagation fixes every
"why doesn't completion work on this override?" report. Items 3–5 are
pure wiring — the data already exists on `ParameterInfo`/`MethodInfo`/
`FunctionInfo`, it just needs plumbing to the LSP response. Half a day
of work for noticeably richer signature popups. Items 6–7 are the
cheapest diagnostics to ship and the most visually impactful —
strikethrough on deprecated calls and dimmed unused imports signal
"production-ready" to new users.

---

## Sprint 2 — Close the LSP feature gap

These items close the most commonly expected LSP feature surface gaps.
Each one removes a reason someone might look elsewhere. Ordered so that
low-effort items land first, then the bigger pieces that round out the
core editing experience.

| # | Item | Effort | Domain | Doc Link |
|---|---|---|---|---|
| 8 | Document Highlighting (`textDocument/documentHighlight`) | Low | LSP Features | [todo-lsp-features.md §2](todo-lsp-features.md#2-document-highlighting-textdocumentdocumenthighlight) |
| 9 | Document Symbols (`textDocument/documentSymbol`) | Low | LSP Features | [todo-lsp-features.md §4](todo-lsp-features.md#4-document-symbols-textdocumentdocumentsymbol) |
| 10 | Workspace Symbols (`workspace/symbol`) | Low-Medium | LSP Features | [todo-lsp-features.md §5](todo-lsp-features.md#5-workspace-symbols-workspacesymbol) |
| 11 | Find References (`textDocument/references`) | Medium-High | LSP Features | [todo-lsp-features.md §1](todo-lsp-features.md#1-find-references-textdocumentreferences) |
| 12 | Rename (`textDocument/rename`) | Medium-High | LSP Features | [todo-lsp-features.md §7](todo-lsp-features.md#7-rename-textdocumentrename) |
| 13 | Implement missing abstract/interface methods (code action) | Medium | Code Actions | [todo-actions.md §1](todo-actions.md#1-implement-missing-abstractinterface-methods) |
| 14 | Retrigger on `)` to dismiss signature help | Trivial | Signature Help | [todo-signatureHelp.md §6](todo-signatureHelp.md#6-retrigger-on--to-dismiss) |

**After Sprint 2:** PHPantom covers every commonly expected LSP feature
and surpasses the field on type intelligence, generics, Laravel, and
performance. No feature gaps remain for typical day-to-day editing.

---

## Sprint 3 — Complete feature set & project polish

This sprint adds the features that power users and full-IDE migrants
expect and would miss. Ordered so that each item provides maximum
coverage of what's expected from a mature PHP language server.

| # | Item | Effort | Domain | Doc Link |
|---|---|---|---|---|
| 15 | Deprecation message text in hover | Low | Hover | [todo-hover.md §1](todo-hover.md#1-deprecation-message-text) |
| 16 | Constant value display in hover | Low | Hover | [todo-hover.md §2](todo-hover.md#2-constant-value-display) |
| 17 | Member origin indicators in hover | Low-Medium | Hover | [todo-hover.md §3](todo-hover.md#3-member-origin-indicators) |
| 18 | `BackedEnum::from()` / `::tryFrom()` return type refinement | Low | Completion | [todo-completion.md §1](todo-completion.md#1-backedenumfrom--tryfrom-return-type-refinement) |
| 19 | Pipe operator (PHP 8.5) type resolution | Low | Type Inference | [todo-type-inference.md §1](todo-type-inference.md#1-pipe-operator-php-85) |
| 20 | Code Lens: jump to prototype method | Low | LSP Features | [todo-lsp-features.md §8](todo-lsp-features.md#8-code-lens-jump-to-prototype-method) |
| 21 | Implementation → interface method declaration (reverse jump) | Low | LSP Features | [todo-lsp-features.md §10](todo-lsp-features.md#10-reverse-jump-implementation--interface-method-declaration) |
| 22 | Conditional return types `($param is T ? A : B)` | Medium | Type Inference | [todo-type-inference.md §3](todo-type-inference.md#3-parse-and-resolve-param-is-t--a--b-return-types) |
| 23 | PHPDoc block generation on `/**` | Medium | LSP Features | [todo-lsp-features.md §3](todo-lsp-features.md#3-phpdoc-block-generation-on-) |
| 24 | Resolution-failure diagnostics | Medium | Diagnostics | [todo-diagnostics.md §2](todo-diagnostics.md#2-resolution-failure-diagnostics) |
| 25 | Warn when composer.json is missing or classmap not optimized | Medium | Diagnostics | [todo-diagnostics.md §5](todo-diagnostics.md#5-warn-when-composerjson-is-missing-or-classmap-is-not-optimized) |
| 26 | File system watching for vendor and project changes | Medium | Type Inference | [todo-type-inference.md §5](todo-type-inference.md#5-file-system-watching-for-vendor-and-project-changes) |
| 27 | Property hooks (PHP 8.4) | Medium | Type Inference | [todo-type-inference.md §6](todo-type-inference.md#6-property-hooks-php-84) |
| 28 | Simplify with null coalescing / null-safe operator (code action) | Medium | Code Actions | [todo-actions.md §2](todo-actions.md#2-simplify-with-null-coalescing--null-safe-operator) |
| 29 | Inlay hints (`textDocument/inlayHint`) | Medium | LSP Features | [todo-lsp-features.md §9](todo-lsp-features.md#9-inlay-hints-textdocumentinlayhint) |

**After Sprint 3:** PHPantom has a complete, polished LSP feature set.
Users moving to Zed/Neovim/Helix lose nothing on the intelligence side
and gain 1000× faster startup. The remaining gaps are Blade and
formatting (not our domain).

---

## Sprint 4 — Deep type accuracy & Laravel excellence

These items push type resolution accuracy beyond what any tool offers.
They're the long tail that makes PHPantom the definitive choice for
projects that care about types.

| # | Item | Effort | Domain | Doc Link |
|---|---|---|---|---|
| 30 | `collect()` and helper functions lose generic type info | High | Laravel | [todo-laravel.md §5](todo-laravel.md#5-collect-and-other-helper-functions-lose-generic-type-info) |
| 31 | Custom Eloquent builders (`HasBuilder` / `#[UseEloquentBuilder]`) | Medium | Laravel | [todo-laravel.md §3](todo-laravel.md#3-custom-eloquent-builders-hasbuilder--useeloquentbuilder) |
| 32 | `abort_if`/`abort_unless` type narrowing | Medium | Laravel | [todo-laravel.md §4](todo-laravel.md#4-abort_ifabort_unless-type-narrowing) |
| 33 | Narrow types of `&$var` parameters after function calls | Medium | Type Inference | [todo-type-inference.md §7](todo-type-inference.md#7-narrow-types-of-var-parameters-after-function-calls) |
| 34 | SPL iterator generic stubs | Medium | Type Inference | [todo-type-inference.md §8](todo-type-inference.md#8-spl-iterator-generic-stubs) |
| 35 | `LanguageLevelTypeAware` version-aware type hints | Medium | Completion | [todo-completion.md §3](todo-completion.md#3-languageleveltypeaware-version-aware-type-hints) |
| 36 | `#[ArrayShape]` return shapes on stub functions | Medium | Completion | [todo-completion.md §4](todo-completion.md#4-arrayshape-return-shapes-on-stub-functions) |
| 37 | `#[Deprecated]` structured deprecation metadata | Low | Completion | [todo-completion.md §5](todo-completion.md#5-deprecated-structured-deprecation-metadata) |
| 38 | Asymmetric visibility (PHP 8.4) | Low | Type Inference | [todo-type-inference.md §9](todo-type-inference.md#9-asymmetric-visibility-php-84) |
| 39 | Attribute constructor signature help | Medium | Signature Help | [todo-signatureHelp.md §4](todo-signatureHelp.md#4-attribute-constructor-signature-help) |
| 40 | Closure/arrow function parameter signature help | Medium | Signature Help | [todo-signatureHelp.md §5](todo-signatureHelp.md#5-closure--arrow-function-parameter-signature-help) |
| 41 | Diagnostic suppression intelligence | Medium | Diagnostics | [todo-diagnostics.md §3](todo-diagnostics.md#3-diagnostic-suppression-intelligence) |
| 42 | Partial result streaming via `$/progress` | Medium-High | LSP Features | [todo-lsp-features.md §6](todo-lsp-features.md#6-partial-result-streaming-via-progress) |

**Note:** Item 30 (`collect()` generics) is a direct payoff of Sprint 1
item 1 (function-level `@template`). Once the infrastructure exists, the
Laravel-specific manifestation is a small incremental step.

---

## Sprint 5 — Blade support

Blade is a multi-phase project tracked in [todo-blade.md](todo-blade.md).
Shipping Blade support makes PHPantom the first open-source PHP language
server with Blade intelligence.

| Phase | Scope | Key Items |
|---|---|---|
| Phase 1 | Blade-to-PHP preprocessor | Module skeleton, directive translation, source map, LSP wiring |
| Phase 2 | Component support | Template/component discovery, `<x-component>` parsing, `@props`/`@aware`, name completion |
| Phase 3 | Cross-file view intelligence | View name GTD, signature merging for `@extends`, component→template variable typing |
| Phase 4 | Blade directive completion | Directive name completion with snippet insertion |

---

## Backlog — Diminishing returns

These items improve accuracy in niche scenarios. They're worth doing
eventually but don't move the needle.

### Completion & type inference tail

| # | Item | Effort | Domain | Doc Link |
|---|---|---|---|---|
| 43 | Array functions needing new code paths | High | Completion | [todo-completion.md §2](todo-completion.md#2-array-functions-needing-new-code-paths) |
| 44 | Go-to-definition for array shape keys via bracket access | Medium | Completion | [todo-completion.md §6](todo-completion.md#6-go-to-definition-for-array-shape-keys-via-bracket-access) |
| 45 | No go-to-definition for built-in (stub) functions and constants | Medium | LSP Features | [todo-lsp-features.md §11](todo-lsp-features.md#11-no-go-to-definition-for-built-in-stub-functions-and-constants) |
| 46 | `str_contains` / `str_starts_with` / `str_ends_with` → non-empty-string narrowing | Low | Type Inference | [todo-type-inference.md §10](todo-type-inference.md#10-str_contains--str_starts_with--str_ends_with--non-empty-string-narrowing) |
| 47 | `count` / `sizeof` comparison → non-empty-array narrowing | Low | Type Inference | [todo-type-inference.md §11](todo-type-inference.md#11-count--sizeof-comparison--non-empty-array-narrowing) |
| 48 | Fiber type resolution | Low | Type Inference | [todo-type-inference.md §12](todo-type-inference.md#12-fiber-type-resolution) |
| 49 | Non-empty-string propagation through string functions | Low | Type Inference | [todo-type-inference.md §13](todo-type-inference.md#13-non-empty-string-propagation-through-string-functions) |
| 50 | `Closure::bind()` / `Closure::fromCallable()` return type preservation | Low-Medium | Type Inference | [todo-type-inference.md §14](todo-type-inference.md#14-closurebind--closurefromcallable-return-type-preservation) |
| 51 | Non-array functions with dynamic return types | High | Completion | [todo-completion.md §7](todo-completion.md#7-non-array-functions-with-dynamic-return-types) |
| 52 | `#[ReturnTypeContract]` parameter-dependent return types | Low | Completion | [todo-completion.md §8](todo-completion.md#8-returntypecontract-parameter-dependent-return-types) |
| 53 | `#[ExpectedValues]` parameter value suggestions | Medium | Completion | [todo-completion.md §9](todo-completion.md#9-expectedvalues-parameter-value-suggestions) |

### Hover & signature help polish

| # | Item | Effort | Domain | Doc Link |
|---|---|---|---|---|
| 54 | Enum case listing in enum hover | Low | Hover | [todo-hover.md §4](todo-hover.md#4-enum-case-listing-in-enum-hover) |
| 55 | Trait hover shows public method signatures | Low | Hover | [todo-hover.md §5](todo-hover.md#5-trait-hover-shows-public-method-signatures) |
| 56 | Multiple overloaded signatures | Medium-High | Signature Help | [todo-signatureHelp.md §7](todo-signatureHelp.md#7-multiple-overloaded-signatures) |
| 57 | Named argument awareness in active parameter | Medium | Signature Help | [todo-signatureHelp.md §8](todo-signatureHelp.md#8-named-argument-awareness-in-active-parameter) |
| 58 | Language construct signature help and hover | Low | Signature Help | [todo-signatureHelp.md §9](todo-signatureHelp.md#9-language-construct-signature-help-and-hover) |

### LSP features & code actions

| # | Item | Effort | Domain | Doc Link |
|---|---|---|---|---|
| 59 | Switch → match conversion | Medium | Code Actions | [todo-actions.md §4](todo-actions.md#4-switch--match-conversion) |
| 60 | Extract Function refactoring | Very High | Code Actions | [todo-actions.md §3](todo-actions.md#3-extract-function-refactoring) |

### Bug fixes

| # | Item | Effort | Domain | Doc Link |
|---|---|---|---|---|
| 61 | Short-name collisions in `find_implementors` | Low | Bug Fixes | [todo-bugs.md §1](todo-bugs.md#1-short-name-collisions-in-find_implementors) |

---

## Laravel-Specific Gaps

These are tracked in [todo-laravel.md](todo-laravel.md) and ranked
separately by their own impact÷effort scoring.

| # | Item | Impact | Effort | Doc Link |
|---|---|---|---|---|
| L1 | `morphedByMany` missing from relationship method map | ★★ | ★ | [§1](todo-laravel.md#1-morphedbymany-missing-from-relationship-method-map) |
| L2 | `$dates` array (deprecated) | ★★ | ★★ | [§2](todo-laravel.md#2-dates-array-deprecated) |
| L3 | Custom Eloquent builders | ★★★★ | ★★★ | [§3](todo-laravel.md#3-custom-eloquent-builders-hasbuilder--useeloquentbuilder) |
| L4 | `abort_if`/`abort_unless` type narrowing | ★★★★ | ★★★ | [§4](todo-laravel.md#4-abort_ifabort_unless-type-narrowing) |
| L5 | `collect()` generic type info | ★★★★★ | ★★★★ | [§5](todo-laravel.md#5-collect-and-other-helper-functions-lose-generic-type-info) |
| L6 | Factory `has*`/`for*` relationship methods | ★★ | ★★★ | [§6](todo-laravel.md#6-factory-hasfor-relationship-methods) |
| L7 | `$pivot` property on BelongsToMany | ★★★ | ★★★★ | [§7](todo-laravel.md#7-pivot-property-on-belongstomany-related-models) |
| L8 | `withSum`/`withAvg`/`withMin`/`withMax` aggregate properties | ★★ | ★★★★ | [§8](todo-laravel.md#8-withsum--withavg--withmin--withmax-aggregate-properties) |
| L9 | Higher-order collection proxies | ★★ | ★★★★ | [§9](todo-laravel.md#9-higher-order-collection-proxies) |
| L10 | `SoftDeletes` trait methods on Builder | ★ | ★ | [§10](todo-laravel.md#10-softdeletes-trait-methods-on-builder) |
| L11 | `View::withX()` / `RedirectResponse::withX()` dynamic methods | ★ | ★★ | [§11](todo-laravel.md#11-viewwithx-and-redirectresponsewithx-dynamic-methods) |
| L12 | `$appends` array | ★ | ★ | [§12](todo-laravel.md#12-appends-array) |
| L13 | Relationship classification matches short name only | ★ | ★★ | [§13](todo-laravel.md#13-relationship-classification-matches-short-name-only) |

---

## Blade Support

Blade is a multi-phase project tracked in [todo-blade.md](todo-blade.md).

| Phase | Scope | Key Items |
|---|---|---|
| Phase 1 | Blade-to-PHP preprocessor | Module skeleton, directive translation, source map, LSP wiring |
| Phase 2 | Component support | Template/component discovery, `<x-component>` parsing, `@props`/`@aware`, name completion |
| Phase 3 | Cross-file view intelligence | View name GTD, signature merging for `@extends`, component→template variable typing |
| Phase 4 | Blade directive completion | Directive name completion with snippet insertion |

---

## Testing

Testing infrastructure and Phpactor fixture mining are tracked in
[todo-testing.md](todo-testing.md).

| Phase | Scope |
|---|---|
| Phase 1 | Build a fixture runner |
| Phase 2 | Audit Phpactor's fixtures against our coverage |
| Phase 3 | Convert high-value fixtures |
| Phase 4 | Mine Phpactor's completion tests |
| Phase 5 | Smoke tests and benchmarks |