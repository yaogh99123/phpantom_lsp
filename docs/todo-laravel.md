# PHPantom — Laravel Support: Remaining Work

> Last updated: 2025-07-19

This document tracks bugs, known gaps, and missing features in
PHPantom's Laravel Eloquent support. For the general architecture and
virtual member provider design, see `ARCHITECTURE.md`.

---

## Current state

The `LaravelModelProvider` in `src/virtual_members/laravel.rs` is the
highest-priority virtual member provider. It synthesizes virtual members
for classes that extend `Illuminate\Database\Eloquent\Model`:

1. **Relationship properties.** All 10 relationship types (`HasOne`,
   `HasMany`, `BelongsTo`, `BelongsToMany`, `MorphOne`, `MorphMany`,
   `MorphTo`, `MorphToMany`, `HasManyThrough`, `HasOneThrough`).
   Supports Larastan-style `@return HasMany<Post, $this>` annotations
   and body-inferred relationships: when no `@return` annotation is
   present, the method body is scanned for patterns like
   `$this->hasMany(Post::class)` to infer the relationship type
   automatically. Chaining through relationship properties resolves
   end-to-end. Collection-type relationship properties use the
   *related* model's custom collection (not the owning model's).

2. **Scope methods.** `scopeActive(Builder $query)` produces `active()`
   as both static and instance virtual methods. The `$query` parameter is
   stripped, extra parameters are preserved, return types default to
   `Builder<static>` when absent or `void`.

3. **Builder-as-static forwarding.** `User::where('active', true)->
   orderBy('name')->get()` resolves the full chain. Template parameters
   (`TModel`) are substituted to the concrete model class.
   `Query\Builder` methods are included via `@mixin`. `BuildsQueries`
   trait methods (`first()`, `firstOrFail()`, `sole()`) work through
   `@use` generics.

4. **Custom Eloquent collections.** `#[CollectedBy]` and
   `@use HasCollection<X>` are detected. Custom collection methods
   appear after `->get()`, after static `Model::get()`, and on
   collection-type relationship properties. The attribute takes priority
   over the trait.

5. **Go-to-definition.** Jumps to `Builder::where()`,
   `Query\Builder::orderBy()`, `BuildsQueries::first()`, and scope
   methods all work through `find_builder_forwarded_method`.

6. **Accessors and mutators.** Legacy accessors (`getFullNameAttribute()`)
   and modern Laravel 9+ accessors (methods returning
   `Illuminate\Database\Eloquent\Casts\Attribute`) produce virtual
   properties. The property name is derived by converting the method
   name portion to snake_case (`getFullNameAttribute` → `full_name`,
   `avatarUrl()` → `avatar_url`). Legacy accessors use the method's
   return type; modern accessors use `mixed`.

Test coverage: 154 unit tests in `laravel.rs`, 75 integration tests in
`completion_laravel.rs`, 15 integration tests in `definition_laravel.rs`.

---

## Known gaps (documented in tests)

### 1. Variable assignment from builder-forwarded static method in GTD

`$q = User::where(...)` then `$q->orderBy()` does not fully resolve for
go-to-definition because the variable resolution path
(`resolve_rhs_static_call`) finds `where()` on the raw `Task` class via
`resolve_method_return_types_with_args`, which calls
`resolve_class_fully` internally. The issue is that the returned Builder
type's methods are resolved, but go-to-definition then cannot trace back
to the declaring class in a Builder loaded through the chain. This
works for completion (which only needs the type) but not for GTD (which
needs the source location).

---

## Missing features

### 2. Eloquent casts

Properties defined in the `$casts` array (or `casts()` method) should
produce typed virtual properties. For example:

```php
protected $casts = [
    'created_at' => 'datetime',    // → Carbon
    'options' => 'array',          // → array
    'is_admin' => 'boolean',       // → bool
];
```

This requires parsing the `$casts` property initializer or `casts()`
method body to extract key-value pairs, then mapping cast type strings
to PHP types. Common mappings: `datetime` → `Carbon\Carbon`,
`array`/`json` → `array`, `boolean`/`bool` → `bool`,
`integer`/`int` → `int`, `float`/`double`/`real`/`decimal:*` → `float`,
`string` → `string`, `collection` →
`Illuminate\Support\Collection`, custom cast classes → inspect
their `get()` return type.

### 3. `newCollection()` override detection

Laravel supports overriding `newCollection()` on a model to return a
custom collection class. Currently only `#[CollectedBy]` and
`@use HasCollection<X>` are detected.

**Implementation sketch:** In `extract_custom_collection`, additionally
check if the class has a method named `newCollection` and inspect its
return type annotation for the custom collection class name.

### 4. Factory support

`User::factory()->create()` is ubiquitous in Laravel test code. The
`factory()` static method returns a `HasFactory` trait method that
produces a factory instance. Resolving the chain requires:

1. Detecting the `HasFactory` trait on the model.
2. Resolving `factory()` to the model's corresponding Factory class
   (convention: `App\Models\User` → `Database\Factories\UserFactory`).
3. Resolving `create()` / `make()` on the factory to return the model.

This is medium complexity because it involves a naming convention
(model name → factory name) and cross-file resolution.

### 5. Closure parameter inference in collection pipelines

`$users->map(fn($u) => $u->...)` does not infer `$u` as the
collection's element type. This is a general generics/callable
inference problem, not Laravel-specific, but Laravel collection
pipelines are the most common place users encounter it.

### 6. Query scope chaining on Builder instances

Inside a scope method body, `$query->verified()` (calling another
scope) does not offer scope method completions. Scope methods are
synthesized on the Model class, not on the Builder class. The Builder
instance inside a scope body resolves to `Illuminate\Database\Eloquent\Builder`
which has no knowledge of the model's scopes.

**Possible fix:** When the Builder's `TModel` template parameter is
known (e.g., `Builder<User>`), load the concrete model and merge its
scope methods as instance methods on the resolved Builder. This
requires extending the virtual member system to also apply to
Builder instances, not just Model classes.

---

## Out of scope (and why)

| Item | Reason |
|------|--------|
| Container string aliases | Requires booting the application. Use `::class` references instead. |
| Facade `getFacadeAccessor()` with string aliases | Same problem. `@method` tags provide a workable fallback. |
| Blade templates | Large scope, separate project. |
| Model column types from DB/migrations | Unreasonable complexity. Require `@property` annotations (via ide-helper or hand-written). |
| Legacy Laravel versions | We target current Larastan-style annotations. Older code may degrade gracefully. |
| Application provider scanning | Low-value, high-complexity. |

---

## Philosophy (unchanged)

- **No application booting.** We never boot a Laravel application to
  resolve types.
- **No SQL/migration parsing.** Model column types are not inferred from
  database schemas or migration files.
- **Larastan-style hints preferred.** We expect relationship methods to be
  annotated in the style that Larastan expects. Fallback heuristics (item 3
  above) are best-effort.
- **Facades fall back to `@method`.** Facades whose `getFacadeAccessor()`
  returns a string alias cannot be resolved. `@method` tags on facade
  classes provide completion without template intelligence.