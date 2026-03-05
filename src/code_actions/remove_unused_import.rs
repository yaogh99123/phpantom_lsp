//! Remove unused import code action.
//!
//! When the cursor overlaps with an unused `use` statement (identified by
//! matching diagnostics with `DiagnosticTag::Unnecessary`), offer:
//!
//! 1. A per-import quick-fix: `Remove unused import 'Foo\Bar'`
//! 2. A bulk action: `Remove all unused imports` (when ≥ 2 unused imports exist)
//!
//! The detection reuses the same logic as `diagnostics::unused_imports` —
//! we collect unused-import diagnostics and then generate `TextEdit`s that
//! delete the corresponding lines.

use std::collections::HashMap;

use tower_lsp::lsp_types::*;

use crate::Backend;

impl Backend {
    /// Collect "Remove unused import" code actions.
    ///
    /// For each unused-import diagnostic that overlaps with the request
    /// range, offer a quick-fix to remove it.  When there are two or more
    /// unused imports in the file, also offer a bulk "Remove all unused
    /// imports" action.
    pub(crate) fn collect_remove_unused_import_actions(
        &self,
        uri: &str,
        content: &str,
        params: &CodeActionParams,
        out: &mut Vec<CodeActionOrCommand>,
    ) {
        // ── Collect all unused-import diagnostics for this file ─────────
        let mut all_unused_diags: Vec<Diagnostic> = Vec::new();
        self.collect_unused_import_diagnostics(uri, content, &mut all_unused_diags);

        if all_unused_diags.is_empty() {
            return;
        }

        let doc_uri: Url = match uri.parse() {
            Ok(u) => u,
            Err(_) => return,
        };

        // ── Find diagnostics that overlap with the request range ────────
        let overlapping: Vec<&Diagnostic> = all_unused_diags
            .iter()
            .filter(|d| ranges_overlap(&d.range, &params.range))
            .collect();

        for diag in &overlapping {
            let removal_edit = build_line_deletion_edit(content, &diag.range);

            let title = format!(
                "Remove {}",
                diag.message
                    .strip_prefix("Unused import ")
                    .map(|rest| format!("unused import {rest}"))
                    .unwrap_or_else(|| "unused import".to_string())
            );

            let mut changes = HashMap::new();
            changes.insert(doc_uri.clone(), vec![removal_edit]);

            out.push(CodeActionOrCommand::CodeAction(CodeAction {
                title,
                kind: Some(CodeActionKind::QUICKFIX),
                diagnostics: Some(vec![(*diag).clone()]),
                edit: Some(WorkspaceEdit {
                    changes: Some(changes),
                    document_changes: None,
                    change_annotations: None,
                }),
                command: None,
                is_preferred: Some(true),
                disabled: None,
                data: None,
            }));
        }

        // ── Bulk action: remove ALL unused imports ──────────────────────
        if all_unused_diags.len() >= 2 {
            let mut bulk_edits: Vec<TextEdit> = all_unused_diags
                .iter()
                .map(|d| build_line_deletion_edit(content, &d.range))
                .collect();

            // Sort edits in reverse order so that byte offsets remain
            // valid as we apply deletions from bottom to top.
            bulk_edits.sort_by(|a, b| b.range.start.cmp(&a.range.start));

            let mut changes = HashMap::new();
            changes.insert(doc_uri.clone(), bulk_edits);

            out.push(CodeActionOrCommand::CodeAction(CodeAction {
                title: format!("Remove all {} unused imports", all_unused_diags.len()),
                kind: Some(CodeActionKind::new("source.organizeImports")),
                diagnostics: Some(all_unused_diags),
                edit: Some(WorkspaceEdit {
                    changes: Some(changes),
                    document_changes: None,
                    change_annotations: None,
                }),
                command: None,
                is_preferred: None,
                disabled: None,
                data: None,
            }));
        }
    }
}

/// Check whether two LSP ranges overlap (share at least one position).
fn ranges_overlap(a: &Range, b: &Range) -> bool {
    a.start <= b.end && b.start <= a.end
}

/// Build a `TextEdit` that deletes the line(s) covered by `range`,
/// including the trailing newline so no blank lines accumulate.
///
/// For group import members (where the diagnostic range covers just the
/// member name within `{...}`), this deletes only the member text plus
/// its trailing comma/space.
fn build_line_deletion_edit(content: &str, range: &Range) -> TextEdit {
    let lines: Vec<&str> = content.split('\n').collect();

    let start_line = range.start.line as usize;
    let end_line = range.end.line as usize;

    // Check if this diagnostic covers a full `use` statement line.
    // If the range spans from the `use` keyword to the semicolon (or end
    // of line), we delete the entire line including its newline.
    let is_full_line = if start_line == end_line && start_line < lines.len() {
        let line = lines[start_line];
        let trimmed = line.trim();
        let leading_ws = line.len() - trimmed.len();
        // Check if the diagnostic range covers the whole trimmed content
        // of a `use` statement line (not just a member inside a group).
        let range_covers_full_line = range.start.character as usize <= leading_ws
            && range.end.character as usize >= leading_ws + trimmed.len();
        range_covers_full_line && trimmed.starts_with("use ") && trimmed.ends_with(';')
    } else {
        false
    };

    if is_full_line {
        // Delete the entire line including the trailing newline.
        let delete_end_line = end_line + 1;
        TextEdit {
            range: Range {
                start: Position::new(start_line as u32, 0),
                end: Position::new(delete_end_line as u32, 0),
            },
            new_text: String::new(),
        }
    } else {
        // Partial deletion (e.g. a member inside a group import).
        // Delete the exact range the diagnostic covers.
        //
        // For group members we also try to clean up a trailing comma
        // and whitespace to keep the group tidy.
        let extended_range = extend_range_for_group_member(content, range);
        TextEdit {
            range: extended_range,
            new_text: String::new(),
        }
    }
}

/// When removing a member from a group import (`use Foo\{Bar, Baz};`),
/// extend the deletion range to include the trailing comma and
/// whitespace (or leading comma and whitespace if it's the last member).
fn extend_range_for_group_member(content: &str, range: &Range) -> Range {
    let lines: Vec<&str> = content.split('\n').collect();
    let line_idx = range.end.line as usize;
    if line_idx >= lines.len() {
        return *range;
    }
    let line = lines[line_idx];
    let end_char = range.end.character as usize;

    // Check for trailing comma + optional whitespace after the member name.
    let after = &line[end_char..];
    if let Some(rest) = after.strip_prefix(',') {
        // Consume optional whitespace after the comma.
        let extra_ws = rest.len() - rest.trim_start().len();
        let new_end_char = end_char + 1 + extra_ws; // 1 for comma + whitespace
        return Range {
            start: range.start,
            end: Position::new(range.end.line, new_end_char as u32),
        };
    }

    // If there's no trailing comma, this might be the last member.
    // Check for a leading comma + whitespace before the member name.
    let start_char = range.start.character as usize;
    let line_for_start = lines[range.start.line as usize];
    let before = &line_for_start[..start_char];
    if before.ends_with(", ") {
        return Range {
            start: Position::new(range.start.line, (start_char - 2) as u32),
            end: range.end,
        };
    }
    if before.ends_with(',') {
        return Range {
            start: Position::new(range.start.line, (start_char - 1) as u32),
            end: range.end,
        };
    }

    *range
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── ranges_overlap tests ────────────────────────────────────────────

    #[test]
    fn overlapping_ranges() {
        let a = Range::new(Position::new(1, 0), Position::new(1, 10));
        let b = Range::new(Position::new(1, 5), Position::new(1, 15));
        assert!(ranges_overlap(&a, &b));
    }

    #[test]
    fn non_overlapping_ranges() {
        let a = Range::new(Position::new(1, 0), Position::new(1, 5));
        let b = Range::new(Position::new(2, 0), Position::new(2, 5));
        assert!(!ranges_overlap(&a, &b));
    }

    #[test]
    fn touching_ranges_overlap() {
        let a = Range::new(Position::new(1, 0), Position::new(1, 5));
        let b = Range::new(Position::new(1, 5), Position::new(1, 10));
        assert!(ranges_overlap(&a, &b));
    }

    #[test]
    fn cursor_inside_range() {
        // Cursor at a single point inside the range.
        let a = Range::new(Position::new(3, 0), Position::new(3, 20));
        let b = Range::new(Position::new(3, 5), Position::new(3, 5));
        assert!(ranges_overlap(&a, &b));
    }

    // ── build_line_deletion_edit tests ───────────────────────────────────

    #[test]
    fn deletes_full_use_line() {
        let content = "<?php\nuse Foo\\Bar;\nuse Baz\\Qux;\n";
        let range = Range::new(Position::new(1, 0), Position::new(1, 12));
        let edit = build_line_deletion_edit(content, &range);
        assert_eq!(edit.new_text, "");
        assert_eq!(edit.range.start, Position::new(1, 0));
        assert_eq!(edit.range.end, Position::new(2, 0));
    }

    #[test]
    fn deletes_partial_group_member_trailing_comma() {
        // `use Foo\{Bar, Baz};` — removing "Bar" which has a trailing ", "
        let content = "<?php\nuse Foo\\{Bar, Baz};\n";
        // Range covering just "Bar" inside the braces.
        let range = Range::new(Position::new(1, 9), Position::new(1, 12));
        let edit = build_line_deletion_edit(content, &range);
        // Should extend to include the trailing ", "
        assert_eq!(edit.range.start, Position::new(1, 9));
        assert_eq!(edit.range.end, Position::new(1, 14)); // "Bar, " = 5 chars from 9
    }

    // ── Integration test: remove unused import action ───────────────────

    #[test]
    fn remove_action_offered_for_unused_import() {
        let backend = crate::Backend::new_test();
        let uri = "file:///test.php";
        let content = "<?php\nnamespace App;\n\nuse Foo\\Bar;\n\nclass Baz {}\n";

        backend.update_ast(uri, content);

        let params = CodeActionParams {
            text_document: TextDocumentIdentifier {
                uri: uri.parse().unwrap(),
            },
            range: Range {
                start: Position::new(3, 0),
                end: Position::new(3, 0),
            },
            context: CodeActionContext {
                diagnostics: vec![],
                only: None,
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let actions = backend.handle_code_action(uri, content, &params);
        let remove_actions: Vec<_> = actions
            .iter()
            .filter(|a| match a {
                CodeActionOrCommand::CodeAction(ca) => ca.title.starts_with("Remove"),
                _ => false,
            })
            .collect();
        assert!(
            !remove_actions.is_empty(),
            "expected at least one remove action for unused import"
        );
    }

    #[test]
    fn no_remove_action_for_used_import() {
        let backend = crate::Backend::new_test();
        let uri = "file:///test.php";
        let content = "<?php\nnamespace App;\n\nuse Foo\\Bar;\n\nclass Baz extends Bar {}\n";

        backend.update_ast(uri, content);

        let params = CodeActionParams {
            text_document: TextDocumentIdentifier {
                uri: uri.parse().unwrap(),
            },
            range: Range {
                start: Position::new(3, 0),
                end: Position::new(3, 0),
            },
            context: CodeActionContext {
                diagnostics: vec![],
                only: None,
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let actions = backend.handle_code_action(uri, content, &params);
        let remove_actions: Vec<_> = actions
            .iter()
            .filter(|a| match a {
                CodeActionOrCommand::CodeAction(ca) => ca.title.starts_with("Remove"),
                _ => false,
            })
            .collect();
        assert!(
            remove_actions.is_empty(),
            "should not offer remove for used import, got: {:?}",
            remove_actions
                .iter()
                .map(|a| match a {
                    CodeActionOrCommand::CodeAction(ca) => ca.title.clone(),
                    CodeActionOrCommand::Command(c) => c.title.clone(),
                })
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn bulk_remove_offered_when_multiple_unused() {
        let backend = crate::Backend::new_test();
        let uri = "file:///test.php";
        let content = "<?php\nnamespace App;\n\nuse Foo\\Bar;\nuse Baz\\Qux;\n\nclass X {}\n";

        backend.update_ast(uri, content);

        let params = CodeActionParams {
            text_document: TextDocumentIdentifier {
                uri: uri.parse().unwrap(),
            },
            range: Range {
                start: Position::new(3, 0),
                end: Position::new(3, 0),
            },
            context: CodeActionContext {
                diagnostics: vec![],
                only: None,
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let actions = backend.handle_code_action(uri, content, &params);
        let bulk_action = actions.iter().find(|a| match a {
            CodeActionOrCommand::CodeAction(ca) => ca.title.starts_with("Remove all"),
            _ => false,
        });
        assert!(
            bulk_action.is_some(),
            "expected a bulk 'Remove all unused imports' action"
        );

        // Verify the bulk action title includes the count.
        if let Some(CodeActionOrCommand::CodeAction(ca)) = bulk_action {
            assert!(
                ca.title.contains('2'),
                "bulk action title should mention the count, got: {}",
                ca.title
            );
        }
    }
}
