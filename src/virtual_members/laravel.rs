//! Laravel Eloquent Model virtual member provider.
//!
//! Synthesizes virtual members for classes that extend
//! `Illuminate\Database\Eloquent\Model`.  This is the highest-priority
//! virtual member provider: its contributions beat `@method` /
//! `@property` / `@mixin` members (PHPDocProvider).
//!
//! Currently implements:
//!
//! - **Relationship properties.** Methods returning a known Eloquent
//!   relationship type (e.g. `HasOne`, `HasMany`, `BelongsTo`) produce
//!   a virtual property with the same name.  The property type is
//!   inferred from the relationship's generic parameters (Larastan-style
//!   `@return HasMany<Post, $this>` annotations) or, as a fallback,
//!   from the first `::class` argument in the method body text.
//!
//! - **Scope methods.** Methods whose name starts with `scope` (e.g.
//!   `scopeActive`, `scopeVerified`) produce a virtual method with the
//!   `scope` prefix stripped and the first letter lowercased (e.g.
//!   `active`, `verified`).  The first `$query` parameter is removed.
//!   Scope methods are available as both static and instance methods
//!   so they resolve for `User::active()` and `$user->active()`.
//!
//! - **Builder-as-static forwarding.** Laravel's `Model::__callStatic()`
//!   forwards static calls to `static::query()`, which returns an
//!   Eloquent Builder.  This provider loads
//!   `\Illuminate\Database\Eloquent\Builder`, fully resolves it
//!   (including its `@mixin` on `Query\Builder`), and presents its
//!   public instance methods as static virtual methods on the model.
//!   Return types are mapped so that `static`/`$this`/`self` resolve
//!   to `Builder<ConcreteModel>` (the chain continues on the builder)
//!   and template parameters like `TModel` resolve to the concrete
//!   model class.  This makes `User::where(...)->orderBy(...)->get()`
//!   resolve end-to-end.

use std::collections::HashMap;

use crate::Backend;
use crate::docblock::types::parse_generic_args;
use crate::inheritance::{apply_substitution, apply_substitution_to_conditional};
use crate::types::ELOQUENT_COLLECTION_FQN;
use crate::types::{ClassInfo, MAX_INHERITANCE_DEPTH, MethodInfo, PropertyInfo, Visibility};

use super::{VirtualMemberProvider, VirtualMembers};

/// The fully-qualified name of the Eloquent base model.
const ELOQUENT_MODEL_FQN: &str = "Illuminate\\Database\\Eloquent\\Model";

/// Maps Eloquent relationship builder method names to their corresponding
/// relationship class short names.  Used by [`infer_relationship_from_body`]
/// to synthesize a return type from the method body when no `@return`
/// annotation is present.
const RELATIONSHIP_METHOD_MAP: &[(&str, &str)] = &[
    ("hasOne", "HasOne"),
    ("hasMany", "HasMany"),
    ("belongsTo", "BelongsTo"),
    ("belongsToMany", "BelongsToMany"),
    ("morphOne", "MorphOne"),
    ("morphMany", "MorphMany"),
    ("morphTo", "MorphTo"),
    ("morphToMany", "MorphToMany"),
    ("hasManyThrough", "HasManyThrough"),
    ("hasOneThrough", "HasOneThrough"),
];

/// Known Eloquent relationship class short names that yield a single
/// (nullable) related model instance when accessed as a property.
const SINGULAR_RELATIONSHIPS: &[&str] = &["HasOne", "MorphOne", "BelongsTo", "HasOneThrough"];

/// Known Eloquent relationship class short names that yield a
/// `Collection<TRelated>` when accessed as a property.
const COLLECTION_RELATIONSHIPS: &[&str] = &[
    "HasMany",
    "MorphMany",
    "BelongsToMany",
    "HasManyThrough",
    "MorphToMany",
];

/// The `MorphTo` relationship resolves to the generic `Model` base class
/// because the concrete related type is determined at runtime.
const MORPH_TO: &str = "MorphTo";

/// The fully-qualified name of the `Attribute` cast class used by
/// Laravel 9+ modern accessors/mutators.
const ATTRIBUTE_CAST_FQN: &str = "Illuminate\\Database\\Eloquent\\Casts\\Attribute";

/// The default return type for scope methods that don't declare a return
/// type or return `void`.
const DEFAULT_SCOPE_RETURN_TYPE: &str = "\\Illuminate\\Database\\Eloquent\\Builder<static>";

/// The fully-qualified name of the Eloquent Builder class.
pub const ELOQUENT_BUILDER_FQN: &str = "Illuminate\\Database\\Eloquent\\Builder";

/// Virtual member provider for Laravel Eloquent models.
///
/// When a class extends `Illuminate\Database\Eloquent\Model` (directly
/// or through an intermediate parent), this provider scans its methods
/// for Eloquent relationship return types and synthesizes virtual
/// properties for each one.
///
/// For example, a method `posts()` returning `HasMany<Post, $this>`
/// produces a virtual property `$posts` with type
/// `\Illuminate\Database\Eloquent\Collection<Post>`.
pub struct LaravelModelProvider;

/// Determine whether `class_name` is the Eloquent Model base class.
///
/// Checks against the FQN with and without a leading backslash, and
/// also against the short name `Model` (which may appear in stubs or
/// in same-file test setups).
fn is_eloquent_model(class_name: &str) -> bool {
    let stripped = class_name.strip_prefix('\\').unwrap_or(class_name);
    stripped == ELOQUENT_MODEL_FQN
}

/// Walk the parent chain of `class` looking for
/// `Illuminate\Database\Eloquent\Model`.
///
/// Returns `true` if the class itself is `Model` or any ancestor is.
pub fn extends_eloquent_model(
    class: &ClassInfo,
    class_loader: &dyn Fn(&str) -> Option<ClassInfo>,
) -> bool {
    if is_eloquent_model(&class.name) {
        return true;
    }

    let mut current = class.clone();
    let mut depth = 0u32;
    while let Some(ref parent_name) = current.parent_class {
        depth += 1;
        if depth > MAX_INHERITANCE_DEPTH {
            break;
        }
        if is_eloquent_model(parent_name) {
            return true;
        }
        match class_loader(parent_name) {
            Some(parent) => {
                if is_eloquent_model(&parent.name) {
                    return true;
                }
                current = parent;
            }
            None => break,
        }
    }

    false
}

/// The category of a relationship return type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RelationshipKind {
    /// HasOne, MorphOne, BelongsTo — singular nullable model.
    Singular,
    /// HasMany, MorphMany, BelongsToMany, HasManyThrough, MorphToMany — Collection.
    Collection,
    /// MorphTo — generic Model.
    MorphTo,
}

/// Try to classify a return type string as a known Eloquent relationship.
///
/// Accepts both short names (`HasMany`) and fully-qualified names
/// (`\Illuminate\Database\Eloquent\Relations\HasMany`).  Generic
/// parameters are stripped before matching.
fn classify_relationship(return_type: &str) -> Option<RelationshipKind> {
    let (base, _) = parse_generic_args(return_type);
    let short_name = extract_short_name(base);

    if SINGULAR_RELATIONSHIPS.contains(&short_name) {
        return Some(RelationshipKind::Singular);
    }
    if COLLECTION_RELATIONSHIPS.contains(&short_name) {
        return Some(RelationshipKind::Collection);
    }
    if short_name == MORPH_TO {
        return Some(RelationshipKind::MorphTo);
    }

    None
}

/// Extract the short (unqualified) class name from a potentially
/// fully-qualified name.
///
/// `"\\Illuminate\\Database\\Eloquent\\Relations\\HasMany"` → `"HasMany"`
/// `"HasMany"` → `"HasMany"`
fn extract_short_name(fqn: &str) -> &str {
    fqn.rsplit('\\').next().unwrap_or(fqn)
}

/// Extract the `TRelated` type from a relationship return type's
/// generic parameters.
///
/// Given `"HasMany<Post, $this>"`, returns `Some("Post")`.
/// Given `"HasOne<\\App\\Models\\Post, $this>"`, returns
/// `Some("\\App\\Models\\Post")`.
///
/// Returns `None` if no generic parameters are present.
fn extract_related_type(return_type: &str) -> Option<String> {
    let (_, args) = parse_generic_args(return_type);
    let first = args.first()?;
    let trimmed = first.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_string())
}

/// Build the virtual property type for a relationship.
/// Build the property type string for a relationship.
///
/// - Singular relationships → the related type as-is (nullable).
/// - Collection relationships → the custom collection class (if set) or
///   `\Illuminate\Database\Eloquent\Collection`, parameterised with `<TRelated>`.
/// - MorphTo → `\Illuminate\Database\Eloquent\Model`.
fn build_property_type(
    kind: RelationshipKind,
    related_type: Option<&str>,
    custom_collection: Option<&str>,
) -> Option<String> {
    match kind {
        RelationshipKind::Singular => related_type.map(|t| t.to_string()),
        RelationshipKind::Collection => {
            let inner = related_type.unwrap_or("\\Illuminate\\Database\\Eloquent\\Model");
            let collection_class = custom_collection
                .map(|c| format!("\\{}", c.strip_prefix('\\').unwrap_or(c)))
                .unwrap_or_else(|| format!("\\{ELOQUENT_COLLECTION_FQN}"));
            Some(format!("{collection_class}<{inner}>"))
        }
        RelationshipKind::MorphTo => Some("\\Illuminate\\Database\\Eloquent\\Model".to_string()),
    }
}

/// Determine whether a method is a legacy Eloquent accessor.
///
/// Legacy accessors follow the `getXAttribute()` naming convention where
/// `X` starts with an uppercase letter.  For example,
/// `getFullNameAttribute()` is a legacy accessor that produces a virtual
/// property `$full_name`.
fn is_legacy_accessor(method: &MethodInfo) -> bool {
    let name = &method.name;
    if !name.starts_with("get") || !name.ends_with("Attribute") {
        return false;
    }
    // Must have at least one character between "get" and "Attribute".
    // "getAttribute" itself (len 12) is a real Eloquent method, not an accessor.
    let middle = &name[3..name.len() - 9]; // strip "get" (3) and "Attribute" (9)
    if middle.is_empty() {
        return false;
    }
    // The first character of the middle portion must be uppercase.
    middle.starts_with(|c: char| c.is_uppercase())
}

/// Extract the virtual property name from a legacy accessor method name.
///
/// Strips `get` prefix and `Attribute` suffix, then converts the
/// remaining CamelCase portion to snake_case.
///
/// `getFullNameAttribute` → `full_name`
/// `getNameAttribute` → `name`
fn legacy_accessor_property_name(method_name: &str) -> String {
    let middle = &method_name[3..method_name.len() - 9];
    camel_to_snake(middle)
}

/// Determine whether a method is a modern Eloquent accessor (Laravel 9+).
///
/// Modern accessors are methods that return
/// `Illuminate\Database\Eloquent\Casts\Attribute`.  The method name
/// is in camelCase and the virtual property name is the snake_case
/// equivalent.  For example, `fullName(): Attribute` produces
/// `$full_name`.
fn is_modern_accessor(method: &MethodInfo) -> bool {
    match method.return_type.as_deref() {
        Some(rt) => {
            let clean = rt.strip_prefix('\\').unwrap_or(rt);
            // Strip generic parameters (e.g. Attribute<string, never>)
            let base = clean.split('<').next().unwrap_or(clean).trim();
            base == ATTRIBUTE_CAST_FQN || base == "Attribute"
        }
        None => false,
    }
}

/// Convert a camelCase or PascalCase string to snake_case.
///
/// Inserts an underscore before each uppercase letter that follows a
/// lowercase letter or digit, and before an uppercase letter that is
/// followed by a lowercase letter when preceded by another uppercase
/// letter (to handle acronyms like `URL` → `u_r_l`).
///
/// `FullName` → `full_name`
/// `firstName` → `first_name`
/// `isAdmin` → `is_admin`
fn camel_to_snake(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 4);
    let chars: Vec<char> = s.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if c.is_uppercase() {
            if i > 0 {
                let prev = chars[i - 1];
                // Insert underscore when: lowercase/digit → uppercase,
                // or uppercase → uppercase followed by lowercase (acronym boundary).
                if prev.is_lowercase() || prev.is_ascii_digit() {
                    result.push('_');
                } else if prev.is_uppercase() {
                    // Check next char for acronym boundary: "URL" + "Name" → "u_r_l_name"
                    if let Some(&next) = chars.get(i + 1)
                        && next.is_lowercase()
                    {
                        result.push('_');
                    }
                }
            }
            for lc in c.to_lowercase() {
                result.push(lc);
            }
        } else {
            result.push(c);
        }
    }
    result
}

/// Infer a relationship return type from a method's body text.
///
/// When a relationship method has no `@return` annotation, this function
/// scans the body for patterns like `$this->hasMany(Post::class)` and
/// synthesizes a return type string (e.g. `HasMany<Post>`).
///
/// Supports all standard Eloquent relationship builder methods:
/// `hasOne`, `hasMany`, `belongsTo`, `belongsToMany`, `morphOne`,
/// `morphMany`, `morphTo`, `morphToMany`, `hasManyThrough`, and
/// `hasOneThrough`.
///
/// Returns `None` if no recognisable pattern is found.
pub fn infer_relationship_from_body(body_text: &str) -> Option<String> {
    for &(method_name, class_name) in RELATIONSHIP_METHOD_MAP {
        // Look for `$this->methodName(` in the body text.
        let needle = format!("$this->{method_name}(");
        let Some(call_pos) = body_text.find(&needle) else {
            continue;
        };

        // `morphTo` never carries a related-model generic parameter;
        // the concrete type is determined at runtime.
        if method_name == "morphTo" {
            return Some(class_name.to_string());
        }

        // Extract the first argument from the call.  We look for
        // `SomeName::class` as the first positional argument.
        let args_start = call_pos + needle.len();
        let after_paren = &body_text[args_start..];

        if let Some(class_arg) = extract_class_argument(after_paren) {
            return Some(format!("{class_name}<{class_arg}>"));
        }

        // No `::class` argument found — return the bare relationship
        // name without generics.  The provider will handle it the same
        // way it handles annotated relationships without generics.
        return Some(class_name.to_string());
    }

    None
}

/// Extract a class name from the first `X::class` argument in a
/// parenthesised argument list.
///
/// Given the text after the opening `(`, e.g. `Post::class, 'user_id')`,
/// returns `Some("Post")`.  Also handles fully-qualified names like
/// `\App\Models\Post::class` and `self::class` / `static::class`.
///
/// Returns `None` if no `::class` token is found before the closing `)`.
fn extract_class_argument(after_paren: &str) -> Option<String> {
    // Find the closing paren to bound our search.
    let end = after_paren.find(')')?;
    let args_region = &after_paren[..end];

    // Isolate the first argument (before the first comma) and look for
    // `X::class` within it.
    let first_arg = args_region.split(',').next().unwrap_or(args_region);
    let class_pos = first_arg.find("::class")?;
    let before = first_arg[..class_pos].trim();

    if before.is_empty() {
        return None;
    }

    // Strip leading backslash for FQNs and extract the short name.
    let name = before.strip_prefix('\\').unwrap_or(before);
    let short_name = name.rsplit('\\').next().unwrap_or(name);

    if short_name.is_empty() {
        return None;
    }

    Some(short_name.to_string())
}

/// Determine whether a method is an Eloquent scope.
///
/// Scopes are methods whose name starts with `scope` (case-sensitive)
/// and have at least five characters (the prefix plus at least one
/// character for the scope name).  For example, `scopeActive` is a
/// scope, but `scope` alone is not.
fn is_scope_method(method: &MethodInfo) -> bool {
    method.name.starts_with("scope") && method.name.len() > 5
}

/// Transform a scope method name into the public-facing scope name.
///
/// Strips the `scope` prefix and lowercases the first character:
/// `scopeActive` → `active`, `scopeVerified` → `verified`.
fn scope_name(method_name: &str) -> String {
    let after_prefix = &method_name[5..]; // skip "scope"
    let mut chars = after_prefix.chars();
    match chars.next() {
        Some(c) => {
            let lower: String = c.to_lowercase().collect();
            format!("{lower}{}", chars.as_str())
        }
        None => String::new(),
    }
}

/// Determine the return type for a synthesized scope method.
///
/// Uses the scope method's declared return type.  If the return type is
/// `void` or absent, defaults to
/// `\Illuminate\Database\Eloquent\Builder<static>`.
fn scope_return_type(method: &MethodInfo) -> String {
    match method.return_type.as_deref() {
        Some("void") | None => DEFAULT_SCOPE_RETURN_TYPE.to_string(),
        Some(rt) => rt.to_string(),
    }
}

/// Build virtual methods for a scope method.
///
/// Returns two `MethodInfo` values: one static and one instance.  Both
/// have the `scope` prefix stripped, the first letter lowercased, and
/// the first `$query` parameter removed.  This makes scope methods
/// accessible via both `User::active()` (static) and `$user->active()`
/// (instance).
fn build_scope_methods(method: &MethodInfo) -> [MethodInfo; 2] {
    let name = scope_name(&method.name);
    let return_type = Some(scope_return_type(method));

    // Strip the first parameter ($query / $builder) that Laravel injects.
    let parameters: Vec<_> = if method.parameters.is_empty() {
        Vec::new()
    } else {
        method.parameters[1..].to_vec()
    };

    let instance_method = MethodInfo {
        name: name.clone(),
        parameters: parameters.clone(),
        return_type: return_type.clone(),
        is_static: false,
        visibility: Visibility::Public,
        conditional_return: None,
        is_deprecated: method.is_deprecated,
        template_params: Vec::new(),
        template_bindings: Vec::new(),
    };

    let static_method = MethodInfo {
        name,
        parameters,
        return_type,
        is_static: true,
        visibility: Visibility::Public,
        conditional_return: None,
        is_deprecated: method.is_deprecated,
        template_params: Vec::new(),
        template_bindings: Vec::new(),
    };

    [instance_method, static_method]
}

/// Build static virtual methods by forwarding Eloquent Builder's public
/// instance methods onto the model class.
///
/// Laravel's `Model::__callStatic()` delegates static calls to
/// `static::query()`, which returns a `Builder<static>`.  This function
/// loads the Builder class, fully resolves it (including `@mixin`
/// `Query\Builder` members), and converts each public instance method
/// into a static virtual method on the model.
///
/// Return type mapping:
/// - `static`, `$this`, `self` → `\Illuminate\Database\Eloquent\Builder<ConcreteModel>`
///   (the chain continues on the builder, not the model).
/// - Template parameters (e.g. `TModel`) → the concrete model class name.
///
/// Methods whose name starts with `__` (magic methods) are skipped.
/// Replace `\Illuminate\Database\Eloquent\Collection` with a custom
/// collection class in a type string, preserving generic parameters.
fn replace_eloquent_collection(type_str: &str, custom_collection: &str) -> String {
    let fqn_prefixed = format!("\\{ELOQUENT_COLLECTION_FQN}");
    let bare_fqn = ELOQUENT_COLLECTION_FQN;
    let replacement = if custom_collection.starts_with('\\') {
        custom_collection.to_string()
    } else {
        format!("\\{custom_collection}")
    };

    // Replace both `\Illuminate\...\Collection` and `Illuminate\...\Collection`
    // (with and without leading backslash).
    let result = type_str.replace(&fqn_prefixed, &replacement);
    result.replace(bare_fqn, replacement.trim_start_matches('\\'))
}

fn build_builder_forwarded_methods(
    class: &ClassInfo,
    class_loader: &dyn Fn(&str) -> Option<ClassInfo>,
) -> Vec<MethodInfo> {
    // Load the Eloquent Builder class.
    let builder_class = match class_loader(ELOQUENT_BUILDER_FQN) {
        Some(c) => c,
        None => return Vec::new(),
    };

    // Fully resolve Builder (own + traits + parents + virtual members
    // including @mixin Query\Builder).  This is safe because Builder
    // does not extend Model, so the LaravelModelProvider will not
    // recurse.
    let resolved_builder = Backend::resolve_class_fully(&builder_class, class_loader);

    // Build a substitution map: TModel → concrete model class name,
    // and static/$this/self → Builder<ConcreteModel>.
    let builder_self_type = format!("\\{ELOQUENT_BUILDER_FQN}<{}>", class.name);
    let mut subs = HashMap::new();
    for param in &builder_class.template_params {
        subs.insert(param.clone(), class.name.clone());
    }
    subs.insert("static".to_string(), builder_self_type.clone());
    subs.insert("$this".to_string(), builder_self_type.clone());
    subs.insert("self".to_string(), builder_self_type.clone());

    let mut methods = Vec::new();

    for method in &resolved_builder.methods {
        if method.visibility != Visibility::Public {
            continue;
        }
        // Skip magic methods (__construct, __call, etc.).
        if method.name.starts_with("__") {
            continue;
        }
        // Skip methods already present on the model (real methods,
        // scope methods, etc.).  The merge logic in
        // `merge_virtual_members` would also skip them, but filtering
        // here avoids unnecessary cloning and substitution work.
        if class
            .methods
            .iter()
            .any(|m| m.name == method.name && m.is_static)
        {
            continue;
        }

        let mut forwarded = method.clone();
        forwarded.is_static = true;

        // Apply template and self-type substitutions.
        if !subs.is_empty() {
            if let Some(ref mut ret) = forwarded.return_type {
                *ret = apply_substitution(ret, &subs);
            }
            if let Some(ref mut cond) = forwarded.conditional_return {
                apply_substitution_to_conditional(cond, &subs);
            }
            for param in &mut forwarded.parameters {
                if let Some(ref mut hint) = param.type_hint {
                    *hint = apply_substitution(hint, &subs);
                }
            }
        }

        // Replace Eloquent Collection with custom collection class.
        if let Some(ref coll) = class.custom_collection
            && let Some(ref mut ret) = forwarded.return_type
        {
            *ret = replace_eloquent_collection(ret, coll);
        }

        methods.push(forwarded);
    }

    methods
}

impl VirtualMemberProvider for LaravelModelProvider {
    /// Returns `true` if the class extends `Illuminate\Database\Eloquent\Model`.
    fn applies_to(
        &self,
        class: &ClassInfo,
        class_loader: &dyn Fn(&str) -> Option<ClassInfo>,
    ) -> bool {
        extends_eloquent_model(class, class_loader)
    }

    /// Scan the class's methods for Eloquent relationship return types,
    /// scope methods, and Builder-as-static forwarded methods.
    fn provide(
        &self,
        class: &ClassInfo,
        class_loader: &dyn Fn(&str) -> Option<ClassInfo>,
    ) -> VirtualMembers {
        let mut properties = Vec::new();
        let mut methods = Vec::new();

        for method in &class.methods {
            // ── Scope methods ───────────────────────────────────────
            if is_scope_method(method) {
                let [instance_method, static_method] = build_scope_methods(method);
                methods.push(instance_method);
                methods.push(static_method);
                continue;
            }

            // ── Legacy accessors (getXAttribute) ────────────────────
            if is_legacy_accessor(method) {
                let prop_name = legacy_accessor_property_name(&method.name);
                properties.push(PropertyInfo {
                    name: prop_name,
                    type_hint: method.return_type.clone(),
                    is_static: false,
                    visibility: Visibility::Public,
                    is_deprecated: method.is_deprecated,
                });
                continue;
            }

            // ── Modern accessors (Laravel 9+ Attribute casts) ───────
            if is_modern_accessor(method) {
                let prop_name = camel_to_snake(&method.name);
                properties.push(PropertyInfo {
                    name: prop_name,
                    type_hint: Some("mixed".to_string()),
                    is_static: false,
                    visibility: Visibility::Public,
                    is_deprecated: method.is_deprecated,
                });
                continue;
            }

            // ── Relationship properties ─────────────────────────────
            let return_type = match method.return_type.as_deref() {
                Some(rt) => rt,
                None => continue,
            };

            let kind = match classify_relationship(return_type) {
                Some(k) => k,
                None => continue,
            };

            let related_type = extract_related_type(return_type);

            // For collection relationships, use the *related* model's
            // custom_collection, not the owning model's.  For example,
            // if Product has `#[CollectedBy(ProductCollection)]` and
            // Review has `#[CollectedBy(ReviewCollection)]`, then
            // `Product::reviews()` returning `HasMany<Review, $this>`
            // should produce `ReviewCollection<Review>`, not
            // `ProductCollection<Review>`.
            let custom_collection = if kind == RelationshipKind::Collection {
                related_type
                    .as_deref()
                    .and_then(|rt| {
                        let clean = rt.strip_prefix('\\').unwrap_or(rt);
                        class_loader(clean)
                    })
                    .and_then(|related_class| related_class.custom_collection)
            } else {
                None
            };

            let type_hint =
                build_property_type(kind, related_type.as_deref(), custom_collection.as_deref());

            if type_hint.is_some() {
                properties.push(PropertyInfo {
                    name: method.name.clone(),
                    type_hint,
                    is_static: false,
                    visibility: Visibility::Public,
                    is_deprecated: false,
                });
            }
        }

        // ── Builder-as-static forwarding ────────────────────────────
        let forwarded = build_builder_forwarded_methods(class, class_loader);
        methods.extend(forwarded);

        VirtualMembers {
            methods,
            properties,
            constants: Vec::new(),
        }
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ClassLikeKind, MethodInfo, ParameterInfo};
    use std::collections::HashMap;

    /// Helper: create a minimal `ClassInfo` with the given name.
    fn make_class(name: &str) -> ClassInfo {
        ClassInfo {
            kind: ClassLikeKind::Class,
            name: name.to_string(),
            methods: Vec::new(),
            properties: Vec::new(),
            constants: Vec::new(),
            start_offset: 0,
            end_offset: 0,
            parent_class: None,
            interfaces: Vec::new(),
            used_traits: Vec::new(),
            mixins: Vec::new(),
            is_final: false,
            is_abstract: false,
            is_deprecated: false,
            template_params: Vec::new(),
            template_param_bounds: HashMap::new(),
            extends_generics: Vec::new(),
            implements_generics: Vec::new(),
            use_generics: Vec::new(),
            type_aliases: HashMap::new(),
            trait_precedences: Vec::new(),
            trait_aliases: Vec::new(),
            class_docblock: None,
            file_namespace: None,
            custom_collection: None,
        }
    }

    /// Helper: create a `MethodInfo` with a given return type.
    fn make_method(name: &str, return_type: Option<&str>) -> MethodInfo {
        MethodInfo {
            name: name.to_string(),
            parameters: Vec::new(),
            return_type: return_type.map(|s| s.to_string()),
            is_static: false,
            visibility: Visibility::Public,
            conditional_return: None,
            is_deprecated: false,
            template_params: Vec::new(),
            template_bindings: Vec::new(),
        }
    }

    /// Helper: create a `MethodInfo` with parameters.
    fn make_method_with_params(
        name: &str,
        return_type: Option<&str>,
        params: Vec<ParameterInfo>,
    ) -> MethodInfo {
        MethodInfo {
            name: name.to_string(),
            parameters: params,
            return_type: return_type.map(|s| s.to_string()),
            is_static: false,
            visibility: Visibility::Public,
            conditional_return: None,
            is_deprecated: false,
            template_params: Vec::new(),
            template_bindings: Vec::new(),
        }
    }

    /// Helper: create a `ParameterInfo`.
    fn make_param(name: &str, type_hint: Option<&str>, is_required: bool) -> ParameterInfo {
        ParameterInfo {
            name: name.to_string(),
            is_required,
            type_hint: type_hint.map(|s| s.to_string()),
            is_variadic: false,
            is_reference: false,
        }
    }

    fn no_loader(_name: &str) -> Option<ClassInfo> {
        None
    }

    // ── is_eloquent_model ───────────────────────────────────────────────

    #[test]
    fn recognises_fqn() {
        assert!(is_eloquent_model("Illuminate\\Database\\Eloquent\\Model"));
    }

    #[test]
    fn recognises_fqn_with_leading_backslash() {
        assert!(is_eloquent_model("\\Illuminate\\Database\\Eloquent\\Model"));
    }

    #[test]
    fn rejects_unrelated_class() {
        assert!(!is_eloquent_model("App\\Models\\User"));
    }

    // ── extends_eloquent_model ──────────────────────────────────────────

    #[test]
    fn direct_child_of_model() {
        let mut user = make_class("App\\Models\\User");
        user.parent_class = Some("Illuminate\\Database\\Eloquent\\Model".to_string());

        let model = make_class("Illuminate\\Database\\Eloquent\\Model");
        let loader = |name: &str| -> Option<ClassInfo> {
            if name == "Illuminate\\Database\\Eloquent\\Model" {
                Some(model.clone())
            } else {
                None
            }
        };

        assert!(extends_eloquent_model(&user, &loader));
    }

    #[test]
    fn indirect_child_of_model() {
        let mut user = make_class("App\\Models\\User");
        user.parent_class = Some("App\\Models\\BaseModel".to_string());

        let mut base_model = make_class("App\\Models\\BaseModel");
        base_model.parent_class = Some("Illuminate\\Database\\Eloquent\\Model".to_string());

        let model = make_class("Illuminate\\Database\\Eloquent\\Model");

        let loader = |name: &str| -> Option<ClassInfo> {
            match name {
                "App\\Models\\BaseModel" => Some(base_model.clone()),
                "Illuminate\\Database\\Eloquent\\Model" => Some(model.clone()),
                _ => None,
            }
        };

        assert!(extends_eloquent_model(&user, &loader));
    }

    #[test]
    fn not_a_model() {
        let service = make_class("App\\Services\\UserService");
        assert!(!extends_eloquent_model(&service, &no_loader));
    }

    // ── classify_relationship ───────────────────────────────────────────

    #[test]
    fn classify_has_one() {
        assert_eq!(
            classify_relationship("HasOne<Profile, $this>"),
            Some(RelationshipKind::Singular)
        );
    }

    #[test]
    fn classify_has_many() {
        assert_eq!(
            classify_relationship("HasMany<Post, $this>"),
            Some(RelationshipKind::Collection)
        );
    }

    #[test]
    fn classify_belongs_to() {
        assert_eq!(
            classify_relationship("BelongsTo<User, $this>"),
            Some(RelationshipKind::Singular)
        );
    }

    #[test]
    fn classify_belongs_to_many() {
        assert_eq!(
            classify_relationship("BelongsToMany<Role, $this>"),
            Some(RelationshipKind::Collection)
        );
    }

    #[test]
    fn classify_morph_one() {
        assert_eq!(
            classify_relationship("MorphOne<Image, $this>"),
            Some(RelationshipKind::Singular)
        );
    }

    #[test]
    fn classify_morph_many() {
        assert_eq!(
            classify_relationship("MorphMany<Comment, $this>"),
            Some(RelationshipKind::Collection)
        );
    }

    #[test]
    fn classify_morph_to() {
        assert_eq!(
            classify_relationship("MorphTo"),
            Some(RelationshipKind::MorphTo)
        );
    }

    #[test]
    fn classify_morph_to_many() {
        assert_eq!(
            classify_relationship("MorphToMany<Tag, $this>"),
            Some(RelationshipKind::Collection)
        );
    }

    #[test]
    fn classify_has_many_through() {
        assert_eq!(
            classify_relationship("HasManyThrough<Post, Country>"),
            Some(RelationshipKind::Collection)
        );
    }

    #[test]
    fn classify_fqn_relationship() {
        assert_eq!(
            classify_relationship(
                "\\Illuminate\\Database\\Eloquent\\Relations\\HasMany<Post, $this>"
            ),
            Some(RelationshipKind::Collection)
        );
    }

    #[test]
    fn classify_non_relationship() {
        assert_eq!(classify_relationship("string"), None);
        assert_eq!(classify_relationship("Collection<User>"), None);
    }

    #[test]
    fn classify_bare_name_without_generics() {
        assert_eq!(
            classify_relationship("HasMany"),
            Some(RelationshipKind::Collection)
        );
    }

    // ── extract_related_type ────────────────────────────────────────────

    #[test]
    fn extracts_first_generic_arg() {
        assert_eq!(
            extract_related_type("HasMany<Post, $this>"),
            Some("Post".to_string())
        );
    }

    #[test]
    fn extracts_fqn_related_type() {
        assert_eq!(
            extract_related_type("HasOne<\\App\\Models\\Profile, $this>"),
            Some("\\App\\Models\\Profile".to_string())
        );
    }

    #[test]
    fn returns_none_without_generics() {
        assert_eq!(extract_related_type("HasMany"), None);
    }

    // ── build_property_type ─────────────────────────────────────────────

    #[test]
    fn singular_with_related() {
        assert_eq!(
            build_property_type(RelationshipKind::Singular, Some("App\\Models\\Post"), None),
            Some("App\\Models\\Post".to_string())
        );
    }

    #[test]
    fn singular_without_related() {
        assert_eq!(
            build_property_type(RelationshipKind::Singular, None, None),
            None
        );
    }

    #[test]
    fn collection_with_related() {
        assert_eq!(
            build_property_type(
                RelationshipKind::Collection,
                Some("App\\Models\\Post"),
                None
            ),
            Some("\\Illuminate\\Database\\Eloquent\\Collection<App\\Models\\Post>".to_string())
        );
    }

    #[test]
    fn collection_without_related_uses_model() {
        assert_eq!(
            build_property_type(RelationshipKind::Collection, None, None),
            Some(
                "\\Illuminate\\Database\\Eloquent\\Collection<\\Illuminate\\Database\\Eloquent\\Model>"
                    .to_string()
            )
        );
    }

    #[test]
    fn morph_to_always_returns_model() {
        assert_eq!(
            build_property_type(RelationshipKind::MorphTo, Some("App\\Models\\Foo"), None),
            Some("\\Illuminate\\Database\\Eloquent\\Model".to_string())
        );
    }

    #[test]
    fn collection_with_custom_collection() {
        assert_eq!(
            build_property_type(
                RelationshipKind::Collection,
                Some("App\\Models\\Post"),
                Some("App\\Collections\\PostCollection")
            ),
            Some("\\App\\Collections\\PostCollection<App\\Models\\Post>".to_string())
        );
    }

    #[test]
    fn collection_custom_collection_with_leading_backslash() {
        assert_eq!(
            build_property_type(
                RelationshipKind::Collection,
                Some("App\\Models\\Post"),
                Some("\\App\\Collections\\PostCollection")
            ),
            Some("\\App\\Collections\\PostCollection<App\\Models\\Post>".to_string())
        );
    }

    #[test]
    fn singular_ignores_custom_collection() {
        assert_eq!(
            build_property_type(
                RelationshipKind::Singular,
                Some("App\\Models\\Post"),
                Some("App\\Collections\\PostCollection")
            ),
            Some("App\\Models\\Post".to_string())
        );
    }

    #[test]
    fn morph_to_ignores_custom_collection() {
        assert_eq!(
            build_property_type(
                RelationshipKind::MorphTo,
                Some("App\\Models\\Foo"),
                Some("App\\Collections\\FooCollection")
            ),
            Some("\\Illuminate\\Database\\Eloquent\\Model".to_string())
        );
    }

    #[test]
    fn replace_eloquent_collection_in_return_type() {
        let result = replace_eloquent_collection(
            "\\Illuminate\\Database\\Eloquent\\Collection<int, App\\Models\\User>",
            "App\\Collections\\UserCollection",
        );
        assert_eq!(
            result,
            "\\App\\Collections\\UserCollection<int, App\\Models\\User>"
        );
    }

    #[test]
    fn replace_eloquent_collection_preserves_other_types() {
        let result = replace_eloquent_collection(
            "\\Illuminate\\Support\\Collection<int, string>",
            "App\\Collections\\UserCollection",
        );
        assert_eq!(result, "\\Illuminate\\Support\\Collection<int, string>");
    }

    #[test]
    fn replace_eloquent_collection_in_union() {
        let result = replace_eloquent_collection(
            "\\Illuminate\\Database\\Eloquent\\Collection<int, App\\Models\\User>|null",
            "App\\Collections\\UserCollection",
        );
        assert_eq!(
            result,
            "\\App\\Collections\\UserCollection<int, App\\Models\\User>|null"
        );
    }

    // ── applies_to ──────────────────────────────────────────────────────

    #[test]
    fn applies_to_model_subclass() {
        let provider = LaravelModelProvider;
        let mut user = make_class("App\\Models\\User");
        user.parent_class = Some("Illuminate\\Database\\Eloquent\\Model".to_string());

        let model = make_class("Illuminate\\Database\\Eloquent\\Model");
        let loader = |name: &str| -> Option<ClassInfo> {
            if name == "Illuminate\\Database\\Eloquent\\Model" {
                Some(model.clone())
            } else {
                None
            }
        };

        assert!(provider.applies_to(&user, &loader));
    }

    #[test]
    fn does_not_apply_to_non_model() {
        let provider = LaravelModelProvider;
        let service = make_class("App\\Services\\UserService");
        assert!(!provider.applies_to(&service, &no_loader));
    }

    // ── provide: relationship properties ────────────────────────────────

    #[test]
    fn synthesizes_has_many_property() {
        let provider = LaravelModelProvider;
        let mut user = make_class("App\\Models\\User");
        user.parent_class = Some("Illuminate\\Database\\Eloquent\\Model".to_string());
        user.methods
            .push(make_method("posts", Some("HasMany<Post, $this>")));

        let result = provider.provide(&user, &no_loader);
        assert_eq!(result.properties.len(), 1);
        assert_eq!(result.properties[0].name, "posts");
        assert_eq!(
            result.properties[0].type_hint.as_deref(),
            Some("\\Illuminate\\Database\\Eloquent\\Collection<Post>")
        );
        assert_eq!(result.properties[0].visibility, Visibility::Public);
        assert!(!result.properties[0].is_static);
    }

    #[test]
    fn synthesizes_has_one_property() {
        let provider = LaravelModelProvider;
        let mut user = make_class("App\\Models\\User");
        user.parent_class = Some("Illuminate\\Database\\Eloquent\\Model".to_string());
        user.methods
            .push(make_method("profile", Some("HasOne<Profile, $this>")));

        let result = provider.provide(&user, &no_loader);
        assert_eq!(result.properties.len(), 1);
        assert_eq!(result.properties[0].name, "profile");
        assert_eq!(result.properties[0].type_hint.as_deref(), Some("Profile"));
    }

    #[test]
    fn synthesizes_belongs_to_property() {
        let provider = LaravelModelProvider;
        let mut post = make_class("App\\Models\\Post");
        post.parent_class = Some("Illuminate\\Database\\Eloquent\\Model".to_string());
        post.methods
            .push(make_method("author", Some("BelongsTo<User, $this>")));

        let result = provider.provide(&post, &no_loader);
        assert_eq!(result.properties.len(), 1);
        assert_eq!(result.properties[0].name, "author");
        assert_eq!(result.properties[0].type_hint.as_deref(), Some("User"));
    }

    #[test]
    fn synthesizes_morph_to_property() {
        let provider = LaravelModelProvider;
        let mut comment = make_class("App\\Models\\Comment");
        comment.parent_class = Some("Illuminate\\Database\\Eloquent\\Model".to_string());
        comment
            .methods
            .push(make_method("commentable", Some("MorphTo")));

        let result = provider.provide(&comment, &no_loader);
        assert_eq!(result.properties.len(), 1);
        assert_eq!(result.properties[0].name, "commentable");
        assert_eq!(
            result.properties[0].type_hint.as_deref(),
            Some("\\Illuminate\\Database\\Eloquent\\Model")
        );
    }

    #[test]
    fn synthesizes_belongs_to_many_property() {
        let provider = LaravelModelProvider;
        let mut user = make_class("App\\Models\\User");
        user.parent_class = Some("Illuminate\\Database\\Eloquent\\Model".to_string());
        user.methods
            .push(make_method("roles", Some("BelongsToMany<Role, $this>")));

        let result = provider.provide(&user, &no_loader);
        assert_eq!(result.properties.len(), 1);
        assert_eq!(result.properties[0].name, "roles");
        assert_eq!(
            result.properties[0].type_hint.as_deref(),
            Some("\\Illuminate\\Database\\Eloquent\\Collection<Role>")
        );
    }

    #[test]
    fn synthesizes_multiple_relationship_properties() {
        let provider = LaravelModelProvider;
        let mut user = make_class("App\\Models\\User");
        user.parent_class = Some("Illuminate\\Database\\Eloquent\\Model".to_string());
        user.methods
            .push(make_method("posts", Some("HasMany<Post, $this>")));
        user.methods
            .push(make_method("profile", Some("HasOne<Profile, $this>")));
        user.methods
            .push(make_method("roles", Some("BelongsToMany<Role, $this>")));

        let result = provider.provide(&user, &no_loader);
        assert_eq!(result.properties.len(), 3);

        let names: Vec<&str> = result.properties.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"posts"));
        assert!(names.contains(&"profile"));
        assert!(names.contains(&"roles"));
    }

    #[test]
    fn skips_non_relationship_methods() {
        let provider = LaravelModelProvider;
        let mut user = make_class("App\\Models\\User");
        user.parent_class = Some("Illuminate\\Database\\Eloquent\\Model".to_string());
        user.methods
            .push(make_method("getFullName", Some("string")));
        user.methods.push(make_method("save", Some("bool")));
        user.methods.push(make_method("toArray", Some("array")));

        let result = provider.provide(&user, &no_loader);
        assert!(result.properties.is_empty());
    }

    #[test]
    fn skips_methods_without_return_type() {
        let provider = LaravelModelProvider;
        let mut user = make_class("App\\Models\\User");
        user.parent_class = Some("Illuminate\\Database\\Eloquent\\Model".to_string());
        user.methods.push(make_method("posts", None));

        let result = provider.provide(&user, &no_loader);
        assert!(result.properties.is_empty());
    }

    #[test]
    fn handles_fqn_relationship_return_types() {
        let provider = LaravelModelProvider;
        let mut user = make_class("App\\Models\\User");
        user.parent_class = Some("Illuminate\\Database\\Eloquent\\Model".to_string());
        user.methods.push(make_method(
            "posts",
            Some("\\Illuminate\\Database\\Eloquent\\Relations\\HasMany<Post, $this>"),
        ));

        let result = provider.provide(&user, &no_loader);
        assert_eq!(result.properties.len(), 1);
        assert_eq!(result.properties[0].name, "posts");
        assert_eq!(
            result.properties[0].type_hint.as_deref(),
            Some("\\Illuminate\\Database\\Eloquent\\Collection<Post>")
        );
    }

    #[test]
    fn relationship_without_generics_and_singular_produces_nothing() {
        // A singular relationship without generics has no TRelated,
        // so we cannot determine the property type.
        let provider = LaravelModelProvider;
        let mut user = make_class("App\\Models\\User");
        user.parent_class = Some("Illuminate\\Database\\Eloquent\\Model".to_string());
        user.methods.push(make_method("profile", Some("HasOne")));

        let result = provider.provide(&user, &no_loader);
        assert!(
            result.properties.is_empty(),
            "Singular relationship without generics should not produce a property"
        );
    }

    #[test]
    fn collection_relationship_without_generics_uses_model_fallback() {
        // A collection relationship without generics defaults to
        // Collection<Model>.
        let provider = LaravelModelProvider;
        let mut user = make_class("App\\Models\\User");
        user.parent_class = Some("Illuminate\\Database\\Eloquent\\Model".to_string());
        user.methods.push(make_method("posts", Some("HasMany")));

        let result = provider.provide(&user, &no_loader);
        assert_eq!(result.properties.len(), 1);
        assert_eq!(
            result.properties[0].type_hint.as_deref(),
            Some(
                "\\Illuminate\\Database\\Eloquent\\Collection<\\Illuminate\\Database\\Eloquent\\Model>"
            )
        );
    }

    #[test]
    fn relationships_produce_no_virtual_methods_or_constants() {
        let provider = LaravelModelProvider;
        let mut user = make_class("App\\Models\\User");
        user.parent_class = Some("Illuminate\\Database\\Eloquent\\Model".to_string());
        user.methods
            .push(make_method("posts", Some("HasMany<Post, $this>")));

        let result = provider.provide(&user, &no_loader);
        assert!(
            result.methods.is_empty(),
            "Relationship methods should not produce virtual methods"
        );
        assert!(result.constants.is_empty());
    }

    #[test]
    fn provides_fqn_related_type_in_collection() {
        let provider = LaravelModelProvider;
        let mut user = make_class("App\\Models\\User");
        user.parent_class = Some("Illuminate\\Database\\Eloquent\\Model".to_string());
        user.methods.push(make_method(
            "posts",
            Some("HasMany<\\App\\Models\\Post, $this>"),
        ));

        let result = provider.provide(&user, &no_loader);
        assert_eq!(result.properties.len(), 1);
        assert_eq!(
            result.properties[0].type_hint.as_deref(),
            Some("\\Illuminate\\Database\\Eloquent\\Collection<\\App\\Models\\Post>")
        );
    }

    #[test]
    fn provides_fqn_related_type_singular() {
        let provider = LaravelModelProvider;
        let mut user = make_class("App\\Models\\User");
        user.parent_class = Some("Illuminate\\Database\\Eloquent\\Model".to_string());
        user.methods.push(make_method(
            "profile",
            Some("HasOne<\\App\\Models\\Profile, $this>"),
        ));

        let result = provider.provide(&user, &no_loader);
        assert_eq!(result.properties.len(), 1);
        assert_eq!(
            result.properties[0].type_hint.as_deref(),
            Some("\\App\\Models\\Profile")
        );
    }

    // ── extract_short_name ──────────────────────────────────────────────

    #[test]
    fn short_name_from_fqn() {
        assert_eq!(
            extract_short_name("\\Illuminate\\Database\\Eloquent\\Relations\\HasMany"),
            "HasMany"
        );
    }

    #[test]
    fn short_name_already_short() {
        assert_eq!(extract_short_name("HasMany"), "HasMany");
    }

    #[test]
    fn short_name_no_backslash_prefix() {
        assert_eq!(
            extract_short_name("Illuminate\\Database\\Eloquent\\Relations\\HasOne"),
            "HasOne"
        );
    }

    // ── is_scope_method ─────────────────────────────────────────────────

    #[test]
    fn scope_method_detected() {
        let method = make_method("scopeActive", Some("void"));
        assert!(is_scope_method(&method));
    }

    #[test]
    fn scope_method_multi_word() {
        let method = make_method("scopeRecentlyVerified", Some("void"));
        assert!(is_scope_method(&method));
    }

    #[test]
    fn not_a_scope_bare_scope_name() {
        // "scope" alone with no suffix is not a scope
        let method = make_method("scope", Some("void"));
        assert!(!is_scope_method(&method));
    }

    #[test]
    fn not_a_scope_different_prefix() {
        let method = make_method("getActive", Some("void"));
        assert!(!is_scope_method(&method));
    }

    #[test]
    fn not_a_scope_lowercase_prefix() {
        // Must be exactly "scope" not "Scope"
        let method = make_method("ScopeActive", Some("void"));
        assert!(!is_scope_method(&method));
    }

    // ── scope_name ──────────────────────────────────────────────────────

    #[test]
    fn scope_name_simple() {
        assert_eq!(scope_name("scopeActive"), "active");
    }

    #[test]
    fn scope_name_multi_word() {
        assert_eq!(scope_name("scopeRecentlyVerified"), "recentlyVerified");
    }

    #[test]
    fn scope_name_single_char() {
        assert_eq!(scope_name("scopeA"), "a");
    }

    #[test]
    fn scope_name_already_lowercase() {
        assert_eq!(scope_name("scopeactive"), "active");
    }

    // ── scope_return_type ───────────────────────────────────────────────

    #[test]
    fn scope_return_type_void_defaults() {
        let method = make_method("scopeActive", Some("void"));
        assert_eq!(
            scope_return_type(&method),
            "\\Illuminate\\Database\\Eloquent\\Builder<static>"
        );
    }

    #[test]
    fn scope_return_type_none_defaults() {
        let method = make_method("scopeActive", None);
        assert_eq!(
            scope_return_type(&method),
            "\\Illuminate\\Database\\Eloquent\\Builder<static>"
        );
    }

    #[test]
    fn scope_return_type_explicit() {
        let method = make_method(
            "scopeActive",
            Some("\\Illuminate\\Database\\Eloquent\\Builder<static>"),
        );
        assert_eq!(
            scope_return_type(&method),
            "\\Illuminate\\Database\\Eloquent\\Builder<static>"
        );
    }

    #[test]
    fn scope_return_type_custom() {
        let method = make_method("scopeActive", Some("\\App\\Builders\\UserBuilder"));
        assert_eq!(scope_return_type(&method), "\\App\\Builders\\UserBuilder");
    }

    // ── build_scope_methods ─────────────────────────────────────────────

    #[test]
    fn build_scope_methods_strips_query_param() {
        let method = make_method_with_params(
            "scopeActive",
            Some("void"),
            vec![make_param(
                "$query",
                Some("\\Illuminate\\Database\\Eloquent\\Builder"),
                true,
            )],
        );

        let [instance, static_m] = build_scope_methods(&method);
        assert!(instance.parameters.is_empty());
        assert!(static_m.parameters.is_empty());
    }

    #[test]
    fn build_scope_methods_preserves_extra_params() {
        let method = make_method_with_params(
            "scopeOfType",
            Some("void"),
            vec![
                make_param(
                    "$query",
                    Some("\\Illuminate\\Database\\Eloquent\\Builder"),
                    true,
                ),
                make_param("$type", Some("string"), true),
                make_param("$strict", Some("bool"), false),
            ],
        );

        let [instance, static_m] = build_scope_methods(&method);
        assert_eq!(instance.parameters.len(), 2);
        assert_eq!(instance.parameters[0].name, "$type");
        assert!(instance.parameters[0].is_required);
        assert_eq!(instance.parameters[1].name, "$strict");
        assert!(!instance.parameters[1].is_required);

        assert_eq!(static_m.parameters.len(), 2);
        assert_eq!(static_m.parameters[0].name, "$type");
        assert_eq!(static_m.parameters[1].name, "$strict");
    }

    #[test]
    fn build_scope_methods_creates_instance_and_static() {
        let method = make_method("scopeActive", Some("void"));
        let [instance, static_m] = build_scope_methods(&method);

        assert_eq!(instance.name, "active");
        assert!(!instance.is_static);
        assert_eq!(instance.visibility, Visibility::Public);

        assert_eq!(static_m.name, "active");
        assert!(static_m.is_static);
        assert_eq!(static_m.visibility, Visibility::Public);
    }

    #[test]
    fn build_scope_methods_default_return_type() {
        let method = make_method("scopeActive", None);
        let [instance, static_m] = build_scope_methods(&method);

        assert_eq!(
            instance.return_type.as_deref(),
            Some("\\Illuminate\\Database\\Eloquent\\Builder<static>")
        );
        assert_eq!(
            static_m.return_type.as_deref(),
            Some("\\Illuminate\\Database\\Eloquent\\Builder<static>")
        );
    }

    #[test]
    fn build_scope_methods_void_return_type() {
        let method = make_method("scopeActive", Some("void"));
        let [instance, _] = build_scope_methods(&method);

        assert_eq!(
            instance.return_type.as_deref(),
            Some("\\Illuminate\\Database\\Eloquent\\Builder<static>")
        );
    }

    #[test]
    fn build_scope_methods_with_no_params() {
        // Scope method without any parameters (unusual but valid)
        let method = make_method("scopeActive", Some("void"));
        let [instance, static_m] = build_scope_methods(&method);

        assert!(instance.parameters.is_empty());
        assert!(static_m.parameters.is_empty());
    }

    #[test]
    fn build_scope_methods_preserves_deprecated() {
        let mut method = make_method("scopeOld", Some("void"));
        method.is_deprecated = true;

        let [instance, static_m] = build_scope_methods(&method);
        assert!(instance.is_deprecated);
        assert!(static_m.is_deprecated);
    }

    // ── provide: scope methods (integration) ────────────────────────────

    #[test]
    fn synthesizes_scope_as_both_static_and_instance() {
        let provider = LaravelModelProvider;
        let mut user = make_class("App\\Models\\User");
        user.parent_class = Some("Illuminate\\Database\\Eloquent\\Model".to_string());
        user.methods.push(make_method_with_params(
            "scopeActive",
            Some("void"),
            vec![make_param(
                "$query",
                Some("\\Illuminate\\Database\\Eloquent\\Builder"),
                true,
            )],
        ));

        let result = provider.provide(&user, &no_loader);
        assert_eq!(result.methods.len(), 2, "Expected both static and instance");

        let instance = result.methods.iter().find(|m| !m.is_static).unwrap();
        assert_eq!(instance.name, "active");
        assert!(instance.parameters.is_empty());
        assert_eq!(
            instance.return_type.as_deref(),
            Some("\\Illuminate\\Database\\Eloquent\\Builder<static>")
        );

        let static_m = result.methods.iter().find(|m| m.is_static).unwrap();
        assert_eq!(static_m.name, "active");
        assert!(static_m.parameters.is_empty());
        assert_eq!(
            static_m.return_type.as_deref(),
            Some("\\Illuminate\\Database\\Eloquent\\Builder<static>")
        );
    }

    #[test]
    fn synthesizes_scope_with_extra_params() {
        let provider = LaravelModelProvider;
        let mut user = make_class("App\\Models\\User");
        user.parent_class = Some("Illuminate\\Database\\Eloquent\\Model".to_string());
        user.methods.push(make_method_with_params(
            "scopeOfType",
            Some("void"),
            vec![
                make_param(
                    "$query",
                    Some("\\Illuminate\\Database\\Eloquent\\Builder"),
                    true,
                ),
                make_param("$type", Some("string"), true),
            ],
        ));

        let result = provider.provide(&user, &no_loader);
        assert_eq!(result.methods.len(), 2);

        let instance = result.methods.iter().find(|m| !m.is_static).unwrap();
        assert_eq!(instance.name, "ofType");
        assert_eq!(instance.parameters.len(), 1);
        assert_eq!(instance.parameters[0].name, "$type");
        assert_eq!(instance.parameters[0].type_hint.as_deref(), Some("string"));
    }

    #[test]
    fn synthesizes_multiple_scopes() {
        let provider = LaravelModelProvider;
        let mut user = make_class("App\\Models\\User");
        user.parent_class = Some("Illuminate\\Database\\Eloquent\\Model".to_string());
        user.methods.push(make_method_with_params(
            "scopeActive",
            Some("void"),
            vec![make_param("$query", Some("Builder"), true)],
        ));
        user.methods.push(make_method_with_params(
            "scopeVerified",
            Some("void"),
            vec![make_param("$query", Some("Builder"), true)],
        ));

        let result = provider.provide(&user, &no_loader);
        // 2 scopes × 2 variants (static + instance) = 4
        assert_eq!(result.methods.len(), 4);

        let names: Vec<&str> = result
            .methods
            .iter()
            .filter(|m| !m.is_static)
            .map(|m| m.name.as_str())
            .collect();
        assert!(names.contains(&"active"));
        assert!(names.contains(&"verified"));
    }

    #[test]
    fn scope_and_relationship_coexist() {
        let provider = LaravelModelProvider;
        let mut user = make_class("App\\Models\\User");
        user.parent_class = Some("Illuminate\\Database\\Eloquent\\Model".to_string());
        user.methods
            .push(make_method("posts", Some("HasMany<Post, $this>")));
        user.methods.push(make_method_with_params(
            "scopeActive",
            Some("void"),
            vec![make_param("$query", Some("Builder"), true)],
        ));

        let result = provider.provide(&user, &no_loader);
        assert_eq!(result.properties.len(), 1, "One relationship property");
        assert_eq!(result.properties[0].name, "posts");
        assert_eq!(
            result.methods.len(),
            2,
            "Two scope methods (static + instance)"
        );
        let instance = result.methods.iter().find(|m| !m.is_static).unwrap();
        assert_eq!(instance.name, "active");
    }

    #[test]
    fn scope_method_not_treated_as_relationship() {
        // scopeActive's return type is "void", not a relationship type.
        // It should be treated as a scope, not produce a property.
        let provider = LaravelModelProvider;
        let mut user = make_class("App\\Models\\User");
        user.parent_class = Some("Illuminate\\Database\\Eloquent\\Model".to_string());
        user.methods.push(make_method_with_params(
            "scopeActive",
            Some("void"),
            vec![make_param("$query", Some("Builder"), true)],
        ));

        let result = provider.provide(&user, &no_loader);
        assert!(
            result.properties.is_empty(),
            "Scope methods should not produce relationship properties"
        );
        assert_eq!(result.methods.len(), 2);
    }

    #[test]
    fn scope_with_custom_return_type() {
        let provider = LaravelModelProvider;
        let mut user = make_class("App\\Models\\User");
        user.parent_class = Some("Illuminate\\Database\\Eloquent\\Model".to_string());
        user.methods.push(make_method_with_params(
            "scopeActive",
            Some("\\App\\Builders\\UserBuilder"),
            vec![make_param("$query", Some("Builder"), true)],
        ));

        let result = provider.provide(&user, &no_loader);
        let instance = result.methods.iter().find(|m| !m.is_static).unwrap();
        assert_eq!(
            instance.return_type.as_deref(),
            Some("\\App\\Builders\\UserBuilder")
        );
    }

    // ── Builder-as-static forwarding (unit tests) ───────────────────────

    /// Helper: create a minimal Builder class with template params and methods.
    fn make_builder(methods: Vec<MethodInfo>) -> ClassInfo {
        let mut builder = make_class(ELOQUENT_BUILDER_FQN);
        builder.template_params = vec!["TModel".to_string()];
        builder.methods = methods;
        builder
    }

    #[test]
    fn builder_forwarding_returns_empty_when_builder_not_found() {
        let class = make_class("App\\Models\\User");
        let result = build_builder_forwarded_methods(&class, &no_loader);
        assert!(result.is_empty());
    }

    #[test]
    fn builder_forwarding_converts_instance_to_static() {
        let mut builder = make_builder(vec![make_method("where", Some("static"))]);
        builder.methods[0].is_static = false;

        let user = make_class("App\\Models\\User");

        let loader = |name: &str| -> Option<ClassInfo> {
            if name == ELOQUENT_BUILDER_FQN {
                Some(builder.clone())
            } else {
                None
            }
        };

        let result = build_builder_forwarded_methods(&user, &loader);
        assert_eq!(result.len(), 1);
        assert!(result[0].is_static, "Forwarded method should be static");
        assert_eq!(result[0].name, "where");
    }

    #[test]
    fn builder_forwarding_maps_static_to_builder_self_type() {
        let builder = make_builder(vec![make_method("where", Some("static"))]);
        let user = make_class("App\\Models\\User");

        let loader = |name: &str| -> Option<ClassInfo> {
            if name == ELOQUENT_BUILDER_FQN {
                Some(builder.clone())
            } else {
                None
            }
        };

        let result = build_builder_forwarded_methods(&user, &loader);
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0].return_type.as_deref(),
            Some("\\Illuminate\\Database\\Eloquent\\Builder<App\\Models\\User>"),
            "static should map to Builder<ConcreteModel>"
        );
    }

    #[test]
    fn builder_forwarding_maps_this_to_builder_self_type() {
        let builder = make_builder(vec![make_method("orderBy", Some("$this"))]);
        let user = make_class("App\\Models\\User");

        let loader = |name: &str| -> Option<ClassInfo> {
            if name == ELOQUENT_BUILDER_FQN {
                Some(builder.clone())
            } else {
                None
            }
        };

        let result = build_builder_forwarded_methods(&user, &loader);
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0].return_type.as_deref(),
            Some("\\Illuminate\\Database\\Eloquent\\Builder<App\\Models\\User>"),
            "$this should map to Builder<ConcreteModel>"
        );
    }

    #[test]
    fn builder_forwarding_maps_self_to_builder_self_type() {
        let builder = make_builder(vec![make_method("limit", Some("self"))]);
        let user = make_class("App\\Models\\User");

        let loader = |name: &str| -> Option<ClassInfo> {
            if name == ELOQUENT_BUILDER_FQN {
                Some(builder.clone())
            } else {
                None
            }
        };

        let result = build_builder_forwarded_methods(&user, &loader);
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0].return_type.as_deref(),
            Some("\\Illuminate\\Database\\Eloquent\\Builder<App\\Models\\User>"),
            "self should map to Builder<ConcreteModel>"
        );
    }

    #[test]
    fn builder_forwarding_maps_tmodel_to_concrete_class() {
        let builder = make_builder(vec![make_method("first", Some("TModel|null"))]);
        let user = make_class("App\\Models\\User");

        let loader = |name: &str| -> Option<ClassInfo> {
            if name == ELOQUENT_BUILDER_FQN {
                Some(builder.clone())
            } else {
                None
            }
        };

        let result = build_builder_forwarded_methods(&user, &loader);
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0].return_type.as_deref(),
            Some("App\\Models\\User|null"),
            "TModel should map to the concrete model class"
        );
    }

    #[test]
    fn builder_forwarding_maps_generic_collection_return() {
        let builder = make_builder(vec![make_method(
            "get",
            Some("\\Illuminate\\Database\\Eloquent\\Collection<int, TModel>"),
        )]);
        let user = make_class("App\\Models\\User");

        let loader = |name: &str| -> Option<ClassInfo> {
            if name == ELOQUENT_BUILDER_FQN {
                Some(builder.clone())
            } else {
                None
            }
        };

        let result = build_builder_forwarded_methods(&user, &loader);
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0].return_type.as_deref(),
            Some("\\Illuminate\\Database\\Eloquent\\Collection<int, App\\Models\\User>"),
            "Collection<int, TModel> should become Collection<int, User>"
        );
    }

    #[test]
    fn builder_forwarding_maps_static_in_union() {
        let builder = make_builder(vec![make_method("whereNull", Some("static|null"))]);
        let user = make_class("App\\Models\\User");

        let loader = |name: &str| -> Option<ClassInfo> {
            if name == ELOQUENT_BUILDER_FQN {
                Some(builder.clone())
            } else {
                None
            }
        };

        let result = build_builder_forwarded_methods(&user, &loader);
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0].return_type.as_deref(),
            Some("\\Illuminate\\Database\\Eloquent\\Builder<App\\Models\\User>|null"),
            "static|null should become Builder<User>|null"
        );
    }

    #[test]
    fn builder_forwarding_skips_magic_methods() {
        let builder = make_builder(vec![
            make_method("where", Some("static")),
            make_method("__construct", None),
            make_method("__call", Some("mixed")),
        ]);
        let user = make_class("App\\Models\\User");

        let loader = |name: &str| -> Option<ClassInfo> {
            if name == ELOQUENT_BUILDER_FQN {
                Some(builder.clone())
            } else {
                None
            }
        };

        let result = build_builder_forwarded_methods(&user, &loader);
        assert_eq!(
            result.len(),
            1,
            "Only non-magic methods should be forwarded"
        );
        assert_eq!(result[0].name, "where");
    }

    #[test]
    fn builder_forwarding_skips_non_public_methods() {
        let mut builder = make_builder(vec![
            make_method("where", Some("static")),
            make_method("internalHelper", Some("void")),
        ]);
        builder.methods[1].visibility = Visibility::Protected;
        let user = make_class("App\\Models\\User");

        let loader = |name: &str| -> Option<ClassInfo> {
            if name == ELOQUENT_BUILDER_FQN {
                Some(builder.clone())
            } else {
                None
            }
        };

        let result = build_builder_forwarded_methods(&user, &loader);
        assert_eq!(result.len(), 1, "Only public methods should be forwarded");
        assert_eq!(result[0].name, "where");
    }

    #[test]
    fn builder_forwarding_skips_methods_already_on_model() {
        let builder = make_builder(vec![
            make_method("where", Some("static")),
            make_method("myMethod", Some("void")),
        ]);
        let mut user = make_class("App\\Models\\User");
        // The model has a static method named "myMethod" already.
        let mut existing = make_method("myMethod", Some("string"));
        existing.is_static = true;
        user.methods.push(existing);

        let loader = |name: &str| -> Option<ClassInfo> {
            if name == ELOQUENT_BUILDER_FQN {
                Some(builder.clone())
            } else {
                None
            }
        };

        let result = build_builder_forwarded_methods(&user, &loader);
        assert_eq!(
            result.len(),
            1,
            "Should skip 'myMethod' because the model already has it as static"
        );
        assert_eq!(result[0].name, "where");
    }

    #[test]
    fn builder_forwarding_does_not_skip_instance_method_with_same_name() {
        // If the model has an instance method named "where", the static
        // forwarded Builder method should still appear since they differ
        // in staticness.
        let builder = make_builder(vec![make_method("where", Some("static"))]);
        let mut user = make_class("App\\Models\\User");
        let mut existing = make_method("where", Some("string"));
        existing.is_static = false;
        user.methods.push(existing);

        let loader = |name: &str| -> Option<ClassInfo> {
            if name == ELOQUENT_BUILDER_FQN {
                Some(builder.clone())
            } else {
                None
            }
        };

        let result = build_builder_forwarded_methods(&user, &loader);
        assert_eq!(
            result.len(),
            1,
            "Static forwarded method should be added even when an instance method with the same name exists"
        );
        assert!(result[0].is_static);
    }

    #[test]
    fn builder_forwarding_maps_parameter_types() {
        let builder = make_builder(vec![make_method_with_params(
            "find",
            Some("TModel|null"),
            vec![make_param("$id", Some("TModel"), true)],
        )]);
        let user = make_class("App\\Models\\User");

        let loader = |name: &str| -> Option<ClassInfo> {
            if name == ELOQUENT_BUILDER_FQN {
                Some(builder.clone())
            } else {
                None
            }
        };

        let result = build_builder_forwarded_methods(&user, &loader);
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0].parameters[0].type_hint.as_deref(),
            Some("App\\Models\\User"),
            "Parameter TModel should map to the concrete model class"
        );
    }

    #[test]
    fn builder_forwarding_preserves_method_metadata() {
        let mut builder = make_builder(vec![make_method_with_params(
            "where",
            Some("static"),
            vec![
                make_param("$column", Some("string"), true),
                make_param("$value", Some("mixed"), false),
            ],
        )]);
        builder.methods[0].is_deprecated = true;

        let user = make_class("App\\Models\\User");

        let loader = |name: &str| -> Option<ClassInfo> {
            if name == ELOQUENT_BUILDER_FQN {
                Some(builder.clone())
            } else {
                None
            }
        };

        let result = build_builder_forwarded_methods(&user, &loader);
        assert_eq!(result.len(), 1);
        assert!(
            result[0].is_deprecated,
            "Deprecated flag should be preserved"
        );
        assert_eq!(result[0].parameters.len(), 2);
        assert_eq!(result[0].parameters[0].name, "$column");
        assert!(!result[0].parameters[1].is_required);
    }

    #[test]
    fn builder_forwarding_multiple_methods() {
        let builder = make_builder(vec![
            make_method("where", Some("static")),
            make_method("orderBy", Some("static")),
            make_method(
                "get",
                Some("\\Illuminate\\Database\\Eloquent\\Collection<int, TModel>"),
            ),
            make_method("first", Some("TModel|null")),
        ]);
        let user = make_class("App\\Models\\User");

        let loader = |name: &str| -> Option<ClassInfo> {
            if name == ELOQUENT_BUILDER_FQN {
                Some(builder.clone())
            } else {
                None
            }
        };

        let result = build_builder_forwarded_methods(&user, &loader);
        assert_eq!(result.len(), 4);
        let names: Vec<&str> = result.iter().map(|m| m.name.as_str()).collect();
        assert!(names.contains(&"where"));
        assert!(names.contains(&"orderBy"));
        assert!(names.contains(&"get"));
        assert!(names.contains(&"first"));
        assert!(result.iter().all(|m| m.is_static));
    }

    #[test]
    fn provide_includes_builder_forwarded_methods() {
        let provider = LaravelModelProvider;
        let mut user = make_class("App\\Models\\User");
        user.parent_class = Some("Illuminate\\Database\\Eloquent\\Model".to_string());

        let model = make_class("Illuminate\\Database\\Eloquent\\Model");
        let builder = make_builder(vec![
            make_method("where", Some("static")),
            make_method(
                "get",
                Some("\\Illuminate\\Database\\Eloquent\\Collection<int, TModel>"),
            ),
        ]);

        let loader = |name: &str| -> Option<ClassInfo> {
            if name == "Illuminate\\Database\\Eloquent\\Model" {
                Some(model.clone())
            } else if name == ELOQUENT_BUILDER_FQN {
                Some(builder.clone())
            } else {
                None
            }
        };

        let result = provider.provide(&user, &loader);

        let static_methods: Vec<&str> = result
            .methods
            .iter()
            .filter(|m| m.is_static)
            .map(|m| m.name.as_str())
            .collect();
        assert!(
            static_methods.contains(&"where"),
            "Builder's where() should be forwarded as static, got: {:?}",
            static_methods
        );
        assert!(
            static_methods.contains(&"get"),
            "Builder's get() should be forwarded as static, got: {:?}",
            static_methods
        );
    }

    #[test]
    fn provide_scope_beats_builder_method_with_same_name() {
        // If the model has a scopeWhere method AND Builder has a where
        // method, both produce static methods named "where". The scope's
        // version is added first, and merge_virtual_members would
        // deduplicate. But within the provider itself, the scope method
        // is added first, and build_builder_forwarded_methods skips
        // methods already on the class. However, scope methods are added
        // to the `methods` vec, not to the class itself, so the builder
        // dedup is based on class.methods (real methods + inherited).
        // The merge_virtual_members in mod.rs handles the final dedup.
        //
        // Here we just verify that both are produced (the dedup happens
        // at the merge layer).
        let provider = LaravelModelProvider;
        let mut user = make_class("App\\Models\\User");
        user.parent_class = Some("Illuminate\\Database\\Eloquent\\Model".to_string());
        user.methods.push(make_method_with_params(
            "scopeWhere",
            Some("void"),
            vec![make_param("$query", Some("Builder"), true)],
        ));

        let model = make_class("Illuminate\\Database\\Eloquent\\Model");
        let builder = make_builder(vec![make_method("where", Some("static"))]);

        let loader = |name: &str| -> Option<ClassInfo> {
            if name == "Illuminate\\Database\\Eloquent\\Model" {
                Some(model.clone())
            } else if name == ELOQUENT_BUILDER_FQN {
                Some(builder.clone())
            } else {
                None
            }
        };

        let result = provider.provide(&user, &loader);

        // Scope produces both static and instance "where".
        // Builder forwarding also produces a static "where".
        // merge_virtual_members will keep the first (scope) static one.
        let static_wheres: Vec<_> = result
            .methods
            .iter()
            .filter(|m| m.name == "where" && m.is_static)
            .collect();
        assert!(
            !static_wheres.is_empty(),
            "At least one static 'where' should exist from scope"
        );
        // The scope version has the default builder return type.
        assert_eq!(
            static_wheres[0].return_type.as_deref(),
            Some("\\Illuminate\\Database\\Eloquent\\Builder<static>"),
            "First static 'where' should be from the scope (added first)"
        );
    }

    #[test]
    fn builder_forwarding_with_no_return_type() {
        let builder = make_builder(vec![make_method("doSomething", None)]);
        let user = make_class("App\\Models\\User");

        let loader = |name: &str| -> Option<ClassInfo> {
            if name == ELOQUENT_BUILDER_FQN {
                Some(builder.clone())
            } else {
                None
            }
        };

        let result = build_builder_forwarded_methods(&user, &loader);
        assert_eq!(result.len(), 1);
        assert!(
            result[0].return_type.is_none(),
            "None return type should stay None"
        );
    }

    #[test]
    fn builder_forwarding_preserves_non_template_return_types() {
        let builder = make_builder(vec![
            make_method("toSql", Some("string")),
            make_method("exists", Some("bool")),
        ]);
        let user = make_class("App\\Models\\User");

        let loader = |name: &str| -> Option<ClassInfo> {
            if name == ELOQUENT_BUILDER_FQN {
                Some(builder.clone())
            } else {
                None
            }
        };

        let result = build_builder_forwarded_methods(&user, &loader);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].return_type.as_deref(), Some("string"));
        assert_eq!(result[1].return_type.as_deref(), Some("bool"));
    }

    // ── camel_to_snake ──────────────────────────────────────────────

    #[test]
    fn camel_to_snake_simple() {
        assert_eq!(camel_to_snake("FullName"), "full_name");
    }

    #[test]
    fn camel_to_snake_single_word() {
        assert_eq!(camel_to_snake("Name"), "name");
    }

    #[test]
    fn camel_to_snake_already_lower() {
        assert_eq!(camel_to_snake("name"), "name");
    }

    #[test]
    fn camel_to_snake_camel_case() {
        assert_eq!(camel_to_snake("firstName"), "first_name");
    }

    #[test]
    fn camel_to_snake_multiple_words() {
        assert_eq!(camel_to_snake("isAdminUser"), "is_admin_user");
    }

    #[test]
    fn camel_to_snake_with_digit() {
        assert_eq!(camel_to_snake("item2Name"), "item2_name");
    }

    #[test]
    fn camel_to_snake_acronym() {
        assert_eq!(camel_to_snake("URLName"), "url_name");
    }

    // ── is_legacy_accessor ──────────────────────────────────────────

    #[test]
    fn legacy_accessor_detected() {
        let method = make_method("getFullNameAttribute", Some("string"));
        assert!(is_legacy_accessor(&method));
    }

    #[test]
    fn legacy_accessor_single_word() {
        let method = make_method("getNameAttribute", Some("string"));
        assert!(is_legacy_accessor(&method));
    }

    #[test]
    fn legacy_accessor_get_attribute_itself_not_accessor() {
        // getAttribute() is a real Eloquent method, not an accessor.
        let method = make_method("getAttribute", Some("mixed"));
        assert!(!is_legacy_accessor(&method));
    }

    #[test]
    fn legacy_accessor_wrong_prefix() {
        let method = make_method("setFullNameAttribute", None);
        assert!(!is_legacy_accessor(&method));
    }

    #[test]
    fn legacy_accessor_no_attribute_suffix() {
        let method = make_method("getFullName", Some("string"));
        assert!(!is_legacy_accessor(&method));
    }

    #[test]
    fn legacy_accessor_lowercase_after_get() {
        // getfooAttribute — first char after "get" must be uppercase.
        let method = make_method("getfooAttribute", Some("string"));
        assert!(!is_legacy_accessor(&method));
    }

    // ── legacy_accessor_property_name ───────────────────────────────

    #[test]
    fn legacy_accessor_prop_name_simple() {
        assert_eq!(legacy_accessor_property_name("getNameAttribute"), "name");
    }

    #[test]
    fn legacy_accessor_prop_name_multi_word() {
        assert_eq!(
            legacy_accessor_property_name("getFullNameAttribute"),
            "full_name"
        );
    }

    #[test]
    fn legacy_accessor_prop_name_three_words() {
        assert_eq!(
            legacy_accessor_property_name("getFirstMiddleLastAttribute"),
            "first_middle_last"
        );
    }

    // ── is_modern_accessor ──────────────────────────────────────────

    #[test]
    fn modern_accessor_fqn() {
        let method = make_method(
            "fullName",
            Some("Illuminate\\Database\\Eloquent\\Casts\\Attribute"),
        );
        assert!(is_modern_accessor(&method));
    }

    #[test]
    fn modern_accessor_fqn_with_backslash() {
        let method = make_method(
            "fullName",
            Some("\\Illuminate\\Database\\Eloquent\\Casts\\Attribute"),
        );
        assert!(is_modern_accessor(&method));
    }

    #[test]
    fn modern_accessor_short_name() {
        let method = make_method("fullName", Some("Attribute"));
        assert!(is_modern_accessor(&method));
    }

    #[test]
    fn modern_accessor_with_generics() {
        let method = make_method(
            "fullName",
            Some("Illuminate\\Database\\Eloquent\\Casts\\Attribute<string, never>"),
        );
        assert!(is_modern_accessor(&method));
    }

    #[test]
    fn modern_accessor_not_matching_return_type() {
        let method = make_method("fullName", Some("string"));
        assert!(!is_modern_accessor(&method));
    }

    #[test]
    fn modern_accessor_no_return_type() {
        let method = make_method("fullName", None);
        assert!(!is_modern_accessor(&method));
    }

    // ── provide: accessor integration ───────────────────────────────

    #[test]
    fn synthesizes_legacy_accessor_property() {
        let provider = LaravelModelProvider;
        let mut user = make_class("App\\Models\\User");
        user.parent_class = Some("Illuminate\\Database\\Eloquent\\Model".to_string());
        user.methods
            .push(make_method("getFullNameAttribute", Some("string")));

        let model = make_class("Illuminate\\Database\\Eloquent\\Model");
        let loader = |name: &str| -> Option<ClassInfo> {
            if name == "Illuminate\\Database\\Eloquent\\Model" {
                Some(model.clone())
            } else {
                None
            }
        };

        let result = provider.provide(&user, &loader);
        let prop = result.properties.iter().find(|p| p.name == "full_name");
        assert!(
            prop.is_some(),
            "Legacy accessor getFullNameAttribute should produce property full_name, got: {:?}",
            result
                .properties
                .iter()
                .map(|p| &p.name)
                .collect::<Vec<_>>()
        );
        assert_eq!(prop.unwrap().type_hint.as_deref(), Some("string"));
        assert!(!prop.unwrap().is_static);
    }

    #[test]
    fn synthesizes_modern_accessor_property() {
        let provider = LaravelModelProvider;
        let mut user = make_class("App\\Models\\User");
        user.parent_class = Some("Illuminate\\Database\\Eloquent\\Model".to_string());
        user.methods.push(make_method(
            "fullName",
            Some("Illuminate\\Database\\Eloquent\\Casts\\Attribute"),
        ));

        let model = make_class("Illuminate\\Database\\Eloquent\\Model");
        let loader = |name: &str| -> Option<ClassInfo> {
            if name == "Illuminate\\Database\\Eloquent\\Model" {
                Some(model.clone())
            } else {
                None
            }
        };

        let result = provider.provide(&user, &loader);
        let prop = result.properties.iter().find(|p| p.name == "full_name");
        assert!(
            prop.is_some(),
            "Modern accessor fullName() returning Attribute should produce property full_name, got: {:?}",
            result
                .properties
                .iter()
                .map(|p| &p.name)
                .collect::<Vec<_>>()
        );
        assert_eq!(prop.unwrap().type_hint.as_deref(), Some("mixed"));
    }

    #[test]
    fn accessor_and_relationship_coexist() {
        let provider = LaravelModelProvider;
        let mut user = make_class("App\\Models\\User");
        user.parent_class = Some("Illuminate\\Database\\Eloquent\\Model".to_string());
        user.methods
            .push(make_method("getFullNameAttribute", Some("string")));
        user.methods.push(make_method(
            "posts",
            Some("HasMany<App\\Models\\Post, $this>"),
        ));

        let model = make_class("Illuminate\\Database\\Eloquent\\Model");
        let loader = |name: &str| -> Option<ClassInfo> {
            if name == "Illuminate\\Database\\Eloquent\\Model" {
                Some(model.clone())
            } else {
                None
            }
        };

        let result = provider.provide(&user, &loader);
        let prop_names: Vec<_> = result.properties.iter().map(|p| p.name.as_str()).collect();
        assert!(
            prop_names.contains(&"full_name"),
            "Should have accessor property"
        );
        assert!(
            prop_names.contains(&"posts"),
            "Should have relationship property"
        );
    }

    #[test]
    fn get_attribute_method_not_treated_as_accessor() {
        // getAttribute() is a real Eloquent method, not an accessor.
        let provider = LaravelModelProvider;
        let mut user = make_class("App\\Models\\User");
        user.parent_class = Some("Illuminate\\Database\\Eloquent\\Model".to_string());
        user.methods
            .push(make_method("getAttribute", Some("mixed")));

        let model = make_class("Illuminate\\Database\\Eloquent\\Model");
        let loader = |name: &str| -> Option<ClassInfo> {
            if name == "Illuminate\\Database\\Eloquent\\Model" {
                Some(model.clone())
            } else {
                None
            }
        };

        let result = provider.provide(&user, &loader);
        // getAttribute should not produce any virtual property.
        assert!(
            result.properties.is_empty(),
            "getAttribute() should not be treated as a legacy accessor, got: {:?}",
            result
                .properties
                .iter()
                .map(|p| &p.name)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn accessor_scope_and_relationship_all_coexist() {
        let provider = LaravelModelProvider;
        let mut user = make_class("App\\Models\\User");
        user.parent_class = Some("Illuminate\\Database\\Eloquent\\Model".to_string());
        user.methods
            .push(make_method("getFullNameAttribute", Some("string")));
        user.methods.push(make_method(
            "firstName",
            Some("Illuminate\\Database\\Eloquent\\Casts\\Attribute"),
        ));
        user.methods.push(make_method_with_params(
            "scopeActive",
            Some("void"),
            vec![make_param("$query", Some("Builder"), true)],
        ));
        user.methods.push(make_method(
            "posts",
            Some("HasMany<App\\Models\\Post, $this>"),
        ));

        let model = make_class("Illuminate\\Database\\Eloquent\\Model");
        let loader = |name: &str| -> Option<ClassInfo> {
            if name == "Illuminate\\Database\\Eloquent\\Model" {
                Some(model.clone())
            } else {
                None
            }
        };

        let result = provider.provide(&user, &loader);
        let prop_names: Vec<_> = result.properties.iter().map(|p| p.name.as_str()).collect();
        assert!(
            prop_names.contains(&"full_name"),
            "Legacy accessor property"
        );
        assert!(
            prop_names.contains(&"first_name"),
            "Modern accessor property"
        );
        assert!(prop_names.contains(&"posts"), "Relationship property");

        let method_names: Vec<_> = result.methods.iter().map(|m| m.name.as_str()).collect();
        assert!(method_names.contains(&"active"), "Scope method");
    }

    #[test]
    fn legacy_accessor_preserves_deprecated() {
        let provider = LaravelModelProvider;
        let mut user = make_class("App\\Models\\User");
        user.parent_class = Some("Illuminate\\Database\\Eloquent\\Model".to_string());

        let mut accessor = make_method("getOldNameAttribute", Some("string"));
        accessor.is_deprecated = true;
        user.methods.push(accessor);

        let model = make_class("Illuminate\\Database\\Eloquent\\Model");
        let loader = |name: &str| -> Option<ClassInfo> {
            if name == "Illuminate\\Database\\Eloquent\\Model" {
                Some(model.clone())
            } else {
                None
            }
        };

        let result = provider.provide(&user, &loader);
        let prop = result.properties.iter().find(|p| p.name == "old_name");
        assert!(prop.is_some());
        assert!(
            prop.unwrap().is_deprecated,
            "Deprecated flag should be preserved"
        );
    }

    // ── infer_relationship_from_body ─────────────────────────────────────

    #[test]
    fn infer_has_many_from_body() {
        let body = "{ return $this->hasMany(Post::class); }";
        assert_eq!(
            infer_relationship_from_body(body),
            Some("HasMany<Post>".to_string())
        );
    }

    #[test]
    fn infer_has_one_from_body() {
        let body = "{ return $this->hasOne(Profile::class); }";
        assert_eq!(
            infer_relationship_from_body(body),
            Some("HasOne<Profile>".to_string())
        );
    }

    #[test]
    fn infer_belongs_to_from_body() {
        let body = "{ return $this->belongsTo(User::class); }";
        assert_eq!(
            infer_relationship_from_body(body),
            Some("BelongsTo<User>".to_string())
        );
    }

    #[test]
    fn infer_belongs_to_many_from_body() {
        let body = "{ return $this->belongsToMany(Role::class); }";
        assert_eq!(
            infer_relationship_from_body(body),
            Some("BelongsToMany<Role>".to_string())
        );
    }

    #[test]
    fn infer_morph_one_from_body() {
        let body = "{ return $this->morphOne(Image::class, 'imageable'); }";
        assert_eq!(
            infer_relationship_from_body(body),
            Some("MorphOne<Image>".to_string())
        );
    }

    #[test]
    fn infer_morph_many_from_body() {
        let body = "{ return $this->morphMany(Comment::class, 'commentable'); }";
        assert_eq!(
            infer_relationship_from_body(body),
            Some("MorphMany<Comment>".to_string())
        );
    }

    #[test]
    fn infer_morph_to_from_body() {
        // morphTo never has a related model class argument.
        let body = "{ return $this->morphTo(); }";
        assert_eq!(
            infer_relationship_from_body(body),
            Some("MorphTo".to_string())
        );
    }

    #[test]
    fn infer_morph_to_many_from_body() {
        let body = "{ return $this->morphToMany(Tag::class, 'taggable'); }";
        assert_eq!(
            infer_relationship_from_body(body),
            Some("MorphToMany<Tag>".to_string())
        );
    }

    #[test]
    fn infer_has_many_through_from_body() {
        let body = "{ return $this->hasManyThrough(Post::class, Country::class); }";
        assert_eq!(
            infer_relationship_from_body(body),
            Some("HasManyThrough<Post>".to_string())
        );
    }

    #[test]
    fn infer_has_one_through_from_body() {
        let body = "{ return $this->hasOneThrough(Owner::class, Car::class); }";
        assert_eq!(
            infer_relationship_from_body(body),
            Some("HasOneThrough<Owner>".to_string())
        );
    }

    #[test]
    fn infer_relationship_fqn_class_argument() {
        let body = r"{ return $this->hasMany(\App\Models\Post::class); }";
        assert_eq!(
            infer_relationship_from_body(body),
            Some("HasMany<Post>".to_string())
        );
    }

    #[test]
    fn infer_relationship_with_extra_arguments() {
        let body = "{ return $this->hasMany(Post::class, 'user_id', 'id'); }";
        assert_eq!(
            infer_relationship_from_body(body),
            Some("HasMany<Post>".to_string())
        );
    }

    #[test]
    fn infer_relationship_with_whitespace() {
        let body = "{
            return $this->hasMany(  Post::class  );
        }";
        assert_eq!(
            infer_relationship_from_body(body),
            Some("HasMany<Post>".to_string())
        );
    }

    #[test]
    fn infer_no_relationship_in_empty_body() {
        let body = "{ }";
        assert_eq!(infer_relationship_from_body(body), None);
    }

    #[test]
    fn infer_no_relationship_for_non_relationship_call() {
        let body = "{ return $this->query(); }";
        assert_eq!(infer_relationship_from_body(body), None);
    }

    #[test]
    fn infer_relationship_without_class_argument() {
        // Some projects use string-based relationship definitions.
        let body = "{ return $this->hasMany('App\\Models\\Post'); }";
        assert_eq!(
            infer_relationship_from_body(body),
            Some("HasMany".to_string()),
            "Without ::class argument, returns bare relationship name"
        );
    }

    #[test]
    fn infer_morph_to_with_arguments() {
        // morphTo can optionally take a name and type column.
        let body =
            "{ return $this->morphTo('commentable', 'commentable_type', 'commentable_id'); }";
        assert_eq!(
            infer_relationship_from_body(body),
            Some("MorphTo".to_string())
        );
    }

    #[test]
    fn infer_relationship_multiline_body() {
        let body = "{
            return $this
                ->hasMany(Post::class, 'author_id');
        }";
        // The needle `$this->hasMany(` won't match across a line break,
        // so this returns None.  This is an acceptable limitation
        // documented in the todo.
        assert_eq!(infer_relationship_from_body(body), None);
    }

    #[test]
    fn infer_relationship_same_line_chain() {
        let body = "{ return $this->hasMany(Post::class)->latest(); }";
        assert_eq!(
            infer_relationship_from_body(body),
            Some("HasMany<Post>".to_string())
        );
    }

    // ── extract_class_argument ──────────────────────────────────────────

    #[test]
    fn extract_simple_class_arg() {
        assert_eq!(
            extract_class_argument("Post::class)"),
            Some("Post".to_string())
        );
    }

    #[test]
    fn extract_fqn_class_arg() {
        assert_eq!(
            extract_class_argument("\\App\\Models\\Post::class)"),
            Some("Post".to_string())
        );
    }

    #[test]
    fn extract_class_arg_with_extra_args() {
        assert_eq!(
            extract_class_argument("Post::class, 'user_id', 'id')"),
            Some("Post".to_string())
        );
    }

    #[test]
    fn extract_class_arg_with_whitespace() {
        assert_eq!(
            extract_class_argument("  Post::class  )"),
            Some("Post".to_string())
        );
    }

    #[test]
    fn extract_class_arg_no_class_token() {
        assert_eq!(extract_class_argument("'App\\Models\\Post')"), None);
    }

    #[test]
    fn extract_class_arg_no_closing_paren() {
        assert_eq!(extract_class_argument("Post::class"), None);
    }

    #[test]
    fn extract_class_arg_empty() {
        assert_eq!(extract_class_argument(")"), None);
    }

    #[test]
    fn extract_class_arg_class_in_second_arg_only() {
        // `::class` appears only after the first comma — should return None.
        assert_eq!(extract_class_argument("'taggable', Tag::class)"), None);
    }

    // ── provider integration with body-inferred relationships ───────────

    #[test]
    fn synthesizes_property_from_body_inferred_has_many() {
        let provider = LaravelModelProvider;
        let mut user = make_class("App\\Models\\User");
        user.parent_class = Some("Illuminate\\Database\\Eloquent\\Model".to_string());

        // Method with no @return annotation — return_type is set by
        // the parser from body inference.
        user.methods.push(MethodInfo {
            name: "posts".to_string(),
            parameters: Vec::new(),
            return_type: Some("HasMany<Post>".to_string()),
            is_static: false,
            visibility: Visibility::Public,
            conditional_return: None,
            is_deprecated: false,
            template_params: Vec::new(),
            template_bindings: Vec::new(),
        });

        let model = make_class("Illuminate\\Database\\Eloquent\\Model");
        let loader = |name: &str| -> Option<ClassInfo> {
            if name == "Illuminate\\Database\\Eloquent\\Model" {
                Some(model.clone())
            } else {
                None
            }
        };

        let result = provider.provide(&user, &loader);
        let prop = result.properties.iter().find(|p| p.name == "posts");
        assert!(
            prop.is_some(),
            "Body-inferred HasMany<Post> should produce a 'posts' property"
        );
    }

    #[test]
    fn synthesizes_property_from_body_inferred_morph_to() {
        let provider = LaravelModelProvider;
        let mut comment = make_class("App\\Models\\Comment");
        comment.parent_class = Some("Illuminate\\Database\\Eloquent\\Model".to_string());

        // morphTo inferred from body — bare name, no generics.
        comment.methods.push(MethodInfo {
            name: "commentable".to_string(),
            parameters: Vec::new(),
            return_type: Some("MorphTo".to_string()),
            is_static: false,
            visibility: Visibility::Public,
            conditional_return: None,
            is_deprecated: false,
            template_params: Vec::new(),
            template_bindings: Vec::new(),
        });

        let model = make_class("Illuminate\\Database\\Eloquent\\Model");
        let loader = |name: &str| -> Option<ClassInfo> {
            if name == "Illuminate\\Database\\Eloquent\\Model" {
                Some(model.clone())
            } else {
                None
            }
        };

        let result = provider.provide(&comment, &loader);
        let prop = result.properties.iter().find(|p| p.name == "commentable");
        assert!(
            prop.is_some(),
            "Body-inferred MorphTo should produce a 'commentable' property"
        );
    }
}
