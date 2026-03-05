//! Code actions — `textDocument/codeAction` handler.
//!
//! This module provides code actions for PHP files:
//!
//! - **Import class** — when the cursor is on an unresolved class name,
//!   offer to add a `use` statement for matching classes found in the
//!   class index, classmap, and stubs.
//! - **Remove unused import** — when the cursor is on (or a diagnostic
//!   overlaps with) an unused `use` statement, offer to remove it.
//!   Also offers a bulk "Remove all unused imports" action.

mod import_class;
mod remove_unused_import;

use tower_lsp::lsp_types::*;

use crate::Backend;

impl Backend {
    /// Handle a `textDocument/codeAction` request.
    ///
    /// Returns a list of code actions applicable at the given range.
    pub(crate) fn handle_code_action(
        &self,
        uri: &str,
        content: &str,
        params: &CodeActionParams,
    ) -> Vec<CodeActionOrCommand> {
        let mut actions = Vec::new();

        // ── Import class ────────────────────────────────────────────────
        self.collect_import_class_actions(uri, content, params, &mut actions);

        // ── Remove unused imports ───────────────────────────────────────
        self.collect_remove_unused_import_actions(uri, content, params, &mut actions);

        actions
    }
}
