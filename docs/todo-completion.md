# PHPantom — Completion Improvements

This document covers completion-specific improvements: dynamic return
type handling for built-in functions, stub attribute extraction, and
argument-level intelligence. Items that are about *type resolution
infrastructure* (generics, narrowing, conditional types) live in
[todo-type-inference.md](todo-type-inference.md).

Items are ordered by **impact** (descending), then **effort** (ascending)
within the same impact tier.

| Label | Scale |
|---|---|
| **Impact** | **Critical**, **High**, **Medium-High**, **Medium**, **Low-Medium**, **Low** |
| **Effort** | **Low** (≤ 1 day), **Medium** (2-5 days), **Medium-High** (1-2 weeks), **High** (2-4 weeks), **Very High** (> 1 month) |

---

## 1. `BackedEnum::from()` / `::tryFrom()` return type refinement
**Impact: Medium · Effort: Low**

When calling `MyEnum::from($value)` or `MyEnum::tryFrom($value)`,
PHPStan resolves the return type to `MyEnum` (or `MyEnum|null` for
`tryFrom`) rather than the generic `BackedEnum` base type.  This is a
static method return type that depends on the calling class — the
same pattern as `static` return types on static methods.

We already handle `new static()` and `static` return types for
instance methods, but static method calls on enums may not flow
through the same path.  Verify and fix if needed.

See `BackedEnumFromMethodDynamicReturnTypeExtension` in PHPStan.

---

## 2. Array functions needing new code paths
**Impact: Medium · Effort: High**

These functions have return type semantics that don't fit into either
`ARRAY_PRESERVING_FUNCS` (same array type out) or `ARRAY_ELEMENT_FUNCS`
(single element out).  Each needs its own mini-resolver.

| Function | Return type logic | PHPStan extension |
|---|---|---|
| `array_keys` | Returns `list<TKey>` — extracts the *key* type, not value type | `ArrayKeysFunctionDynamicReturnTypeExtension` |
| `array_column` | Extracts a column from a 2D array, preserving types | `ArrayColumnFunctionReturnTypeExtension` |
| `array_combine` | Keys from first array arg, values from second | `ArrayCombineFunctionReturnTypeExtension` |
| `array_fill` | `array<int, TValue>` preserving the fill value type | `ArrayFillFunctionReturnTypeExtension` |
| `array_fill_keys` | Preserves key array type + value type | `ArrayFillKeysFunctionReturnTypeExtension` |
| `array_flip` | Swaps key↔value types | `ArrayFlipFunctionReturnTypeExtension` |
| `array_pad` | Union of existing value type + pad value type | `ArrayPadDynamicReturnTypeExtension` |
| `array_replace` | Merge-like, preserving types from all args | `ArrayReplaceFunctionReturnTypeExtension` |
| `array_change_key_case` | Preserves value type, transforms key type | `ArrayChangeKeyCaseFunctionReturnTypeExtension` |
| `array_intersect_key` | Preserves first array's types (dedicated extension) | `ArrayIntersectKeyFunctionReturnTypeExtension` |
| `array_reduce` | Returns the callback's return type (like `array_map`) | `ArrayReduceFunctionReturnTypeExtension` |
| `array_search` | Returns key type of the haystack array | `ArraySearchFunctionDynamicReturnTypeExtension` |
| `array_rand` | Returns key type of the input array | `ArrayRandFunctionReturnTypeExtension` |
| `array_sum` | Computes numeric return type from value types | `ArraySumFunctionDynamicReturnTypeExtension` |
| `array_count_values` | Returns `array<TValue, int>` | `ArrayCountValuesDynamicReturnTypeExtension` |
| `array_key_first` / `array_key_last` | Returns key type (usually scalar, low completion value) | `ArrayFirstLastDynamicReturnTypeExtension` |
| `array_find_key` | Returns key type (PHP 8.4) | `ArrayFindKeyFunctionReturnTypeExtension` |
| `iterator_to_array` | Preserves iterable key/value types into array | `IteratorToArrayFunctionReturnTypeExtension` |
| `compact` | Builds typed array from variable names | `CompactFunctionReturnTypeExtension` |
| `count` / `sizeof` | Returns precise int range based on array size | `CountFunctionReturnTypeExtension` |
| `min` / `max` | Returns union of argument types | `MinMaxFunctionReturnTypeExtension` |

---

## 3. `LanguageLevelTypeAware` version-aware type hints
**Impact: Medium · Effort: Medium**

phpstorm-stubs use a second version attribute, `#[LanguageLevelTypeAware]`,
to override **type hints** (not element availability) based on the PHP
version. Unlike `#[PhpStormStubsElementAvailable]` which controls whether
an entire function, method, or parameter exists, `LanguageLevelTypeAware`
changes the type of a parameter or return value while the element itself
stays present. There are ~2,000 occurrences across the stubs.

The attribute takes an associative array mapping version strings to type
hints, plus a `default` fallback:

```php
// Return type changes by version:
#[LanguageLevelTypeAware(["8.4" => "StreamBucket|null"], default: "object|null")]
function stream_bucket_make_writeable($brigade) {}

// Parameter type changes by version:
function array_key_exists(
    $key,
    #[LanguageLevelTypeAware(["8.0" => "array"], default: "array|ArrayObject")] $array
): bool {}
```

PHPantom currently ignores these attributes. The native type hint from the
AST is used as-is, which means on PHP 8.4 a function might show
`object|null` instead of `StreamBucket|null`, or a parameter might show
`array|ArrayObject` instead of `array`.

**Implementation:** During parameter and return-type extraction (when
`DocblockCtx.php_version` is set), scan the element's attributes for
`LanguageLevelTypeAware`. Find the highest version key that is ≤ the
target version. If found, use that type string as the native type hint;
otherwise use the `default` value. This should integrate into the same
extraction points that already handle `PhpStormStubsElementAvailable`.

**Note:** Two stub files alias the attribute name: `intl/intl.php` uses
`LanguageAware` (~249 usages) and `ldap/ldap.php` uses `PhpVersionAware`
(~101 usages). The attribute matcher must recognise all three names.

---

## 4. `#[ArrayShape]` return shapes on stub functions
**Impact: Medium · Effort: Medium**

phpstorm-stubs annotate ~84 functions and methods with
`#[ArrayShape(["key" => "type", ...])]` to declare the structure of
their array return values. Almost none of these have a companion
`@return array{...}` docblock, so the shape information is invisible
to PHPantom. This affects commonly used functions like `parse_url`,
`stat`, `pathinfo`, `gc_status`, `getimagesize`,
`session_get_cookie_params`, `stream_get_meta_data`, and
`password_get_info`.

```php
#[ArrayShape(["lifetime" => "int", "path" => "string", "domain" => "string",
              "secure" => "bool", "httponly" => "bool", "samesite" => "string"])]
function session_get_cookie_params(): array {}

#[ArrayShape(["runs" => "int", "collected" => "int", "threshold" => "int", "roots" => "int"])]
function gc_status(): array {}
```

**Implementation:** During function/method extraction, scan for the
`ArrayShape` attribute. Parse the associative array literal in its
argument to build an `array{key: type, ...}` string, and use it as
the effective return type (or parameter type when applied to a
parameter). This complements the existing docblock `array{...}`
parsing and should feed into the same `return_type` field on
`FunctionInfo` / `MethodInfo`.

---

## 5. `#[Deprecated]` structured deprecation metadata
**Impact: Low-Medium · Effort: Low**

phpstorm-stubs annotate ~362 functions, methods, classes, constants,
properties, and parameters with `#[Deprecated(reason: "...",
replacement: "...", since: "X.Y")]`. PHPantom already reads
`@deprecated` from docblocks, but many stub entries use the attribute
instead of (or in addition to) a docblock tag. The attribute carries
richer data than the free-text `@deprecated` tag:

- `since` — the PHP version when the element was deprecated. Combined
  with PHP version detection, this could suppress deprecation warnings
  when targeting an older version where the element was not yet
  deprecated, or show "deprecated since PHP 8.0" in hover.
- `reason` — a human-readable explanation.
- `replacement` — a code template for auto-replacement (e.g.
  `"exif_read_data(%parametersList%)"` for `read_exif_data`). Could
  power a future "replace deprecated call" code action.

```php
#[Deprecated(reason: "Use anonymous functions instead", since: "7.2")]
function create_function(string $args, string $code): false|string {}

#[Deprecated(replacement: "exif_read_data(%parametersList%)", since: "7.2")]
function read_exif_data($filename, $sections = null, $arrays = false, $thumbnail = false) {}
```

**Implementation:** During extraction, scan for the `Deprecated`
attribute. Store the `since`, `reason`, and `replacement` fields on
`FunctionInfo` / `MethodInfo` / `ClassInfo`. In hover, prefer the
structured message over the raw `@deprecated` text. Optionally, use
the `since` version to make deprecation warnings version-aware.

---

## 6. Go-to-definition for array shape keys via bracket access
**Impact: Low-Medium · Effort: Medium**

Array shape keys accessed via bracket notation (`$status['code']`)
have no go-to-definition support. The type comes from a
`@phpstan-type` / `@phpstan-import-type` alias or a direct
`@var` / `@return` annotation resolved to
`array{code: int, label: string}`, but Ctrl+Click on the string
key inside `['code']` does nothing.

Object shape properties (`$profile->name` from
`@return object{name: string}`) already jump to the property key
in the docblock. Extending the same approach to bracket-access
array shapes would require detecting the array key context in the
GTD path (similar to array shape completion) and searching for the
key inside the matching `array{…}` annotation.

---

## 7. Non-array functions with dynamic return types
**Impact: Low · Effort: High**

PHPStan also provides dynamic return type extensions for many non-array
functions.  These are lower priority because they mostly refine scalar
return types (less impactful for class-based completion).

| Function | Return type logic | PHPStan extension |
|---|---|---|
| `abs` | Preserves int/float return type | `AbsFunctionDynamicReturnTypeExtension` |
| `base64_decode` | `string\|false` based on strict param | `Base64DecodeDynamicFunctionReturnTypeExtension` |
| `explode` | `list<string>` / `non-empty-list<string>` / `false` | `ExplodeFunctionDynamicReturnTypeExtension` |
| `filter_var` | Return type depends on filter constant | `FilterVarDynamicReturnTypeExtension` |
| `filter_input` | Same as `filter_var` | `FilterInputDynamicReturnTypeExtension` |
| `filter_var_array` / `filter_input_array` | Typed array based on filter definitions | `FilterVarArrayDynamicReturnTypeExtension` |
| `get_class` | Returns `class-string<T>` | `GetClassDynamicReturnTypeExtension` |
| `get_called_class` | Returns `class-string<static>` | `GetCalledClassDynamicReturnTypeExtension` |
| `get_parent_class` | Returns parent class-string | `GetParentClassDynamicFunctionReturnTypeExtension` |
| `gettype` | Returns specific string literal for known types | `GettypeFunctionReturnTypeExtension` |
| `get_debug_type` | Returns specific string literal | `GetDebugTypeFunctionReturnTypeExtension` |
| `constant` | Resolves named constant to its type | `ConstantFunctionReturnTypeExtension` |
| `date` / `date_format` | Precise string return types | `DateFunctionReturnTypeExtension` |
| `date_create` / `date_create_immutable` | `DateTime\|false` | `DateTimeCreateDynamicReturnTypeExtension` |
| `hash` / `hash_file` / etc. | Precise return types | `HashFunctionsReturnTypeExtension` |
| `sprintf` / `vsprintf` | Non-empty-string preservation | `SprintfFunctionDynamicReturnTypeExtension` |
| `preg_split` | `list<string>\|false` based on flags | `PregSplitDynamicReturnTypeExtension` |
| `str_split` / `mb_str_split` | Non-empty-list | `StrSplitFunctionReturnTypeExtension` |
| `class_implements` / `class_uses` / `class_parents` | `array<string, string>\|false` | `ClassImplementsFunctionReturnTypeExtension` |

---

## 8. `#[ReturnTypeContract]` parameter-dependent return types
**Impact: Low · Effort: Low**

phpstorm-stubs use `#[ReturnTypeContract]` (aliased as `TypeContract`)
on 4 functions to express return type narrowing based on a parameter's
value or presence. These functions have no `@phpstan-return` conditional
type in their docblocks, so the narrowing information is only available
through the attribute.

The attribute has four named arguments:
- `true` / `false` — narrows the return type when the annotated boolean
  parameter is `true` or `false`.
- `exists` / `notExists` — narrows the return type when an optional
  variadic parameter is passed or omitted.

```php
// microtime(true) → float, microtime(false) → string
function microtime(
    #[TypeContract(true: "float", false: "string")] bool $as_float = false
): string|float {}

// sscanf with extra args → int|null, without → array|null
function sscanf(
    string $string, string $format,
    #[TypeContract(exists: "int|null", notExists: "array|null")] mixed &...$vars
): array|int|null {}
```

Affected functions: `microtime`, `gettimeofday`, `sscanf`, `fscanf`.

**Implementation:** When resolving a call to one of these functions,
check whether the annotated parameter was passed (for `exists`/
`notExists`) or matches a literal boolean (for `true`/`false`). Use the
narrowed type from the attribute instead of the declared union return
type. This integrates into the call return type resolution path.

---

## 9. `#[ExpectedValues]` parameter value suggestions
**Impact: Low · Effort: Medium**

phpstorm-stubs annotate ~62 parameters and return values (including
usages via the `EV` alias in `intl` and `ftp`) with
`#[ExpectedValues]` to declare the set of valid constant values or
flags. This could power smarter completions inside function call
arguments by suggesting the valid constants.

The attribute supports several forms:
- `values: [CONST_A, CONST_B]` — one of the listed values is expected.
- `flags: [FLAG_A, FLAG_B]` — a bitmask combination is expected.
- `valuesFromClass: MyClass::class` — one of the class's constants.
- `flagsFromClass: MyClass::class` — bitmask of the class's constants.

```php
function phpinfo(
    #[ExpectedValues(flags: [INFO_GENERAL, INFO_CREDITS, INFO_CONFIGURATION,
                             INFO_MODULES, INFO_ENVIRONMENT, INFO_VARIABLES,
                             INFO_LICENSE, INFO_ALL])]
    int $flags = INFO_ALL
): bool {}

function pathinfo(
    string $path,
    #[ExpectedValues(flags: [PATHINFO_DIRNAME, PATHINFO_BASENAME,
                             PATHINFO_EXTENSION, PATHINFO_FILENAME])]
    int $flags = PATHINFO_ALL
): string|array {}
```

**Implementation:** During parameter extraction, store the expected
values metadata. When providing completions inside a function call
argument position, check whether the target parameter has expected
values and offer the listed constants at the top of the suggestions
list. Flag-style parameters should also suggest bitwise-OR
combinations.