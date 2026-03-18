# PHPantom — Refactoring

Technical debt and internal cleanup tasks. This document is the first
item in every sprint. The sprint cannot begin feature work until this
gate is clear.

> **Housekeeping:** When a task is completed, remove it from this
> document entirely. Do not strike through or mark as done.

## Sprint-opening gate process

Every sprint lists "Clear refactoring gate" as its first item,
linking here. When an agent starts a sprint, follow these steps:

1. **Resolve outstanding items.** If this document contains any tasks,
   work through them. Remove each one as it is completed.
2. **Request a fresh session.** After completing refactoring work,
   stop and ask the user to start a new session. Analysis must happen
   in a session where no refactoring work was performed (since loading
   `AGENTS.md`). This ensures the analyst is not biased by the work
   just done.
3. **Analyze (fresh session only).** In a fresh session with no
   outstanding items, review the codebase for technical debt that
   would hinder the current sprint's tasks. Follow the checklist
   below. If issues are found, add them to this document, work through
   them, and go back to step 2.
4. **Declare the gate clear.** When a fresh-session analysis finds no
   issues worth adding, remove the "Clear refactoring gate" row from
   the current sprint table. The sprint is now open for feature work.

A "fresh session" means one where no refactoring edits have been made
since the session started. The point is to get an unbiased second look
at the codebase after cleanup, not to rubber-stamp work just completed
in the same context.

---

## Analysis checklist

Run through every item below when performing step 3. For each item,
actually read the relevant files — do not rely on memory or prior
context. The goal is to catch things that would slow down the sprint or
introduce bugs during it.

### File size and module boundaries

- Read the files most likely to be touched by this sprint's tasks.
  Any file over ~600 lines is a candidate for splitting. Look for
  natural seams: logically distinct groups of functions, multiple
  unrelated `impl` blocks, or a section that is already commented
  as a separate concern.
- Check whether any module is doing two jobs (e.g. parsing _and_
  resolution, or building _and_ formatting). If the sprint will
  add a third job to the same file, split it now.
- Look for `mod.rs` files that have grown beyond a thin re-export
  layer. Logic that lives in `mod.rs` is harder to find and test.

### Test placement

- Check whether any `#[cfg(test)]` blocks exist inside `src/` files
  rather than in `tests/`. Inline tests are fine for pure unit tests
  on private helpers, but integration tests and anything that touches
  the `Backend` or multi-file resolution should live in `tests/`.
- Check whether the existing `tests/` files cover the modules the
  sprint will modify. Missing coverage now means broken code goes
  undetected later.
- Look for test helper code duplicated across multiple test files.
  If the same fixture setup or assertion pattern appears more than
  twice, it belongs in `tests/common/mod.rs`.

### Code duplication

- Grep for structurally similar functions across the modules the
  sprint will touch. Duplicated logic that diverges under maintenance
  is a reliability risk.
- Pay particular attention to: type string manipulation, AST node
  offset extraction, docblock text extraction, and `WorkspaceEdit`
  construction. These patterns tend to proliferate.
- If two code action handlers share a non-trivial pattern (e.g. "find
  the token at the cursor, determine its span, build an edit"), check
  whether a shared helper already exists or should be created before
  the sprint adds a third copy.

### Performance and memory

- Look for any place where the full file AST is re-parsed inside a
  hot path (completion, hover, diagnostics). Re-parsing should happen
  at most once per request and the result passed down, not re-derived.
- Look for unbounded clones of `ClassInfo`, `MethodInfo`, or other
  large structs inside loops. These should be references or
  `Arc`-wrapped.
- Check whether any new data structures added in the previous sprint
  are stored per-file but never evicted. Unbounded growth in
  `DashMap` entries is a memory leak.
- Look for `Vec::contains` or `Vec::iter().find()` used as a set
  membership check on collections that could grow with the number of
  files. These should be `HashSet` or `DashSet`.

### Fragility and error handling

- Look for `unwrap()` and `expect()` calls in request-handling code
  paths (anything reachable from `server.rs`). A panic in a request
  handler crashes the language server. These should be `?` or
  explicit early returns.
- Check whether the sprint's target modules propagate errors up or
  silently swallow them with `let _ = ...` or empty `Err(_) => {}`
  arms. Silent failures produce confusing user-visible behaviour.
- Look for code that assumes a particular UTF-8 byte offset is a valid
  char boundary without checking. This is a common source of panics
  when files contain multibyte characters.
- Check whether any `Arc<RwLock<...>>` or `Arc<Mutex<...>>` is held
  across an `await` point or across a call that re-acquires the same
  lock. These cause deadlocks or unnecessary blocking.

### Sprint-specific concerns

After working through the general items above, read the sprint's
feature tasks and ask:

- Will any task require touching a module that is already large or
  doing too many things? If so, split it now.
- Will any task duplicate logic that already exists elsewhere? If so,
  extract the shared helper first.
- Will any task add a new data structure that needs an eviction path?
  Plan the eviction before writing the feature.
- Will any task generate `WorkspaceEdit` responses? Check that the
  existing edit-building helpers (if any) are adequate, or that a
  new shared helper should be written before the first action is
  implemented.

---

### What belongs here

Only add items that would actively hinder the upcoming sprint's work
or that have accumulated enough friction to justify a focused cleanup
pass. Small fixes that can be done inline during feature work should
just be done inline. Items do not need to be scoped to the sprint's
feature area, but they should be completable in reasonable time (not
multi-week rewrites that would stall the sprint indefinitely).

---

No outstanding items.
