use std::collections::btree_map::Entry;
use std::collections::{HashMap, HashSet};
use std::fmt::Display;

use crate::editor::Action;
use crate::events::{
    DiagnosticsDidChange, DocumentDidChange, DocumentDidClose, LanguageServerInitialized,
};
use crate::{DocumentId, Editor, ViewId};
use helix_core::diagnostic::DiagnosticProvider;
use helix_core::Uri;
use helix_event::register_hook;
use helix_lsp::util::generate_transaction_from_edits;
use helix_lsp::{lsp, LanguageServerId, OffsetEncoding};

use super::Handlers;

pub struct DocumentColorsEvent(pub DocumentId);
pub struct DocumentLinksEvent(pub DocumentId);

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum SignatureHelpInvoked {
    Automatic,
    Manual,
}

pub enum SignatureHelpEvent {
    Invoked,
    Trigger,
    ReTrigger,
    Cancel,
    RequestComplete { open: bool },
}

pub struct PullDiagnosticsEvent {
    pub document_id: DocumentId,
}

pub struct PullAllDocumentsDiagnosticsEvent {
    pub language_servers: HashSet<LanguageServerId>,
}

pub struct CodeActionHintEvent {
    pub document_id: DocumentId,
    pub view_id: ViewId,
}

#[derive(Debug)]
pub struct ApplyEditError {
    pub kind: ApplyEditErrorKind,
    pub failed_change_idx: usize,
}

#[derive(Debug)]
pub enum ApplyEditErrorKind {
    DocumentChanged,
    FileNotFound,
    InvalidUrl(helix_core::uri::UrlConversionError),
    IoError(std::io::Error),
}

impl From<std::io::Error> for ApplyEditErrorKind {
    fn from(err: std::io::Error) -> Self {
        ApplyEditErrorKind::IoError(err)
    }
}

impl From<helix_core::uri::UrlConversionError> for ApplyEditErrorKind {
    fn from(err: helix_core::uri::UrlConversionError) -> Self {
        ApplyEditErrorKind::InvalidUrl(err)
    }
}

impl Display for ApplyEditErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApplyEditErrorKind::DocumentChanged => f.write_str("document has changed"),
            ApplyEditErrorKind::FileNotFound => f.write_str("file not found"),
            ApplyEditErrorKind::InvalidUrl(err) => f.write_str(&format!("{err}")),
            ApplyEditErrorKind::IoError(err) => f.write_str(&format!("{err}")),
        }
    }
}

fn collect_text_edits(
    edits: &[lsp::OneOf<lsp::TextEdit, lsp::AnnotatedTextEdit>],
) -> Vec<lsp::TextEdit> {
    edits
        .iter()
        .map(|edit| match edit {
            lsp::OneOf::Left(text_edit) => text_edit,
            lsp::OneOf::Right(annotated_text_edit) => &annotated_text_edit.text_edit,
        })
        .cloned()
        .collect()
}

impl Editor {
    /// Resolve the target of a text edit and confirm it can be edited.
    ///
    /// This opens the buffer and checks the document version when one is given.
    /// It changes no document content, so it is safe to run as a validation
    /// step before any edit in a workspace edit is applied.
    fn validate_text_edit_target(
        &mut self,
        url: &helix_lsp::Url,
        version: Option<i32>,
    ) -> Result<DocumentId, ApplyEditErrorKind> {
        let uri = match Uri::try_from(url) {
            Ok(uri) => uri,
            Err(err) => {
                log::error!("{err}");
                return Err(err.into());
            }
        };
        let path = uri.as_path().expect("URIs are valid paths");

        let doc_id = match self.open(path, Action::Load) {
            Ok(doc_id) => doc_id,
            Err(err) => {
                let err = format!(
                    "failed to open document: {}: {}",
                    path.to_string_lossy(),
                    err
                );
                log::error!("{}", err);
                self.set_error(err);
                return Err(ApplyEditErrorKind::FileNotFound);
            }
        };

        let doc = doc_mut!(self, &doc_id);
        if let Some(version) = version {
            if version != doc.version() {
                let err = format!("outdated workspace edit for {path:?}");
                log::error!("{err}, expected {} but got {version}", doc.version());
                self.set_error(err);
                return Err(ApplyEditErrorKind::DocumentChanged);
            }
        }

        Ok(doc_id)
    }

    /// Apply one document's edits and commit them to history.
    fn apply_resolved_text_edits(
        &mut self,
        doc_id: DocumentId,
        text_edits: Vec<lsp::TextEdit>,
        offset_encoding: OffsetEncoding,
    ) {
        // Need to determine a view for apply/append_changes_to_history
        let view_id = self.get_synced_view_id(doc_id);
        let doc = doc_mut!(self, &doc_id);
        let transaction = generate_transaction_from_edits(doc.text(), text_edits, offset_encoding);
        let view = view_mut!(self, view_id);
        // Validation opened the buffer and matched its version, and the
        // transaction is built from the same text it is applied to here. No
        // path is left for apply to fail. A false return means an invariant
        // broke, so make it loud in tests and visible in release.
        let applied = doc.apply(&transaction, view.id);
        debug_assert!(
            applied,
            "workspace edit failed to apply to {doc_id:?} after validation"
        );
        if !applied {
            log::error!("workspace edit failed to apply to {doc_id:?} after validation");
        }
        doc.append_changes_to_history(view);
    }

    /// Apply a single text edit immediately, opening and version-checking the
    /// target first. Used by the abort path where edits are applied in order.
    fn apply_text_edit_group(
        &mut self,
        url: &helix_lsp::Url,
        version: Option<i32>,
        text_edits: Vec<lsp::TextEdit>,
        offset_encoding: OffsetEncoding,
    ) -> Result<(), ApplyEditErrorKind> {
        let doc_id = self.validate_text_edit_target(url, version)?;
        self.apply_resolved_text_edits(doc_id, text_edits, offset_encoding);
        Ok(())
    }

    /// Apply a set of text edits atomically.
    ///
    /// Every edit is resolved and version-checked before any of them is
    /// applied. If validation fails the whole set is rejected and no document
    /// is touched. Edits targeting the same document are combined into one
    /// transaction so their offsets stay correct. Combining relies on the LSP
    /// rule that edits within a workspace edit do not overlap. Overlapping
    /// edits are dropped when the transaction is built.
    fn apply_text_edits_transactionally<'a>(
        &mut self,
        edits: impl Iterator<Item = (&'a helix_lsp::Url, Option<i32>, Vec<lsp::TextEdit>)>,
        offset_encoding: OffsetEncoding,
    ) -> Result<(), ApplyEditError> {
        let mut grouped: HashMap<DocumentId, Vec<lsp::TextEdit>> = HashMap::new();
        for (i, (url, version, text_edits)) in edits.enumerate() {
            let doc_id = self
                .validate_text_edit_target(url, version)
                .map_err(|kind| ApplyEditError {
                    kind,
                    failed_change_idx: i,
                })?;
            grouped.entry(doc_id).or_default().extend(text_edits);
        }

        for (doc_id, text_edits) in grouped {
            self.apply_resolved_text_edits(doc_id, text_edits, offset_encoding);
        }

        Ok(())
    }

    pub fn apply_workspace_edit(
        &mut self,
        offset_encoding: OffsetEncoding,
        workspace_edit: &lsp::WorkspaceEdit,
    ) -> Result<(), ApplyEditError> {
        if let Some(ref document_changes) = workspace_edit.document_changes {
            match document_changes {
                lsp::DocumentChanges::Edits(document_edits) => {
                    self.apply_text_edits_transactionally(
                        document_edits.iter().map(|document_edit| {
                            (
                                &document_edit.text_document.uri,
                                document_edit.text_document.version,
                                collect_text_edits(&document_edit.edits),
                            )
                        }),
                        offset_encoding,
                    )?;
                }
                lsp::DocumentChanges::Operations(operations) => {
                    log::debug!("document changes - operations: {:?}", operations);
                    let has_resource_op = operations
                        .iter()
                        .any(|op| matches!(op, lsp::DocumentChangeOperation::Op(_)));
                    if has_resource_op {
                        // Resource operations create, rename, and delete files.
                        // They are not reversible, so the whole edit falls back
                        // to applying each change in order and stopping at the
                        // first failure. Edits are applied as they appear rather
                        // than grouped per document, because order matters once
                        // resource operations are interleaved with them.
                        for (i, operation) in operations.iter().enumerate() {
                            match operation {
                                lsp::DocumentChangeOperation::Op(op) => {
                                    self.apply_document_resource_op(op).map_err(|kind| {
                                        ApplyEditError {
                                            kind,
                                            failed_change_idx: i,
                                        }
                                    })?;
                                }
                                lsp::DocumentChangeOperation::Edit(document_edit) => {
                                    self.apply_text_edit_group(
                                        &document_edit.text_document.uri,
                                        document_edit.text_document.version,
                                        collect_text_edits(&document_edit.edits),
                                        offset_encoding,
                                    )
                                    .map_err(|kind| {
                                        ApplyEditError {
                                            kind,
                                            failed_change_idx: i,
                                        }
                                    })?;
                                }
                            }
                        }
                    } else {
                        // No resource operations, so this is pure text and can
                        // be applied atomically.
                        self.apply_text_edits_transactionally(
                            operations.iter().filter_map(|operation| match operation {
                                lsp::DocumentChangeOperation::Edit(document_edit) => Some((
                                    &document_edit.text_document.uri,
                                    document_edit.text_document.version,
                                    collect_text_edits(&document_edit.edits),
                                )),
                                lsp::DocumentChangeOperation::Op(_) => None,
                            }),
                            offset_encoding,
                        )?;
                    }
                }
            }

            return Ok(());
        }

        if let Some(ref changes) = workspace_edit.changes {
            log::debug!("workspace changes: {:?}", changes);
            self.apply_text_edits_transactionally(
                changes
                    .iter()
                    .map(|(uri, text_edits)| (uri, None, text_edits.to_vec())),
                offset_encoding,
            )?;
        }

        Ok(())
    }

    fn apply_document_resource_op(
        &mut self,
        op: &lsp::ResourceOp,
    ) -> Result<(), ApplyEditErrorKind> {
        use lsp::ResourceOp;
        // NOTE: If `Uri` gets another variant than `Path`, the below `expect`s
        // may no longer be valid.
        match op {
            ResourceOp::Create(op) => {
                let uri = Uri::try_from(&op.uri)?;
                let path = uri.as_path().expect("URIs are valid paths");
                let ignore_if_exists = op.options.as_ref().is_some_and(|options| {
                    !options.overwrite.unwrap_or(false) && options.ignore_if_exists.unwrap_or(false)
                });
                if !ignore_if_exists || !path.exists() {
                    self.create_path(path, false)?;
                }
            }
            ResourceOp::Delete(op) => {
                let uri = Uri::try_from(&op.uri)?;
                let path = uri.as_path().expect("URIs are valid paths");
                let ignore_if_not_exists = op
                    .options
                    .as_ref()
                    .is_some_and(|options| options.ignore_if_not_exists.unwrap_or(false));
                if ignore_if_not_exists && !path.exists() {
                    return Ok(());
                }
                let recursive = op
                    .options
                    .as_ref()
                    .and_then(|options| options.recursive)
                    .unwrap_or(false);
                self.delete_path(path, recursive)?;
            }
            ResourceOp::Rename(op) => {
                let from_uri = Uri::try_from(&op.old_uri)?;
                let from = from_uri.as_path().expect("URIs are valid paths");
                let to_uri = Uri::try_from(&op.new_uri)?;
                let to = to_uri.as_path().expect("URIs are valid paths");
                let ignore_if_exists = op.options.as_ref().is_some_and(|options| {
                    !options.overwrite.unwrap_or(false) && options.ignore_if_exists.unwrap_or(false)
                });
                if !ignore_if_exists || !to.exists() {
                    self.move_path(from, to)?;
                }
            }
        }
        Ok(())
    }

    pub fn handle_lsp_diagnostics(
        &mut self,
        provider: &DiagnosticProvider,
        uri: Uri,
        version: Option<i32>,
        mut diagnostics: Vec<lsp::Diagnostic>,
    ) {
        let doc = self
            .documents
            .values_mut()
            .find(|doc| doc.uri().is_some_and(|u| u == uri));

        if let Some((version, doc)) = version.zip(doc.as_ref()) {
            if version != doc.version() {
                log::info!("Version ({version}) is out of date for {uri:?} (expected ({})), dropping PublishDiagnostic notification", doc.version());
                return;
            }
        }

        let mut unchanged_diag_sources = Vec::new();
        if let Some((lang_conf, old_diagnostics)) = doc
            .as_ref()
            .and_then(|doc| Some((doc.language_config()?, self.diagnostics.get(&uri)?)))
        {
            if !lang_conf.persistent_diagnostic_sources.is_empty() {
                // Sort diagnostics first by severity and then by line numbers.
                // Note: The `lsp::DiagnosticSeverity` enum is already defined in decreasing order
                diagnostics.sort_by_key(|d| (d.severity, d.range.start));
            }
            for source in &lang_conf.persistent_diagnostic_sources {
                let new_diagnostics = diagnostics
                    .iter()
                    .filter(|d| d.source.as_ref() == Some(source));
                let old_diagnostics = old_diagnostics
                    .iter()
                    .filter(|(d, d_provider)| {
                        d_provider == provider && d.source.as_ref() == Some(source)
                    })
                    .map(|(d, _)| d);
                if new_diagnostics.eq(old_diagnostics) {
                    unchanged_diag_sources.push(source.clone())
                }
            }
        }

        let diagnostics = diagnostics.into_iter().map(|d| (d, provider.clone()));

        // Insert the original lsp::Diagnostics here because we may have no open document
        // for diagnostic message and so we can't calculate the exact position.
        // When using them later in the diagnostics picker, we calculate them on-demand.
        let diagnostics = match self.diagnostics.entry(uri) {
            Entry::Occupied(o) => {
                let current_diagnostics = o.into_mut();
                // there may entries of other language servers, which is why we can't overwrite the whole entry
                current_diagnostics.retain(|(_, d_provider)| d_provider != provider);
                current_diagnostics.extend(diagnostics);
                current_diagnostics
                // Sort diagnostics first by severity and then by line numbers.
            }
            Entry::Vacant(v) => v.insert(diagnostics.collect()),
        };

        // Sort diagnostics first by severity and then by line numbers.
        // Note: The `lsp::DiagnosticSeverity` enum is already defined in decreasing order
        diagnostics.sort_by_key(|(d, provider)| (d.severity, d.range.start, provider.clone()));

        if let Some(doc) = doc {
            let diagnostic_of_language_server_and_not_in_unchanged_sources =
                |diagnostic: &lsp::Diagnostic, d_provider: &DiagnosticProvider| {
                    d_provider == provider
                        && diagnostic
                            .source
                            .as_ref()
                            .is_none_or(|source| !unchanged_diag_sources.contains(source))
                };
            let diagnostics = Self::doc_diagnostics_with_filter(
                &self.language_servers,
                &self.diagnostics,
                doc,
                diagnostic_of_language_server_and_not_in_unchanged_sources,
            );
            doc.replace_diagnostics(diagnostics, &unchanged_diag_sources, Some(provider));

            let doc = doc.id();
            helix_event::dispatch(DiagnosticsDidChange { editor: self, doc });
        }
    }

    pub fn execute_lsp_command(&mut self, command: lsp::Command, server_id: LanguageServerId) {
        // the command is executed on the server and communicated back
        // to the client asynchronously using workspace edits
        let Some(future) = self
            .language_server_by_id(server_id)
            .and_then(|server| server.command(command))
        else {
            self.set_error("Language server does not support executing commands");
            return;
        };

        tokio::spawn(async move {
            if let Err(err) = future.await {
                log::error!("Error executing LSP command: {err}");
            }
        });
    }
}

pub fn register_hooks(_handlers: &Handlers) {
    register_hook!(move |event: &mut LanguageServerInitialized<'_>| {
        let language_server = event.editor.language_server_by_id(event.server_id).unwrap();
        log::debug!(target: "lsp", "server {} initialized", language_server.name());

        for doc in event
            .editor
            .documents()
            .filter(|doc| doc.supports_language_server(event.server_id))
        {
            let Some(url) = doc.url() else {
                continue;
            };

            let language_id = doc.language_id().map(ToOwned::to_owned).unwrap_or_default();

            language_server.text_document_did_open(url, doc.version(), doc.text(), language_id);
        }

        Ok(())
    });

    register_hook!(move |event: &mut DocumentDidChange<'_>| {
        // Send textDocument/didChange notifications.
        if !event.ghost_transaction {
            for language_server in event.doc.language_servers() {
                language_server.text_document_did_change(
                    event.doc.versioned_identifier(),
                    event.old_text,
                    event.doc.text(),
                    event.changes,
                );
            }
        }

        Ok(())
    });

    register_hook!(move |event: &mut DocumentDidClose<'_>| {
        // Send textDocument/didClose notifications.
        for language_server in event.doc.language_servers() {
            language_server.text_document_did_close(event.doc.identifier());
        }

        Ok(())
    });
}
