use std::{collections::HashSet, time::Duration};

use futures_util::{stream::FuturesUnordered, StreamExt};
use helix_core::{syntax::config::LanguageServerFeature, text_annotations::InlineAnnotation};
use helix_event::{cancelable_future, register_hook};
use helix_lsp::lsp;
use helix_view::{
    document::DocumentColors,
    editor::DocumentColorDisplay,
    events::{DocumentDidChange, DocumentDidOpen, LanguageServerExited, LanguageServerInitialized},
    handlers::{lsp::DocumentColorsEvent, Handlers},
    DocumentId, Editor, Theme,
};
use tokio::time::Instant;

use crate::job;

#[derive(Default)]
pub(super) struct DocumentColorsHandler {
    docs: HashSet<DocumentId>,
}

const DOCUMENT_CHANGE_DEBOUNCE: Duration = Duration::from_millis(250);

impl helix_event::AsyncHook for DocumentColorsHandler {
    type Event = DocumentColorsEvent;

    fn handle_event(&mut self, event: Self::Event, _timeout: Option<Instant>) -> Option<Instant> {
        let DocumentColorsEvent(doc_id) = event;
        self.docs.insert(doc_id);
        Some(Instant::now() + DOCUMENT_CHANGE_DEBOUNCE)
    }

    fn finish_debounce(&mut self) {
        let docs = std::mem::take(&mut self.docs);

        job::dispatch_blocking(move |editor, _compositor| {
            for doc in docs {
                request_document_colors(editor, doc);
            }
        });
    }
}

fn request_document_colors(editor: &mut Editor, doc_id: DocumentId) {
    if editor.config().lsp.document_color == DocumentColorDisplay::Off {
        return;
    }

    let Some(doc) = editor.document_mut(doc_id) else {
        return;
    };

    let cancel = doc.color_swatch_controller.restart();

    let mut seen_language_servers = HashSet::new();
    let mut futures: FuturesUnordered<_> = doc
        .language_servers_with_feature(LanguageServerFeature::DocumentColors)
        .filter(|ls| seen_language_servers.insert(ls.id()))
        .map(|language_server| {
            let text = doc.text().clone();
            let offset_encoding = language_server.offset_encoding();
            let future = language_server
                .text_document_document_color(doc.identifier(), None)
                .unwrap();

            async move {
                let colors: Vec<_> = future
                    .await?
                    .into_iter()
                    .filter_map(|color_info| {
                        let start = helix_lsp::util::lsp_pos_to_pos(
                            &text,
                            color_info.range.start,
                            offset_encoding,
                        )?;
                        let end = helix_lsp::util::lsp_pos_to_pos(
                            &text,
                            color_info.range.end,
                            offset_encoding,
                        )?;
                        Some((start..end, color_info.color))
                    })
                    .collect();
                anyhow::Ok(colors)
            }
        })
        .collect();

    if futures.is_empty() {
        return;
    }

    tokio::spawn(async move {
        let mut all_colors = Vec::new();
        loop {
            match cancelable_future(futures.next(), &cancel).await {
                Some(Some(Ok(items))) => all_colors.extend(items),
                Some(Some(Err(err))) => log::error!("document color request failed: {err}"),
                Some(None) => break,
                // The request was cancelled.
                None => return,
            }
        }
        job::dispatch(move |editor, _| attach_document_colors(editor, doc_id, all_colors)).await;
    });
}

fn attach_document_colors(
    editor: &mut Editor,
    doc_id: DocumentId,
    mut doc_colors: Vec<(std::ops::Range<usize>, lsp::Color)>,
) {
    if editor.config().lsp.document_color == DocumentColorDisplay::Off {
        return;
    }

    let Some(doc) = editor.documents.get_mut(&doc_id) else {
        return;
    };

    if doc_colors.is_empty() {
        doc.document_colors.take();
        return;
    }

    doc_colors.sort_by(|(a, _), (b, _)| a.start.cmp(&b.start));

    let mut ranges = Vec::with_capacity(doc_colors.len());
    let mut colors = Vec::with_capacity(doc_colors.len());
    let mut swatch_markers = Vec::with_capacity(doc_colors.len());
    let mut swatch_padding = Vec::with_capacity(doc_colors.len());

    for (range, color) in doc_colors {
        swatch_padding.push(InlineAnnotation::new(range.start, " "));
        swatch_markers.push(InlineAnnotation::new(range.start, "■"));
        ranges.push(range);
        colors.push(Theme::rgb_highlight(
            (color.red * 255.) as u8,
            (color.green * 255.) as u8,
            (color.blue * 255.) as u8,
        ));
    }

    doc.document_colors = Some(DocumentColors {
        ranges,
        colors,
        swatch_markers,
        swatch_padding,
    });
}

pub(super) fn register_hooks(handlers: &Handlers) {
    register_hook!(move |event: &mut DocumentDidOpen<'_>| {
        // when a document is initially opened, request colors for it
        request_document_colors(event.editor, event.doc);

        Ok(())
    });

    let tx = handlers.document_colors.clone();
    register_hook!(move |event: &mut DocumentDidChange<'_>| {
        // Keep stored positions aligned with the edit so colors stay on their literal
        // between language-server refreshes.
        let apply_annotation_changes = |annotations: &mut Vec<InlineAnnotation>| {
            event.changes.update_positions(
                annotations
                    .iter_mut()
                    .map(|annotation| (&mut annotation.char_idx, helix_core::Assoc::After)),
            );
        };

        if let Some(DocumentColors {
            ranges,
            colors: _colors,
            swatch_markers,
            swatch_padding,
        }) = &mut event.doc.document_colors
        {
            apply_annotation_changes(swatch_markers);
            apply_annotation_changes(swatch_padding);

            event
                .changes
                .update_positions(ranges.iter_mut().flat_map(|range| {
                    [
                        (&mut range.start, helix_core::Assoc::After),
                        (&mut range.end, helix_core::Assoc::Before),
                    ]
                }));
        }

        // Avoid re-requesting document colors if the change is a ghost transaction (completion)
        // because the language server will not know about the updates to the document and will
        // give out-of-date locations.
        if !event.ghost_transaction {
            // Cancel the ongoing request, if present.
            event.doc.color_swatch_controller.cancel();
            helix_event::send_blocking(&tx, DocumentColorsEvent(event.doc.id()));
        }

        Ok(())
    });

    register_hook!(move |event: &mut LanguageServerInitialized<'_>| {
        let doc_ids: Vec<_> = event.editor.documents().map(|doc| doc.id()).collect();

        for doc_id in doc_ids {
            request_document_colors(event.editor, doc_id);
        }

        Ok(())
    });

    register_hook!(move |event: &mut LanguageServerExited<'_>| {
        // Clear and re-request all document colors when a server exits.
        for doc in event.editor.documents_mut() {
            if doc.supports_language_server(event.server_id) {
                doc.document_colors.take();
            }
        }

        let doc_ids: Vec<_> = event.editor.documents().map(|doc| doc.id()).collect();

        for doc_id in doc_ids {
            request_document_colors(event.editor, doc_id);
        }

        Ok(())
    });
}
