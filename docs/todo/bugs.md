# PHPantom — Bug Fixes

## B12. `Collection::reduce()` generic return type not inferred
**Impact: Low · Effort: Medium**

Pattern:
```php
$result = $collection
    ->reduce(fn(Decimal $carry, OrderProduct $p): Decimal => $carry->add($p->price), new Decimal('0'))
    ->add($total);  // unresolved
```

The `reduce()` method on Laravel collections has this signature:
```
@template TReduceInitial
@template TReduceReturnType
@param callable(TReduceInitial|TReduceReturnType, TValue, TKey): TReduceReturnType $callback
@param TReduceInitial $initial
@return TReduceReturnType
```

PHPantom should infer:
- `TReduceInitial = Decimal` (from the `$initial` argument)
- `TReduceReturnType = Decimal` (from the closure return type hint)
- Therefore `reduce()` returns `Decimal`

The bidirectional template inference (`4329efe`) partially addresses
this, but `reduce()` still returns unresolved. The likely gap is the
union `TReduceInitial|TReduceReturnType` in the callable's first
parameter position: the inference engine may not decompose the union
to extract individual template bindings when both templates appear
in the same callable parameter type.

**Observed in:** `FlowService:517` — `->reduce(fn(Decimal $c, ...):
Decimal => ..., new Decimal('0'))->add($total)`.

---

## B13. Array shape tracking from keyed literal assignments in loops
**Impact: Low · Effort: High**

Pattern:
```php
$bundleProductCounts = [];
foreach ($items as $item) {
    $bundleProductCounts[$item->id] = [
        'bundle' => $item->productBundle,
        'count'  => 1,
    ];
}
foreach ($bundleProductCounts as $entry) {
    $entry['bundle']->parentProduct();  // unresolved
}
```

PHPantom tracks array value types from variable-key assignments
(`$arr[$key] = $value`), but when the value is an array literal with
string keys (a shape), the element type is not preserved as a shape.
Subsequent access like `$entry['bundle']->method()` requires knowing
that `'bundle'` maps to a specific class type.

**Observed in:** `ProductSupplyAmountChangeListener:58` — array built
with `['bundle' => $productBundle, 'count' => 1]` in a loop, then
iterated; `$bundleProductCount['bundle']->parentProduct()` is
unresolvable because the shape is lost.

**Depends on:** T19 (structured type representation) or at minimum
a basic array shape inference that preserves `array{key: Type}` from
literal array constructors and propagates it through foreach.